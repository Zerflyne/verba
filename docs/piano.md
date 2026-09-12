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

Il guscio Tauri ora compila e si apre: la finestra 1600x980 esiste, la
webview disegna l'interfaccia e il primo comando IPC (`informazioni`) e'
arrivato fino a NVML e tornato indietro. Compilando davvero e' saltato fuori
un errore che nessuna rilettura aveva visto — `FormatiDisponibili` e
`FormatoTesto` derivavano `Deserialize` senza poterlo fare — ed e' corretto.

Resta **una cosa scritta e non verificata**: i due workflow di CI non sono
mai stati eseguiti. GitHub Actions non si prova da questa macchina, e il primo
push su un tag e' anche il loro primo collaudo.

Il file caricato *dalla finestra*, che prima mancava, ora c'e'. I clic si
sintetizzano con XTEST, ma con un'avvertenza che e' costata tempo: il server X
consegna il clic sintetico alla finestra piu' in alto in quel punto, e mutter
ignora le richieste di sollevamento fatte dal client. Finche' non si manda un
`_NET_ACTIVE_WINDOW` alla radice, i clic finiscono a chi sta sopra e sembra
semplicemente che l'applicazione non risponda.

## Dopo la prima prova vera (10 settembre 2026)

Sei aggiustamenti chiesti dopo aver usato l'applicazione. Cosa e' verificato e
cosa no, uno per uno.

| | Verificato | Come |
|---|---|---|
| GPU scelta a mano in Impostazioni | **a meta'** | Il menu si popola e ricorda la scelta; che il calcolo finisca davvero sulla scheda scelta **non e' stato provato** (qui ce n'e' una sola) |
| Audio trattato come video | **si'** | `verba overlay prova.mp3` produce `alpha_mode=1` in WebM e `yuva444p12le` in ProRes 4444; la finestra Esporta li offre di nuovo |
| Anteprima su nero per i file audio | **si'** | `prova.mp3` aperto nella finestra vera: fotogramma nero, sottotitolo sopra, contorno chiaro attorno alla tela per non confonderla con lo sfondo |
| Whisper e allineatore insieme sopra il 20% | **si'** | Trascrizione vera di `prova.mp3` sulla Tesla P40: «restano caricati insieme: 22901 MiB liberi, ne servivano 7200», e lo scarico avviene dopo l'allineamento |
| Riproduzione dell'audio | **la causa e' misurata, il pacchetto e' da riprovare** | Il difetto non era nell'audio: la CSP bloccava la `fetch` verso `ipc://localhost`, Tauri ripiegava su `postMessage` e i byte grezzi arrivavano come array di numeri. Ogni anello e' stato misurato con `scripts/banco_webview.py` su questa macchina, compreso l'errore identico a quello visto. Resta da sentire il suono da un pacchetto costruito con la CSP nuova |
| Editor dei termini noti | **a meta'** | Il pannello si apre, si scrive, si conta; il salvataggio passa dal comando Tauri, che non e' mai stato chiamato per davvero |
| Modelli mancanti in evidenza | **a meta'** | La scheda d'avviso si vede con `?modelli=mancanti` nel banco; nella finestra vera i modelli ci sono e la scheda non compare |

Aprendo davvero un file dalla finestra sono saltati fuori quattro difetti che
nessuna rilettura aveva visto, tutti nell'anteprima: l'audio che non partiva,
la tela senza misura, i fotogrammi fermi durante la riproduzione e la colonna
di Modifica che spingeva il trasporto sotto il bordo. Sono descritti nel
diario, alla voce *Corretto*.

Sull'audio ci sono voluti tre tentativi, e i primi due hanno curato il sintomo
perche' nessuno aveva **misurato** niente: si vedeva «formato non supportato» e
si concludeva che il formato o l'indirizzo fossero sbagliati. Erano giusti. Il
numero che ha risolto la questione e' stato la lunghezza del blob, 1.010.572
byte per un WAV che ne conta 288.044: tre volte e mezzo, cioe' byte scritti
come numeri decimali. Da li' la catena e' venuta da se'. La morale non e'
sull'audio: **un messaggio d'errore dice cosa si e' rotto, non dove**, e finche'
una verifica gira soltanto in sviluppo non e' una verifica del pacchetto.

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
**Verifica**: `cargo build -p verba-app` compila e linka; l'eseguibile apre
una finestra X di 1600x980 intitolata Verba, la webview disegna la sezione
Carica e chiama `informazioni`, che risponde con GPU, provider ONNX e
cartelle. L'interfaccia era gia' stata percorsa per intero in un browser (i
quattro stati di Carica, il pannello di Modifica, l'elenco dei formati, le
impostazioni). **Non verificato**: caricare un file dalla finestra, che
richiede un click che da qui non si puo' dare.

### ✅ Fase 9 — Repository e distribuzione
README con lo screenshot in testa e i limiti noti in alto, LICENSE (MIT),
CHANGELOG, CONTRIBUTING, `docs/termini.md`, `docs/tempi.md` con i diagrammi
generati dal codice, workflow di verifica e di rilascio per `.deb`,
`.AppImage` e `.exe`.
**Verificata a meta'**: i documenti e le figure ci sono e sono stati
riletti; **i workflow non sono mai stati eseguiti**, perche' GitHub Actions non
si prova da qui.

## Come rimettere in piedi la verifica mancante

Su una macchina dove le librerie non ci sono ancora:

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev \
                 libayatana-appindicator3-dev librsvg2-dev \
                 libdbus-1-dev patchelf
```

Poi, per aprire la finestra collegata al motore:

```bash
npm install --prefix ui
npm run dev --prefix ui &
cargo build -p verba-app && ./target/debug/verba-app
```

Il binario di debug carica l'interfaccia da `http://localhost:5173`: senza il
server di sviluppo acceso mostra soltanto *Connection refused*. Un binario di
`--release` incorpora invece `ui/dist` e non ha bisogno di niente.
`cargo tauri` non e' un sottocomando di cargo: e' un binario a parte
(`cargo install tauri-cli --version '^2'`) e serve solo per impacchettare.

Il percorso da provare e' quello della spec: trascinare un file, aspettare le
fasi, guardare l'anteprima, cambiare qualcosa in Modifica, esportare.

Per i workflow basta il primo push: `verifica.yml` parte su qualsiasi commit,
`rilascio.yml` su un tag `v0.1.0`.
