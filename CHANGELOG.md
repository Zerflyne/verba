# Diario delle versioni

Le versioni seguono [SemVer](https://semver.org/lang/it/). Le voci sono scritte
per chi usa Verba, non per chi ne legge i commit.

## Non ancora rilasciato

### Aggiunto
- **Riga di comando a sottocomandi**: `verba trascrivi`, `verba rendi`,
  `verba overlay`, piu' `caratteri`, `preset`, `formati` e `modelli`. Il
  formato di uscita si sceglie con l'estensione di `--out`.
- **`--json`** su qualsiasi comando: l'avanzamento diventa un oggetto JSON per
  riga su stderr, integrabile in un altro script.
- **Scaricamento dei modelli** con verifica SHA-256 e ripresa dopo
  un'interruzione (`verba modelli --scarica`), nella cartella dati dell'utente.
- **Selettore della dimensione** del modello di trascrizione: `large-v3`,
  `medium`, `small`, con il compromesso scritto invece che lasciato indovinare.
- **Ingresso video**: `.mp4`, `.mov`, `.mkv`, `.webm`, `.avi`. L'audio viene
  estratto, e il filmato fa da base per l'anteprima e per l'export.
- **Quattro formati video di uscita**: H.264, ProRes 422 HQ, ProRes 4444 con
  alfa, VP9 con alfa. La traccia audio del sorgente viene ricopiata senza
  ricodifica nei due formati impressi.
- **Uscite testuali**: `.srt`, `.vtt`, `.json` parola per parola, `.txt`.
- **Righe multiple** (1, 2 o 3) con spezzatura equilibrata, allineamento,
  larghezza e posizione della colonna, maiuscole.
- **Forme dell'evidenziazione**: rettangolo, sottolineatura, solo colore,
  nessuna. Piu' ombra e contorno del testo.
- **Sei caratteri di serie** da Google Fonts, i font di sistema a richiesta, e
  la possibilita' di aggiungere un `.ttf` scaricato per conto proprio.
- **Preset** salvabili e ricaricabili in JSON, con tre di serie: `Verticale`,
  `Orizzontale`, `Sobrio`.
- **Applicazione desktop** (Tauri v2 + React): quattro sezioni, anteprima
  navigabile, striscia di parole con le incerte segnalate.
- **Annullamento reale**: `Ctrl-C` o il pulsante *Annulla* fermano
  l'elaborazione al primo punto utile e cancellano il file parziale.
- **Riproduzione dell'audio** nell'anteprima. Il motore scrive un WAV
  temporaneo dal PCM gia' decodificato, cosi' si sente qualunque formato Verba
  sappia aprire — e si sente esattamente cio' su cui hanno lavorato i modelli.
  Quando c'e', e' l'audio a fare da orologio: la posizione mostrata e' la sua,
  non un contatore parallelo che deriverebbe.
- **Scelta della GPU** in Impostazioni, su una macchina che ne ha piu' d'una.
  La scelta automatica prende quella con piu' memoria, che non e' sempre la
  piu' veloce. Chiedere «GPU» esplicitamente toglie anche la soglia minima di
  VRAM: negarla in silenzio sarebbe un modo elaborato di ignorare
  un'impostazione.
- **Editor dei termini noti** integrato, raggiungibile dalle Impostazioni e —
  segnato come *consigliato* — dalla schermata di caricamento, prima di
  scegliere il file. I termini vanno decisi li': applicarli dopo vuol dire
  ritrascrivere. Restano salvati in CSV, quindi chi ne aveva gia' uno se lo
  ritrova e chi non ne aveva non deve piu' procurarselo.
- **Whisper e allineatore caricati insieme** quando la memoria libera supera
  del 20% la somma stimata dei due. Dove non li supera resta la staffetta di
  prima, che su una scheda da 8 GB e' l'unica strada. La decisione finisce nel
  log con i numeri che l'hanno prodotta.

### Cambiato
- **Un file di solo audio si comporta come un video.** L'anteprima mostra i
  sottotitoli su nero invece che su una scacchiera, e la sezione Esporta offre
  i formati video che conservano la trasparenza — prima li nascondeva tutti,
  lasciando le sole uscite testuali.
- **Quando mancano i modelli lo dice la schermata di caricamento**, al posto
  dell'area di trascinamento, con l'elenco di cosa manca e il pulsante per
  scaricarlo. Prima era una riga nella barra di stato, in basso a sinistra, e
  restava da capire perche' la trascrizione non partisse.
- Le opzioni della riga di comando sono in italiano (`--lingua`, `--termini`,
  `--thread`, `--modello-whisper`, `--gpu`, …). I nomi inglesi di prima restano
  accettati come alias.
- Il log passa su **stderr** insieme all'avanzamento: su stdout resta solo cio'
  che il comando ha il compito di stampare.
- `libonnxruntime` viene cercata accanto all'eseguibile, nella cartella dati e
  nel sistema prima di guardare `ORT_DYLIB_PATH`. Se manca, l'errore dice cosa
  fare invece di far andare in panico il caricatore.
- Il provider CUDA di ONNX Runtime viene registrato in modo **osservabile**: se
  non e' utilizzabile si continua su CPU e lo si scrive, invece di ricadere in
  silenzio.
- La segmentazione usa l'esportazione ONNX pubblica di `onnx-community`, che non
  e' *gated*: al primo avvio non servono ne' account ne' token.
- La durata minima di una parola passa da 40 a 80 ms.

### Corretto
- **Il pacchetto `.deb` si installava e poi non partiva**, senza dire niente:
  `dpkg` non segnalava nulla perche' il pacchetto non dichiarava le librerie di
  FFmpeg fra le sue dipendenze, e il programma moriva all'avvio su
  `libavcodec.so.58: cannot open shared object file`. Ora le dichiara, cosi'
  `apt` rifiuta l'installazione dove non possono essere soddisfatte invece di
  lasciare un'icona che non fa nulla. Il pacchetto e' costruito sulla 24.04,
  i cui numeri di versione sono quelli delle distribuzioni in uso adesso.
- `libonnxruntime` non veniva trovata dentro un `.deb`: le risorse finiscono in
  `/usr/lib/Verba/lib`, e la ricerca si fermava un livello sopra. Era un
  secondo difetto, nascosto dietro il primo — il programma non arrivava mai
  abbastanza avanti per accorgersene.
- **Il comando per esportare l'allineatore era sbagliato in tutti i posti dove
  era scritto**, compreso il messaggio che il programma mostra quando quel file
  manca: `--w2v` e' un prefisso che argparse risolve in `--w2v-model` e
  fallisce con «expected one argument». Il flag e' `--wav2vec2`. Chi ha provato
  a seguire l'istruzione non ha ottenuto niente, ed e' il passaggio che sta fra
  l'installazione e i tempi parola per parola.
- Il `vocab.json` dell'allineatore ora ha un'impronta SHA-256 dichiarata. Era
  l'unico file del catalogo scaricato senza verifica.
- **L'audio dell'anteprima non si sentiva, e la causa non era quella che
  sembrava.** Il primo sospetto era lo schema: la traccia arrivava come
  indirizzo `asset://`, e WebKitGTK rifiuta gli schemi personalizzati per i
  media. Passando i byte come `blob:` in sviluppo funzionava — e nel pacchetto
  no, con lo stesso «formato o indirizzo non supportati» di prima.
  Il vero difetto stava due strati piu' sotto, ed era **uno solo per tutta
  l'applicazione**: la CSP dichiarava `default-src 'self'` senza concedere
  niente a `connect-src`, e la `fetch` con cui Tauri parla col motore punta a
  `ipc://localhost`. Bloccata. Tauri allora ripiega in silenzio su
  `postMessage`, dove una risposta di byte grezzi viene serializzata come
  **array JSON di numeri**. Il WAV non era piu' un WAV: `new Blob([array])`
  scrive quei numeri come testo, e 288 kB di audio diventavano un documento di
  1 MB che GStreamer, con pieno diritto, chiamava «file di testo».
  Era invisibile perche' si manifestava in un posto solo. `new
  Uint8ClampedArray(array)` accetta un array di numeri, quindi i fotogrammi
  comparivano come sempre: nessuno poteva sospettare che ogni risposta binaria
  stesse viaggiando per la via lenta, tre volte piu' grande del necessario.
  In sviluppo non si vedeva perche' la pagina la serve Vite, senza CSP: la
  `fetch` passava e i byte arrivavano interi.
  Ora la CSP concede `connect-src 'self' ipc: http://ipc.localhost`, e i byte
  di ogni risposta binaria passano da un punto solo che li accetta in
  entrambe le forme — se un domani l'IPC dovesse ripiegare di nuovo, si perde
  velocita' e non la riproduzione, e il registro lo dice invece di tacere.
  Sparisce anche il file temporaneo su disco.
- **Il fotogramma d'anteprima usciva dalla sua scatola.** Un `max-height: 100%`
  su un elemento di griglia con riga automatica si misura su un'altezza
  indefinita, cioe' non vincola niente: un video 1920×1080 traboccava da un
  riquadro di 1316×657. Il riquadro ora centra con flex, che un'altezza
  definita ce l'ha.
- **I fotogrammi non scorrevano durante la riproduzione.** Ogni cambio di
  posizione faceva avanzare il numero di richiesta, e al ritorno l'immagine
  veniva scartata perche' quel numero non era piu' l'ultimo: con l'orologio che
  cambia sessanta volte al secondo, veniva scartata *ogni* immagine e
  l'anteprima restava sul fotogramma in cui si era premuto play.
- **In Modifica il trasporto finiva sotto il bordo della finestra.** Alla
  colonna di sinistra mancava `min-height: 0`: senza, un elemento di griglia non
  puo' rimpicciolirsi sotto il proprio contenuto, e una tela alta 1920 px la
  faceva crescere finche' i comandi di riproduzione uscivano dallo schermo.
- Il cursore parte dalla prima parola invece che da `00:00`, dove quasi nessun
  file ha gia' qualcosa da mostrare: l'anteprima sembrava vuota appena aperta.
- Gli errori della webview — eccezioni, promesse rifiutate, fallimenti
  dell'elemento audio, fotogrammi non disegnati — finiscono nel log del motore.
  Senza, un difetto dentro la finestra si manifesta come un riquadro nero e
  nient'altro.
- La conversione colore YUV→RGB segue la convenzione dei lettori quando il file
  non dichiara lo spazio colore (BT.601 sotto i 576 punti di altezza, BT.709
  sopra): prima si usava sempre BT.709, con una deriva visibile sui file non
  etichettati.
- `verba rendi` su un file audio fallisce in un secondo invece che dopo l'intera
  trascrizione.
- Una fase annullata non si dichiara piu' conclusa: un segno di spunta subito
  dopo un «annullata» diceva il contrario di quello che era successo.
