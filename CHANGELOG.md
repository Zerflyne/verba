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

### Cambiato
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
- La conversione colore YUV→RGB segue la convenzione dei lettori quando il file
  non dichiara lo spazio colore (BT.601 sotto i 576 punti di altezza, BT.709
  sopra): prima si usava sempre BT.709, con una deriva visibile sui file non
  etichettati.
- `verba rendi` su un file audio fallisce in un secondo invece che dopo l'intera
  trascrizione.
- Una fase annullata non si dichiara piu' conclusa: un segno di spunta subito
  dopo un «annullata» diceva il contrario di quello che era successo.
