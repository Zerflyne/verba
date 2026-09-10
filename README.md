# Verba — sottotitoli automatici in locale

Trascrizione audio con **mappatura testuale parola per parola** e **sottotitoli
grafici su sfondo trasparente**, pronti da sovrapporre a un video nel montaggio.

| Fase | Modello | Runtime |
|---|---|---|
| Pre-elaborazione | — | Symphonia + rubato, tutto in RAM |
| Segmentazione | `pyannote/segmentation-3.0` | ONNX Runtime |
| Trascrizione | `whisper-large-v3` | whisper.cpp (GGML) |
| Allineamento | `wav2vec2` italiano (CTC) | ONNX Runtime |
| Impaginazione | — | cosmic-text, font Inter 700, una riga per volta |
| Disegno | — | RGBA con alfa: testo + rettangolo di evidenziazione |
| Codifica | — | libavcodec/libavformat (C++), `prores_ks` 4444 |
| Uscita | — | MOV ProRes 4444 con canale alfa (+ SRT e JSON opzionali) |

La trascrizione **non passa da alcun file intermedio**: resta in RAM e alimenta
direttamente l'impaginazione e il disegno dei fotogrammi.

I tre modelli non sono mai residenti insieme: ogni fase carica il proprio
modello e lo rilascia prima della successiva. In particolare **Whisper viene
scaricato esplicitamente dalla GPU prima che l'allineatore venga caricato**
(`Transcriber::release()`), e l'occupazione di VRAM viene tracciata nel log
prima e dopo lo scarico.

## Architettura

```
crates/
  verba-core/            libreria: tutta la logica, nessuna dipendenza da UI
    src/
      audio.rs           pre-elaborazione: multi-formato -> mono -> 16 kHz -> normalizzazione (0 file temporanei)
      segmentation.rs    pyannote ONNX: finestre da 10 s, powerset/multi-label, isteresi
      transcribe.rs      Whisper large-v3 via whisper.cpp; release() libera la VRAM
      align.rs           wav2vec2 ONNX + Viterbi CTC: intervallo temporale di ogni parola;
                         ripulisci() normalizza la sequenza (buchi, ordine, durate)
      layout.rs          impaginazione: righe, chunk, finestre di accensione
      render.rs          disegno RGBA: maschere, contorno, rettangolo smussato
      encoder.rs         ponte FFI verso l'encoder C++
      video.rs           dalla linea temporale ai fotogrammi codificati
      srt.rs             battute blocchi / word / line / karaoke, timestamp HH:MM:SS,mmm
      prompt.rs          initial prompt di Whisper da file CSV di termini
      gpu.rs             selezione GPU su VRAM totale, monitoraggio NVML
      onnx.rs            sessioni ORT condivise, softmax / log-softmax
    cpp/
      encoder.h/.cpp     ProRes 4444 con alfa su libavcodec + libavformat
    assets/
      Inter-Bold.ttf     Inter statico peso 700, incorporato nel binario
  verba-cli/             binario `verba`: orchestrazione a fasi e riga di comando
assets/
  fonts/                 caratteri aggiuntivi offerti nel selettore
docs/
  piano.md               piano di costruzione della 0.1
scripts/
  export_models.py       esporta pyannote e wav2vec2 in ONNX
esempi/
  vocabolario.csv        CSV di esempio per l'initial prompt
```

### `verba-core/src/audio.rs` — pre-elaborazione (modulo separato, come richiesto)

* **Input misto**: percorsi di file, `-` per stdin, o buffer gia' in memoria
  (`AudioInput::Bytes`); piu' sorgenti nella stessa invocazione vengono
  concatenate dopo il ricampionamento individuale.
* **Multi-formato**: tutto cio' che Symphonia decodifica (MP3, WAV, FLAC,
  OGG/Vorbis, AAC/M4A, MP4, MKV, AIFF...). Se un formato non e' supportato,
  fallback opzionale su `ffmpeg` in *pipe mode* (`pipe:0` -> `pipe:1`).
