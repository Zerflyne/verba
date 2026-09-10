#!/usr/bin/env python3
"""Esporta in ONNX i due modelli ausiliari richiesti da AutoSubtitler.

  * segmentazione : pyannote/segmentation-3.0   -> models/pyannote-segmentation-3.0.onnx
  * allineamento  : wav2vec2 italiano (CTC)     -> models/wav2vec2-italian.onnx
                                                    models/wav2vec2-italian.vocab.json

Dipendenze:
    pip install "torch>=2.2" onnx transformers pyannote.audio huggingface_hub

pyannote/segmentation-3.0 e' un modello "gated": serve accettare le condizioni
sulla pagina del modello e passare un token HF (--hf-token o env HF_TOKEN).

Uso tipico:
    python scripts/export_models.py --all --hf-token hf_xxx
"""

import argparse
import json
import os
from pathlib import Path

import torch

MODELS_DIR = Path(__file__).resolve().parent.parent / "models"
DEFAULT_W2V = "jonatasgrosman/wav2vec2-large-xlsr-53-italian"


def export_segmentation(hf_token: str | None, out: Path) -> None:
    from pyannote.audio import Model

    print("[segmentazione] carico pyannote/segmentation-3.0 ...")
    model = Model.from_pretrained("pyannote/segmentation-3.0", use_auth_token=hf_token)
    model.eval()

    # finestra fissa di 10 s a 16 kHz: e' la durata su cui il modello e' addestrato
    dummy = torch.zeros(1, 1, 160_000)

    out.parent.mkdir(parents=True, exist_ok=True)
    torch.onnx.export(
        model,
        dummy,
        str(out),
        input_names=["waveform"],
        output_names=["segmentation"],
        # il batch resta dinamico; la lunghezza della finestra e' fissa
        dynamic_axes={"waveform": {0: "batch"}, "segmentation": {0: "batch"}},
        opset_version=17,
        do_constant_folding=True,
    )
    with torch.no_grad():
        ref = model(dummy)
    print(f"[segmentazione] scritto {out}  (uscita torch: {tuple(ref.shape)})")
    print("               classi =", ref.shape[-1], "(7 = powerset seg-3.0)")


def export_wav2vec2(model_id: str, out: Path) -> None:
    from transformers import AutoProcessor, AutoModelForCTC

    print(f"[allineamento] carico {model_id} ...")
    processor = AutoProcessor.from_pretrained(model_id)
    model = AutoModelForCTC.from_pretrained(model_id)
    model.eval()

    class Wrapper(torch.nn.Module):
        """Espone solo i logits: l'export ONNX non gestisce le dataclass HF."""

        def __init__(self, m):
            super().__init__()
            self.m = m

        def forward(self, input_values):
            return self.m(input_values).logits

    dummy = torch.zeros(1, 16_000 * 5)  # 5 s
    out.parent.mkdir(parents=True, exist_ok=True)
    torch.onnx.export(
        Wrapper(model),
        dummy,
        str(out),
        input_names=["input_values"],
        output_names=["logits"],
        # lunghezza audio dinamica: i segmenti hanno durate diverse
        dynamic_axes={"input_values": {0: "batch", 1: "samples"},
                      "logits": {0: "batch", 1: "frames"}},
        opset_version=17,
        do_constant_folding=True,
    )

    tokenizer = getattr(processor, "tokenizer", processor)
    vocab = tokenizer.get_vocab()
    vocab_path = out.with_suffix("").with_suffix(".vocab.json")
    vocab_path.write_text(json.dumps(vocab, ensure_ascii=False, indent=2), encoding="utf-8")

    fe = getattr(processor, "feature_extractor", None)
    do_norm = getattr(fe, "do_normalize", True) if fe else True

    print(f"[allineamento] scritto {out}")
    print(f"[allineamento] vocabolario ({len(vocab)} token) -> {vocab_path}")
    print(f"[allineamento] do_normalize = {do_norm}"
          f"  ->  passare --no-normalize a autosubtitler se False")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--all", action="store_true", help="esporta entrambi i modelli")
    ap.add_argument("--segmentation", action="store_true")
    ap.add_argument("--wav2vec2", action="store_true")
    ap.add_argument("--w2v-model", default=DEFAULT_W2V, help=f"default: {DEFAULT_W2V}")
    ap.add_argument("--hf-token", default=os.environ.get("HF_TOKEN"))
    ap.add_argument("--out-dir", type=Path, default=MODELS_DIR)
    args = ap.parse_args()

    if not (args.all or args.segmentation or args.wav2vec2):
        ap.error("scegliere --all, --segmentation o --wav2vec2")

    if args.all or args.segmentation:
        export_segmentation(args.hf_token, args.out_dir / "pyannote-segmentation-3.0.onnx")
    if args.all or args.wav2vec2:
        export_wav2vec2(args.w2v_model, args.out_dir / "wav2vec2-italian.onnx")


if __name__ == "__main__":
    main()
