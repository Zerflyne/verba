#!/usr/bin/env bash
# Produce l'allineatore ONNX e lo pubblica come allegato di una release, cosi'
# che il programma lo scarichi invece di chiederlo a chi lo installa.
#
# L'allineatore e' l'unico dei quattro file che non si scarica da solo: di
# wav2vec2 italiano non esiste un'esportazione ONNX pubblica di cui fidarsi, e
# finora l'unica strada era produrlo sulla propria macchina. Il modello di
# partenza e' Apache-2.0, quindi ridistribuirlo e' permesso: basta farlo con
# l'attribuzione e con un'impronta verificabile.
#
# Non finisce in git: GitHub rifiuta i file oltre i 100 MB, e Git LFS nel piano
# gratuito da' 1 GB di spazio e 1 GB di traffico al mese — meno di una copia di
# questo file. Gli allegati di release invece arrivano a 2 GB l'uno e non
# consumano traffico conteggiato.
#
# Serve `gh` autenticato (`gh auth login`).

set -euo pipefail

TAG="${1:-modelli-1}"
RADICE="$(cd "$(dirname "$0")/.." && pwd)"
ONNX="$RADICE/models/wav2vec2-italian.onnx"
VOCAB="$RADICE/models/wav2vec2-italian.vocab.json"

# Il file pesa 1,18 GiB, e torch per produrlo ne vuole altri 2,5. Chiederlo
# prima e' meglio che scoprirlo a meta' esportazione con il disco pieno.
SERVONO_MIB=4096
liberi_mib=$(df -Pm "$RADICE" | awk 'NR==2 {print $4}')
if [ "$liberi_mib" -lt "$SERVONO_MIB" ]; then
    echo "Spazio insufficiente: ${liberi_mib} MiB liberi, ne servono ~${SERVONO_MIB}." >&2
    echo "L'esportazione produce 1,18 GiB e torch ne occupa altri 2,5." >&2
    exit 1
fi

if ! command -v gh >/dev/null; then
    echo "Serve la CLI di GitHub: https://cli.github.com — poi 'gh auth login'." >&2
    exit 1
fi

if [ ! -f "$ONNX" ]; then
    echo "== Esportazione (una volta sola, qualche minuto) =="
    pip install --quiet "torch>=2.2" onnx transformers huggingface_hub
    python3 "$RADICE/scripts/export_models.py" --wav2vec2 --out-dir "$RADICE/models"
fi

[ -f "$ONNX" ] || { echo "L'esportazione non ha prodotto $ONNX." >&2; exit 1; }

impronta=$(sha256sum "$ONNX" | cut -d' ' -f1)
byte=$(stat -c%s "$ONNX")

echo
echo "== Pubblicazione =="
if gh release view "$TAG" >/dev/null 2>&1; then
    gh release upload "$TAG" "$ONNX" --clobber
else
    gh release create "$TAG" "$ONNX" \
        --title "Modelli — allineatore ONNX" \
        --notes "wav2vec2 italiano esportato in ONNX, da \`jonatasgrosman/wav2vec2-large-xlsr-53-italian\` (Apache-2.0).

Non e' una release del programma: e' il file che \`verba modelli --scarica\`
preleva da qui, cosi' che non debba produrlo chi installa. L'impronta
SHA-256 e' dichiarata nel codice e verificata a ogni scaricamento.

    $impronta"
fi

repo=$(gh repo view --json nameWithOwner -q .nameWithOwner)
echo
echo "== Da mettere in crates/verba-core/src/modelli.rs, in ALLINEATORE =="
cat <<FINE

    byte: ${byte},
    provenienza: Provenienza::Scaricabile {
        url: "https://github.com/${repo}/releases/download/${TAG}/wav2vec2-italian.onnx",
        sha256: "${impronta}",
    },
FINE
echo
echo "Impronta: $impronta"
echo "Dimensione: $byte byte"
[ -f "$VOCAB" ] && echo "Il vocabolario resta su Hugging Face: e' 410 byte e si scarica da la'."