* **Downmix a mono**: media aritmetica dei canali (niente raddoppio di ampiezza).
* **16 kHz**: ricampionamento sinc band-limited (rubato, finestra
  Blackman-Harris, sinc a 256 tap).
* **Normalizzazione**: rimozione dell'offset DC, guadagno verso un target RMS
  (default −20 dBFS) vincolato da un tetto sul picco (−1 dBFS) e da un guadagno
  massimo (+30 dB), clamp finale in [−1, 1].
* **Zero file temporanei**: nessun percorso su disco viene mai scritto, neppure
  nel fallback ffmpeg.

Verificabile senza modelli:

```bash
cargo run --release -- prova.mp3 --solo-audio -v
```

### Dai tempi delle parole al fotogramma

**`layout.rs` — impaginazione.** Le parole allineate diventano *righe*, e a
schermo ne compare **una sola per volta**: piu' righe insieme rendono la lettura
caotica, e con l'evidenziazione che salta da una parola all'altra lo sguardo non
saprebbe dove stare. Una riga si chiude quando succede una di queste cose: una
pausa piu' lunga di `--pausa-blocco`, un cambio di segmento di pyannote, il
superamento di `--durata-blocco`, la punteggiatura di fine frase, oppure — ed e'
il vincolo grafico — la parola successiva non ci starebbe piu'.

La capienza non e' stimata a caratteri: ogni riga candidata viene **misurata con
cosmic-text sul font che verra' davvero disegnato**. In Inter 700 le stringhe
`illlli` e `WWWWWW` hanno lo stesso numero di caratteri e larghezze che
differiscono di piu' del doppio; contare i caratteri farebbe uscire il testo dai
bordi. Lo spazio disponibile e' la larghezza del fotogramma meno due volte
`--margine`.

**`render.rs` — disegno.** Il fotogramma e' RGBA con **alfa dritta** (non
premoltiplicata) e sfondo completamente trasparente. L'ordine di sovrapposizione
e' rettangolo, contorno, testo: il rettangolo sta **dietro**, e il testo sopra
resta del suo colore. Il disegno e' in due passate:

1. per riga, una volta sola: i glifi diventano una maschera di copertura, da cui
   il contorno si ricava con una trasformata di distanza chamfer a due passate —
   lineare nell'area invece che proporzionale al quadrato del raggio. Dagli
   stessi glifi si ricava anche il rettangolo di ogni parola;
2. a ogni cambio di parola indicata: si ridisegna il rettangolo e si ricompone
   la maschera gia' pronta. La geometria del testo non cambia dentro una riga,
   quindi il costo per fotogramma resta trascurabile.

La pulizia della tela agisce solo sul riquadro effettivamente sporcato dal
disegno precedente.

### Il rettangolo di evidenziazione

Dietro la parola in corso di pronuncia c'e' un rettangolo pieno con gli angoli
arrotondati, di colore configurabile (viola di default). Non e' un cambio di
colore del testo: il testo resta bianco sopra.

**La geometria viene dallo shaping, non da una nuova misura.** Gli estremi
orizzontali della parola sono il minimo e il massimo dei riquadri d'avanzamento
dei glifi che cosmic-text ha gia' posizionato nella riga, riconosciuti
dall'intervallo di byte che la parola occupa nel testo. Rimisurare la parola
isolata darebbe una larghezza diversa — crenatura con i vicini, spazi che
cadono dentro o fuori — e il rettangolo si scosterebbe progressivamente dal
testo lungo la riga.

**L'altezza viene dal corpo, non dai limiti dei glifi.** Se derivasse
dall'inchiostro, "pagina" (con discendente) e "come" (senza) avrebbero
rettangoli di forma diversa e la riga sembrerebbe ballare a ogni parola.
L'altezza e' quindi `--altezza-evidenziazione` volte il corpo, centrata
sull'altezza della fascia di riga. Anche il padding orizzontale e il raggio
degli angoli sono frazioni del corpo, cosi' la forma resta identica a qualunque
risoluzione.

