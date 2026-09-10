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

## Stato

Fatte tutte e dieci le fasi, ognuna verificata prima della successiva.

Due cose restano **scritte e non verificate**, ed e' giusto che si sappia
prima di leggere il resto:

1. Il codice dell'applicazione e' scritto per intero — comandi Tauri e
   interfaccia — e l'interfaccia e' stata percorsa tutta in un browser, ma
   **la finestra non e' mai stata compilata su questa macchina**: mancano le
   librerie di sistema (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
   `libdbus-1-dev`), e installarle richiede i permessi di amministratore.

2. I due workflow di CI non sono mai stati eseguiti: GitHub Actions non si
   prova da questa macchina. Il primo push su un tag e' anche il loro primo
   collaudo.

## Fasi

### ✅ Fase 0 — Ristrutturazione, a comportamento invariato
Repository git; workspace Cargo con `verba-core`, `verba-cli`, `verba-app`;
rinomina completa da AutoSubtitler a Verba. Nessun cambiamento funzionale.
**Verifica**: gli stessi 79 test passano e la CLI produce lo stesso MOV di prima.

### ✅ Fase 1 — Fondamenta per la 0.2
Identificativo stabile per parola; la sequenza diventa struttura mutabile
separata dal risultato grezzo del modello. Canale di eventi di avanzamento
(fase, percentuale, tempo) al posto del solo logging. Modulo `project`: preset
in JSON e i tre di serie (Verticale, Orizzontale, Sobrio).
**Verifica**: la CLI stampa l'avanzamento per fasi; un preset salvato e
ricaricato riproduce lo stesso fotogramma.

### ✅ Fase 2 — Ingresso video
Demux e decodifica con libavformat/libavcodec (il `cpp/` e' gia' linkato):
estrazione dell'audio dai container video, decodifica del fotogramma al tempo
`t`, rilevamento della modalita' audio/video. Errore esplicito per i file senza
traccia audio.
**Verifica**: trascrizione di un `.mp4` e di un `.mkv`; estrazione di un
fotogramma a un tempo dato.

### ✅ Fase 3 — Layout completo
Righe massime 1/2/3 con spezzatura bilanciata; larghezza massima, posizione
verticale e orizzontale, allineamento, margine. Corpo in pixel riferiti
all'altezza del sorgente, con la percentuale sul fotogramma. Maiuscole.
**Verifica**: test di geometria per ogni combinazione; nessuna regressione sul
caso a una riga.

### ✅ Fase 4 — Render completo
Forme dell'evidenziazione (rettangolo, sottolineatura, solo colore); colore del
testo attivo; ombra; font di sistema e da `assets/fonts` con peso selezionabile
e ricaduta annunciata se manca. API `disegna_fotogramma(t) -> RGBA`, **la stessa
usata dall'export**: l'anteprima non ha un percorso di codice proprio.
**Verifica**: confronto pixel a pixel fra un fotogramma d'anteprima e lo stesso
fotogramma estratto dall'export.

### ✅ Fase 5 — Export
H.264 CRF 18 yuv420p impresso (default video), ProRes 422 impresso, ProRes 4444
overlay (gia' fatto), WebM VP9 con alpha; SRT (fatto), VTT, JSON (fatto), TXT.
Avanzamento, annullamento reale e cancellazione del file parziale.
**Verifica**: ogni formato prodotto e riaperto; l'annullamento non lascia file.

### ✅ Fase 6 — Riga di comando
Sottocomandi `trascrivi`, `rendi`, `overlay`, piu' `caratteri`, `preset`,
`formati` e `modelli`; `--preset`; `--json` scrive l'avanzamento su stderr.
Il formato di uscita lo dice l'estensione di `--out`.
**Verificata**: le tre righe della spec eseguite alla lettera su un mp4 reale;
177 test verdi, clippy pulito.

### ✅ Fase 7 — Modelli e dispositivo
Scaricamento con verifica SHA-256 e ripresa, nella cartella dati dell'utente;
`libonnxruntime` cercata accanto all'eseguibile prima che nel sistema; ricaduta
CUDA-CPU senza errori bloccanti e annunciata; selettore della dimensione.
**Verificata**: `ggml-small.bin` scaricato, interrotto a meta' con Ctrl-C,
ripreso e chiuso con l'impronta corretta; trascrizione completa con `small`
scaricato e allineatore locale.

### ✅ Fase 8 — Applicazione Tauri
Guscio 1600x980 fisso, barra laterale, quattro sezioni, barra di stato; ventotto
comandi sopra `verba-core`; le tre stanze nell'ordine della spec. Lo stato di
un lavoro aperto sta in `verba_core::sessione`, non nel guscio: e' quello che
tiene l'anteprima sullo stesso codice dell'export.
**Verificata a meta'**: l'interfaccia e' stata percorsa per intero in un
browser (i quattro stati di Carica, il pannello di Modifica, l'elenco dei
formati, le impostazioni); il guscio Tauri **non e' stato compilato** perche'
mancano le librerie di sistema.

### ✅ Fase 9 — Repository e distribuzione
README con lo screenshot in testa e i limiti noti in alto, LICENSE (MIT),
CHANGELOG, CONTRIBUTING, `docs/termini.md`, `docs/tempi.md` con i diagrammi
generati dal codice, workflow di verifica e di rilascio per `.deb`,
`.AppImage` e `.exe`.
**Verificata a meta'**: i documenti e le figure ci sono e sono stati
riletti; **i workflow non sono mai stati eseguiti**, perche' GitHub Actions non
si prova da qui.

## Come rimettere in piedi le due verifiche mancanti

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev \
                 libayatana-appindicator3-dev librsvg2-dev \
                 libdbus-1-dev patchelf
npm install --prefix ui
cargo build -p verba-app --release
```

Poi `cargo tauri dev --config crates/verba-app/tauri.conf.json` apre la
finestra vera, con il motore dietro invece del banco di prova. Il percorso da
provare e' quello della spec: trascinare un file, aspettare le fasi, guardare
l'anteprima, cambiare qualcosa in Modifica, esportare.

Per i workflow basta il primo push: `verifica.yml` parte su qualsiasi commit,
`rilascio.yml` su un tag `v0.1.0`.
