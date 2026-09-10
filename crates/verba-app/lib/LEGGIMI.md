# Le librerie che viaggiano con l'applicazione

Qui dentro va **ONNX Runtime**, che non e' impacchettabile dentro l'eseguibile
e non si puo' dare per scontata sulla macchina di chi installa:

```
libonnxruntime.so                    (Linux)
libonnxruntime_providers_shared.so   (Linux, se si vuole il provider CUDA)
libonnxruntime_providers_cuda.so     (Linux, idem)
onnxruntime.dll                      (Windows)
```

`bundle.resources` in `tauri.conf.json` li copia accanto all'eseguibile, e
`verba_core::onnx::assicura_libreria` li trova senza bisogno di
`ORT_DYLIB_PATH`. In sviluppo la cartella e' vuota: si usa la copia in
`~/.local/share/verba/lib` o quella di sistema.

Li mette qui il workflow di rilascio (`.github/workflows/rilascio.yml`), che li
scarica dalla release ufficiale di Microsoft alla versione **1.22.x** — quella
e' l'unica che `ort 2.0.0-rc.10` accetta.

Questa cartella e' sotto controllo di versione solo per questo file: i `.so` e
le `.dll` non stanno in un repository git.