Il rettangolo e' uno sfondo, non testo: puo' sconfinare nel margine laterale
della misura del padding. Con i valori di default sono 13 px dentro un margine
di 86 px.

**L'accensione e' temporale**, con tre correzioni:

* `--anticipo` — il rettangolo arriva un istante prima dell'inizio nominale
  della parola. Il sistema visivo e' piu' lento di quello uditivo: senza
  anticipo il rettangolo sembra sempre in ritardo;
* `--pausa-massima` — tetto alla permanenza nel silenzio che segue la parola.
  Senza tetto, in una pausa di due secondi il rettangolo resterebbe acceso su
  una parola che non si sta piu' pronunciando. Se la parola successiva arriva
  prima del tetto, il rettangolo le salta addosso senza spegnersi;
* `--coda` — permanenza dopo l'ultima parola della riga, comunque non oltre
  `--tenuta`, che e' quando la riga sparisce.

Quando nessuna finestra contiene l'istante corrente non si disegna alcun
rettangolo: la riga resta a schermo da sola. Il rettangolo **si sposta a scatti**
da una parola all'altra, senza interpolazione: la posizione e' quella della
parola indicata e basta.

**`cpp/encoder.cpp` — codifica.** Contenitore MOV, encoder `prores_ks` in
**profilo 4444**, l'unico ProRes che trasporta il canale alfa, in `yuva444p10le`:
4:4:4 senza sottocampionamento della crominanza, cosi' i bordi del testo restano
netti. La conversione RGBA -> YUVA e' fatta a mano con i coefficienti BT.709
(luminanza e crominanza in range video, alfa a range pieno) invece che con
swscale: e' poche righe, e toglie ogni dubbio su come venga trattata l'alfa.

**`video.rs` — linea temporale.** Il tempo campionato e' il **centro** del
fotogramma. I fotogrammi consecutivi con lo stesso stato (stessa riga, stessa
parola indicata) vengono raggruppati: la conversione colore avviene una volta sola
e il fotogramma gia' convertito viene ricodificato. Su un audio di 9,6 s a 30 fps
questo significa 26 disegni invece di 290.

## Prerequisiti di build

```bash
sudo apt install build-essential cmake clang libclang-dev pkg-config \
                 libavcodec-dev libavformat-dev libavutil-dev
```

`cmake` compila whisper.cpp; `clang` serve a bindgen; gli header `libav*`
servono all'encoder video in `cpp/`. Se gli header non sono installati a livello
di sistema si possono indicare a mano:

```bash
FFMPEG_INCLUDE_DIR=/percorso/include FFMPEG_LIB_DIR=/percorso/lib cargo build --release
``` Se bindgen non trova
`stdbool.h` (accade con clang senza i propri header di sistema):

```bash
export BINDGEN_EXTRA_CLANG_ARGS="-I$(dirname $(find /usr/lib/gcc -name stdbool.h | head -1))"
```

### ONNX Runtime

Il crate `ort` e' configurato in `load-dynamic`: la libreria viene caricata a
runtime, quindi si sceglie liberamente la build CPU o GPU.

```bash
# build GPU ufficiale (CUDA 12 + cuDNN 9)
wget https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/onnxruntime-linux-x64-gpu-1.22.0.tgz
tar xf onnxruntime-linux-x64-gpu-1.22.0.tgz
export ORT_DYLIB_PATH=$PWD/onnxruntime-linux-x64-gpu-1.22.0/lib/libonnxruntime.so
```

La versione e' vincolata: `ort 2.0.0-rc.10` legge `GetVersionString` all'avvio
e accetta **solo la 1.22.x**.

In alternativa, `--features ort-download` fa scaricare i binari a `ort`
(richiede `libssl-dev`).

#### Librerie CUDA a runtime

`libonnxruntime_providers_cuda.so` non porta con se' le librerie CUDA: le cerca
via `LD_LIBRARY_PATH`. Oltre a `cudart`, `cublas` e `cudnn` gli servono anche
`curand`, `cufft` e `nvrtc`. Se una sola manca, il provider CUDA **non si
registra e l'inferenza ricade su CPU senza alcun messaggio**.

