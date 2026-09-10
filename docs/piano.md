# Verba — piano di costruzione della 0.1

Documento di lavoro. Riferimento: `verba-app-spec.md`. Ogni fase termina con
qualcosa di osservabile e con i test verdi; nessuna fase comincia prima che la
precedente sia verificata.

## Scostamenti dalla spec, decisi e motivati

| Punto della spec | Decisione | Perche' |
|---|---|---|
| `asr`: Whisper large-v3 **in ONNX** | Resta **GGML/whisper.cpp** | Gira su CPU senza toolkit, ha CUDA e Vulkan (quindi anche le schede che CUDA ha abbandonato), e le quantizzazioni coprono il selettore large-v3/medium/small. ONNX resta per pyannote e wav2vec2. |
| `Righe massime: default 2` | Configurabile 1/2/3, **default 1** | Scelta dell'autore: piu' di una riga per volta rende il risultato caotico. Il controllo esiste, il default no. |
| (non trattato) `ORT_DYLIB_PATH` | `libonnxruntime` impacchettata accanto all'eseguibile | Oggi la libreria e' risolta da una variabile d'ambiente. Un `.deb` o un `.exe` non puo' assumerla: senza questo, l'app non parte su nessuna macchina che non sia quella di sviluppo. |

## Fasi

### Fase 0 — Ristrutturazione, a comportamento invariato
Repository git; workspace Cargo con `verba-core`, `verba-cli`, `verba-app`;
rinomina completa da AutoSubtitler a Verba. Nessun cambiamento funzionale.
**Verifica**: gli stessi 79 test passano e la CLI produce lo stesso MOV di prima.

### Fase 1 — Fondamenta per la 0.2
Identificativo stabile per parola; la sequenza diventa struttura mutabile
separata dal risultato grezzo del modello. Canale di eventi di avanzamento
(fase, percentuale, tempo) al posto del solo logging. Modulo `project`: preset
in JSON e i tre di serie (Verticale, Orizzontale, Sobrio).
**Verifica**: la CLI stampa l'avanzamento per fasi; un preset salvato e
ricaricato riproduce lo stesso fotogramma.

### Fase 2 — Ingresso video
Demux e decodifica con libavformat/libavcodec (il `cpp/` e' gia' linkato):
estrazione dell'audio dai container video, decodifica del fotogramma al tempo
`t`, rilevamento della modalita' audio/video. Errore esplicito per i file senza
traccia audio.
**Verifica**: trascrizione di un `.mp4` e di un `.mkv`; estrazione di un
fotogramma a un tempo dato.

### Fase 3 — Layout completo
Righe massime 1/2/3 con spezzatura bilanciata; larghezza massima, posizione
verticale e orizzontale, allineamento, margine. Corpo in pixel riferiti
all'altezza del sorgente, con la percentuale sul fotogramma. Maiuscole.
**Verifica**: test di geometria per ogni combinazione; nessuna regressione sul
caso a una riga.

### Fase 4 — Render completo
Forme dell'evidenziazione (rettangolo, sottolineatura, solo colore); colore del
testo attivo; ombra; font di sistema e da `assets/fonts` con peso selezionabile
e ricaduta annunciata se manca. API `disegna_fotogramma(t) -> RGBA`, **la stessa
usata dall'export**: l'anteprima non ha un percorso di codice proprio.
**Verifica**: confronto pixel a pixel fra un fotogramma d'anteprima e lo stesso
fotogramma estratto dall'export.

### Fase 5 — Export
H.264 CRF 18 yuv420p impresso (default video), ProRes 422 impresso, ProRes 4444
overlay (gia' fatto), WebM VP9 con alpha; SRT (fatto), VTT, JSON (fatto), TXT.
Avanzamento, annullamento reale e cancellazione del file parziale.
**Verifica**: ogni formato prodotto e riaperto; l'annullamento non lascia file.

### Fase 6 — Riga di comando
Sottocomandi `trascrivi`, `rendi`, `overlay`; `--preset`; `--json` scrive
l'avanzamento su stderr.
**Verifica**: i tre comandi della spec funzionano alla lettera.

### Fase 7 — Modelli e dispositivo
Scaricamento con verifica dell'hash e ripresa, nella cartella dati dell'utente;
`libonnxruntime` impacchettata; ricaduta CUDA-CPU senza errori bloccanti;
selettore della dimensione del modello.
**Verifica**: primo avvio su una macchina senza modelli e senza GPU.

### Fase 8 — Applicazione Tauri
Guscio 1600x980 fisso, barra laterale, quattro sezioni, barra di stato; comandi
sopra `verba-core`; le tre stanze nell'ordine della spec.
**Verifica**: il percorso completo dal trascinamento del file all'export.

### Fase 9 — Repository e distribuzione
README, LICENSE, CHANGELOG, CONTRIBUTING, `docs/termini.md`, `docs/tempi.md`,
workflow di build per `.deb`, `.AppImage` e `.exe`.

## Ordine di verifica

I punti da 1 a 6 sono l'applicazione vera. Se il tempo finisce, un progetto
fermo alla fine della Fase 6 con una buona riga di comando e' comunque
pubblicabile, come dice la spec.