Verifica prima di lanciare (nessuna riga in uscita = tutto a posto):

```bash
ldd $ORT_DYLIB_PATH/../libonnxruntime_providers_cuda.so | grep "not found"
```

Se sul sistema non c'e' un CUDA toolkit, le librerie di un ambiente Python con
PyTorch vanno benissimo (CUDA 12 + cuDNN 9 sono le versioni richieste):

```bash
NV=/percorso/venv/lib/python3.12/site-packages/nvidia
export LD_LIBRARY_PATH="$(find "$NV" -maxdepth 2 -type d -name lib -printf '%p:' | sed 's/:$//')${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
```

### Build

```bash
cargo build --release                     # solo CPU
cargo build --release --features vulkan   # Whisper su GPU via Vulkan
cargo build --release --features cuda     # Whisper su GPU via CUDA
```

Le feature riguardano **solo Whisper**: ONNX Runtime prende la GPU senza feature
aggiuntive, gli basta che `ORT_DYLIB_PATH` punti a una build GPU.

|  | Richiede | Note |
|---|---|---|
| `cuda` | CUDA toolkit con `nvcc` (2-3 GB) | La piu' veloce. whisper.cpp compila i kernel in fase di build. |
| `vulkan` | `libvulkan-dev` e `glslc` (~50 MB) | Piu' lenta di CUDA, molto piu' veloce della CPU. Funziona anche sulle schede abbandonate dai toolkit recenti (Tesla P40, compute 6.1). |

```bash
sudo apt install libvulkan-dev glslc     # per --features vulkan
```

whisper.cpp con Vulkan enumera i dispositivi per conto suo e non segue
`--gpu-index`: se sceglie la scheda sbagliata, si forza con
`GGML_VK_VISIBLE_DEVICES=0` davanti al comando.

## Modelli

```bash
# 1. Whisper large-v3 (GGML per whisper.cpp)
mkdir -p models && cd models
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin
cd ..

# 2. pyannote + wav2vec2 esportati in ONNX
pip install "torch>=2.2" onnx transformers pyannote.audio huggingface_hub
python scripts/export_models.py --all --hf-token hf_xxx
```

`pyannote/segmentation-3.0` e' un modello *gated*: vanno accettate le
condizioni sulla sua pagina Hugging Face e serve un token.

Il modello di allineamento predefinito e'
`jonatasgrosman/wav2vec2-large-xlsr-53-italian`; se ne puo' usare un altro con
`--w2v-model`. Lo script salva anche il `vocab.json`, da cui il programma
deduce da solo blank CTC, delimitatore di parola e maiuscolo/minuscolo.

**Attenzione al language model.** Quel repo contiene una cartella
`language_model/` e il suo `preprocessor_config.json` dichiara
`"processor_class": "Wav2Vec2ProcessorWithLM"`, quindi `AutoProcessor` pretende
`pyctcdecode` e l'esportazione fallisce. Il LM non serve a niente qui: lo script
legge dal processor solo il vocabolario, e l'allineamento CTC lo fa Viterbi in
Rust. La via pulita e' una cartella specchio senza la parte LM, con i pesi
lasciati come symlink:

```bash
S=/percorso/wav2vec2-italian; D=/percorso/wav2vec2-senza-lm
mkdir -p "$D" && ln -sf "$S/pytorch_model.bin" "$D/" \
  && cp "$S/config.json" "$S/vocab.json" "$S/special_tokens_map.json" "$D/" \
  && python3 -c "import json; d=json.load(open('$S/preprocessor_config.json')); d['processor_class']='Wav2Vec2Processor'; json.dump(d,open('$D/preprocessor_config.json','w'),indent=2)"
```

poi `--w2v-model "$D"`.

Risultato atteso in `models/`:

```
ggml-large-v3.bin
pyannote-segmentation-3.0.onnx
wav2vec2-italian.onnx
wav2vec2-italian.vocab.json
```

## Initial prompt da CSV

Whisper accetta un testo di contesto che precede idealmente l'audio: e' la via
ufficiale per orientarlo su nomi propri, sigle e termini tecnici. Le parole con
cui inizializzare il modello si mettono in un CSV:

```csv
# esempi/vocabolario.csv
termine,categoria,note
Anthropic,azienda,
Claude Opus,modello,
wav2vec2,tecnico,allineatore CTC
"Milano, Italia",luogo,esempio con virgola nel termine
```

```bash
verba prova.mp3 --prompt-csv esempi/vocabolario.csv
```

Il parser e' tollerante e non richiede configurazione:

* **delimitatore** dedotto fra `,` `;` `\t` `|` (forzabile con `--prompt-delimiter`);
* **intestazione** riconosciuta e scartata da sola; un elenco di nomi propri
  non viene mai scambiato per un header;
* **colonna** selezionabile per nome (`--prompt-column termine`) o per indice
  (`--prompt-column 1`); default: la prima. Una colonna inesistente e' un
  errore esplicito che elenca quelle disponibili;
* **virgolette** RFC 4180, quindi un termine puo' contenere virgole;
* righe vuote e commenti `#` ignorati, duplicati rimossi ignorando
  maiuscole/minuscole ma conservando la prima grafia — che e' quella suggerita
  al modello.

Il CSV si combina con il prompt libero, che lo precede:

```bash
verba prova.mp3 \
  --prompt-csv esempi/vocabolario.csv \
  --prompt "Intervista tecnica in italiano." \
  --prompt-preamble "Termini ricorrenti:"
# -> "Intervista tecnica in italiano. Termini ricorrenti: Anthropic, Claude Opus, ..."
```

**Limite di lunghezza.** whisper.cpp accetta al massimo `n_text_ctx / 2` token
di contesto (224 per large-v3) e un prompt piu' lungo verrebbe troncato in modo
cieco, magari a meta' di un nome. Il default di 700 caratteri
(`--prompt-max-chars`) resta sotto quel limite anche nel caso peggiore; i
termini in eccesso vengono scartati **a termine intero** e il log dice quanti.
Con un vocabolario molto grande conviene quindi ordinare il CSV per importanza:
i primi termini sono quelli che arrivano al modello.

Per vedere il prompt senza trascrivere nulla:

```bash
verba prova.mp3 --prompt-csv esempi/vocabolario.csv --solo-prompt
```

Il prompt viene costruito e validato **prima** della decodifica audio e del
caricamento dei modelli: un CSV malformato fallisce subito.

## Uso

### Prima di lanciarlo

Tre condizioni, tutte necessarie.

1. **Le due variabili d'ambiente**, `ORT_DYLIB_PATH` e `LD_LIBRARY_PATH` (vedi
   *ONNX Runtime*). Conviene metterle nel `~/.bashrc`: senza, ONNX Runtime
   ricade su CPU in silenzio.
2. **I quattro file dei modelli raggiungibili da `models/`.** Il percorso e'
   relativo alla cartella da cui lanci, quindi o ci si posiziona dentro, o si
   passano i percorsi assoluti con `--whisper-model`, `--segmentation-model`,
   `--align-model`, `--align-vocab`.
3. **La build fatta**, `target/release/verba`.

```bash
cd /percorso/di/Verba
```

### Esecuzione

Forma minima: scrive `nomefile.mov` accanto al sorgente — un ProRes 4444 in 9:16
con sfondo trasparente, da mettere sopra il video nel montaggio.

```bash
./target/release/verba /percorso/del/file.mp4
```

Formato orizzontale, contorno nero, e anche l'SRT e il JSON:

```bash
./target/release/verba /percorso/del/file.mp4 --formato 16:9 --bordo 5 \
    -o sottotitoli.mov --srt sottotitoli.srt --json parole.json -v
```

Risoluzione e frame rate espliciti (il frame rate accetta interi, decimali e
frazioni: `30`, `29.97`, `30000/1001`):

```bash
./target/release/verba intervista.m4a --risoluzione 2160x3840 --fps 29.97
```

Stile: rettangolo arancione piu' schiacciato e piu' squadrato, contorno spesso,
sottotitoli a meta' altezza.

```bash
./target/release/verba intervista.m4a \
    --colore-evidenziazione '#F97316' --altezza-evidenziazione 0.95 \
    --raggio-evidenziazione 0.08 --colore-bordo '#101010' --bordo 8 \
    --posizione centro
```

Accensione: rettangolo piu' in anticipo, che si spegne prima nei silenzi e non
resta in coda alla riga.

```bash
./target/release/verba intervista.m4a --anticipo 0.12 --pausa-massima 0.15 --coda 0
```

Nessun rettangolo, righe piu' lunghe:

```bash
./target/release/verba intervista.m4a --senza-evidenziazione --durata-blocco 8
```

Sovrapposizione al video sorgente, per vedere il risultato:

```bash
ffmpeg -i video.mp4 -i sottotitoli.mov -filter_complex overlay -c:a copy anteprima.mp4
```

Piu' sorgenti concatenate, e ingresso da stdin:

```bash
./target/release/verba parte1.wav parte2.mp3
cat registrazione.opus | ./target/release/verba - -o out.mov
```

Solo pre-elaborazione audio, con statistiche: non carica alcun modello, ed e'
il modo piu' rapido per verificare che la build regga.

```bash
./target/release/verba prova.mp3 --solo-audio -v
```

### Averlo nel PATH

```bash
ln -s "$PWD/target/release/verba" ~/.local/bin/verba
```

Cosi' pero' si perde il `models/` relativo: lanciandolo da fuori dalla cartella
del progetto vanno passati i quattro percorsi assoluti dei modelli. In
alternativa si tiene un alias che entra prima nella cartella.

### Cosa aspettarsi

* **Formati**: qualsiasi cosa Symphonia decodifichi, quindi anche un MP4 o un
  MKV direttamente, senza estrarre prima l'audio.
* **Picco di VRAM**: circa 4,3 GB con Whisper large-v3 caricato. Le fasi non
  convivono mai — Whisper viene scaricato prima che l'allineatore parta — quindi
  quello e' il massimo, non la somma.
* **Log da controllare** con `-v`: che la sessione ONNX dichiari
  `device=CUDA:0` e non CPU; che la VRAM scenda alla riga *dopo lo scarico di
  Whisper*; e quale scheda annuncia `whisper_default_buffer_type`, che con
  Vulkan puo' non essere quella di `--gpu-index`.
* **Dimensione del file**: ProRes 4444 e' un codec da montaggio, non da
  distribuzione. A 1080x1920, 30 fps e `--qualita 4` sono circa 3,4 MB/s. Con
  `--qualita` piu' alta il file cala, a scapito della nitidezza dei bordi.
* **Tempo di codifica**: sull'audio di prova (9,6 s) la codifica occupa circa
  5 s, cioe' meta' del tempo totale. Cresce con la risoluzione e con il frame
  rate, non con il numero di parole.

### Il file video

Il MOV contiene una sola traccia video: nessun audio, nessun video di fondo,
solo i sottotitoli su trasparenza. In un montaggio va messo su una traccia
superiore a quella del video; il canale alfa viene riconosciuto da Resolve,
Premiere, Final Cut e da `ffmpeg -filter_complex overlay`.

L'alfa e' **dritta** (non premoltiplicata). Se il programma di montaggio chiede
come interpretarla, va scelta *straight* / *non premultiplied*.

### Opzioni principali

**Formato del video**

| Opzione | Default | Descrizione |
|---|---|---|
| `-o`, `--output` | `<input>.mov` | file MOV ProRes 4444 di uscita |
| `--formato 9:16\|16:9` | `9:16` | proporzioni del fotogramma |
| `--risoluzione LxA` | dal formato | risoluzione esplicita, dimensioni pari |
| `--fps` | `30` | intero, decimale o frazione (`30000/1001`) |
| `--durata` | durata audio | durata del video in secondi |
| `--qualita` | `4` | quantizzatore ProRes: piu' basso, piu' qualita' |

**Tipografia e impaginazione**

| Opzione | Default | Descrizione |
|---|---|---|
| `--font` | Inter 700 incorporato | file `.ttf` alternativo |
| `--dimensione-font` | 6,5 % del lato minore | corpo in pixel |
| `--margine` | `0.08` | margine laterale, frazione della larghezza |
| `--margine-verticale` | `0.14` | distanza dal bordo, frazione dell'altezza |
| `--posizione alto\|centro\|basso` | `basso` | collocazione verticale |
| `--interlinea` | `1.18` | multiplo del corpo; e' la fascia su cui il rettangolo e' centrato |
| `--durata-blocco` | `5.0` | durata massima di una riga, in secondi |
| `--pausa-blocco` | `0.7` | pausa che chiude la riga, in secondi |
| `--tenuta` | `0.30` | permanenza della riga dopo l'ultima parola, in secondi |

**Evidenziazione**

| Opzione | Default | Descrizione |
|---|---|---|
| `--colore-evidenziazione` | `#7C3AED` | rettangolo dietro la parola in corso |
| `--padding-evidenziazione` | `0.18` | margine oltre la parola, in frazione del corpo |
| `--altezza-evidenziazione` | `1.12` | altezza del rettangolo, in frazione del corpo |
| `--raggio-evidenziazione` | `0.20` | raggio degli angoli, in frazione del corpo |
| `--anticipo` | `0.06` | quanto il rettangolo precede la parola, in secondi |
| `--pausa-massima` | `0.35` | tetto alla permanenza nel silenzio, in secondi |
| `--coda` | `0.25` | permanenza dopo l'ultima parola della riga, in secondi |
| `--senza-evidenziazione` | off | non disegnare il rettangolo |

**Stile del testo**

| Opzione | Default | Descrizione |
|---|---|---|
| `--colore` | `#FFFFFF` | testo, `#RRGGBB` o `#RRGGBBAA` |
| `--colore-bordo` | `#000000` | contorno del testo |
| `--bordo` | `0.0` | spessore del contorno in pixel |

**Trascrizione, uscite accessorie e dispositivo**

| Opzione | Default | Descrizione |
|---|---|---|
| `--srt FILE` | — | esporta anche l'SRT |
| `--srt-mode blocchi\|word\|line\|karaoke` | `blocchi` | struttura dell'SRT: `blocchi` = una battuta per riga a schermo |
| `--srt-max-chars` | `84` | caratteri per battuta in `line` e `karaoke` |
| `--json FILE` | — | mappatura parola-per-parola in JSON |
| `--language` | `it` | lingua Whisper (`auto` per rilevamento) |
| `--beam-size` | `5` | ampiezza del beam search |
| `--prompt` | — | prompt iniziale libero (stile, punteggiatura) |
| `--prompt-csv` | — | CSV con le parole di inizializzazione |
| `--prompt-column` | prima | colonna del CSV, per nome o indice |
| `--prompt-delimiter` | auto | forza il delimitatore del CSV |
| `--prompt-preamble` | — | testo davanti all'elenco dei termini |
| `--prompt-max-chars` | `700` | limite del prompt (~224 token Whisper) |
| `--solo-prompt` | off | stampa l'initial prompt ed esce |
| `--min-vram-mib` | `8000` | VRAM **totale** minima per usare la GPU |
| `--gpu-index` | auto | forza una GPU specifica |
| `--cpu` | off | forza la CPU |
| `--normalize none\|peak\|rms` | `rms` | strategia di normalizzazione |
| `--target-dbfs` | `-20` | target RMS |
| `--onset` / `--offset` | `0.50` / `0.35` | soglie di isteresi di pyannote |
| `--no-segmentation` | off | finestre uniformi al posto di pyannote |
| `--solo-audio` | off | solo pre-elaborazione, con statistiche |

### Selezione della GPU

La regola e' **VRAM totale >= soglia**, indipendentemente da quanta memoria sia
libera al momento: le fasi si susseguono una alla volta, quindi la memoria
occupata da altri processi al momento della scelta non e' un criterio utile.
Fra le GPU idonee vince quella con piu' VRAM totale.

La soglia di default e' 8000 MiB e non 8192: una scheda "da 8 GB" espone spesso
8188 MiB, e una soglia in GiB stretti la escluderebbe per 4 MiB.

## Come nascono i tempi delle parole

Whisper fornisce il testo, non tempi affidabili a livello di parola. Questi
arrivano da un **allineamento forzato CTC**:

1. wav2vec2 produce log-probabilita' per frame (~20 ms) sui caratteri;
2. il testo di Whisper viene convertito nella sequenza di token del
   vocabolario, con `|` fra le parole;
3. Viterbi sulla sequenza estesa `[blank, c1, blank, c2, ...]` trova il
   percorso ottimo, cioe' quali frame appartengono a quale carattere;
4. i frame si aggregano in intervalli di parola, convertiti in secondi e
   traslati sull'inizio del segmento.

Se l'allineamento di un intero segmento fallisce, si ricade su una
ripartizione proporzionale alla lunghezza delle parole. Ogni parola porta con
se' una confidenza in `[0, 1]`, esportata nel JSON.

### `ripulisci`: l'invariante della sequenza

Tutto cio' che sta a valle — raggruppamento in battute, resa SRT, export JSON —
assume una sequenza **ordinata e senza buchi**. Quell'invariante viene
stabilita in un solo punto, `align::ripulisci`, che:

* scarta le parole vuote;
* **riempie i timestamp mancanti**: l'allineatore CTC non aggancia numeri e
  simboli, che non hanno una grafia nel vocabolario dei caratteri. Il tempo si
  ricava interpolando fra i vicini noti, e quando le parole senza tempo sono
  piu' d'una di fila l'intervallo viene spartito equamente fra loro; agli
  estremi le ancore sono 0 e la durata dell'audio. Queste parole restano
  riconoscibili dalla confidenza a 0;
* impone **monotonia** (nessuna parola inizia prima che finisca la precedente),
  **durata minima** per parola e **troncamento alla durata dell'audio**.

Gli ultimi due vincoli possono entrare in conflitto in coda al file: li' vince
il troncamento, perche' un sottotitolo che punta oltre la fine del media e' un
errore visibile mentre una battuta corta non lo e'. Per lo stesso motivo la
resa SRT, che allunga le battute fino a `min_duration`, tronca poi ciascuna
all'inizio della successiva: la durata minima non deve reintrodurre le
sovrapposizioni che `ripulisci` ha eliminato.

## Test

```bash
cargo test
```

79 test coprono ricampionamento (lunghezza e assenza di deriva temporale),
normalizzazione, Viterbi CTC (inclusi i token ripetuti che richiedono un blank
di separazione), la normalizzazione di `ripulisci` (interpolazione dei buchi,
spartizione equa, monotonia, durata minima, troncamento, idempotenza),
l'assenza di sovrapposizioni nelle battute SRT, il parsing del CSV dei termini
(delimitatori, intestazioni, virgolette, duplicati, troncamento) e la parte
grafica: misura del testo con il font reale, righe che restano dentro i margini,
righe che non si sovrappongono e coprono tutte le parole, sfondo che resta
trasparente, contorno che allarga la sagoma, frame rate NTSC che resta una
frazione esatta.

Sul rettangolo di evidenziazione in particolare: che stia dietro al testo (che
resta bianco), che avvolga i glifi della parola indicata, che superi la parola
esattamente del padding, che l'altezza **non** dipenda dai discendenti, che sia
centrato sulla fascia di riga, che gli angoli siano smussati, che salti da una
parola all'altra, e che non venga disegnato quando nessuna parola e' attiva. Sulle
finestre di accensione: anticipo, tetto alla pausa, coda, salto senza spegnimento
fra parole vicine, e finestre sempre ordinate e disgiunte dentro la riga.

L'encoder C++ non e' coperto dai test unitari: si verifica sull'uscita reale,
con `ffprobe` (`profile=4444`, `pix_fmt=yuva444p...`) e sovrapponendo il MOV a
un fondo pieno.
