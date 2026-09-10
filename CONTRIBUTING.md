# Contribuire a Verba

Grazie. Poche cose, tutte brevi.

## Prima di aprire una issue

Se e' un problema di installazione o di avvio, allega l'uscita di:

```bash
verba modelli
verba trascrivi ilfile.mp3 --solo-audio -v
```

La prima dice se i modelli ci sono, la seconda se la build regge senza caricare
niente. Nove problemi su dieci si risolvono guardando quelle due.

## Prima di aprire una pull request

```bash
cargo test
cargo clippy --all-targets
npm run build --prefix ui
```

Tutti e tre devono essere puliti. `clippy` senza avvisi, non «con qualche
avviso noto».

## Convenzioni

**Il codice e' in italiano.** Nomi, commenti, messaggi di log, opzioni, test.
Non e' una preferenza estetica: e' la lingua in cui e' scritta la specifica e
in cui si parla del progetto, e mescolarne due rende ogni nome una decisione.
Restano in inglese solo i nomi che vengono da fuori (`Whisper`, `ONNX`,
`ProRes`) e le API delle librerie.

**I commenti spiegano il perche', non il cosa.** Che una funzione sommi due
numeri si vede; perche' li sommi *cosi'* no. Un commento che ripete la riga
sotto e' rumore che invecchia.

**Ogni cosa che si puo' sbagliare ha un test.** In particolare tutto quello che
tocca i tempi delle parole: `pulizia.rs` e' la funzione su cui si regge il
resto, e i suoi casi limite (numeri, simboli, silenzi lunghi, parole senza
tempo in coda) sono gia' scritti.

**I messaggi di errore dicono cosa fare.** «formato non supportato» non aiuta
nessuno; «estensione non riconosciuta: il formato si sceglie con .srt, .vtt,
.json o .txt» si'.

## Struttura

- `verba-core` non deve sapere che esistono la riga di comando o Tauri. Se una
  funzione ha bisogno di sapere chi la chiama, sta nel posto sbagliato.
- **L'anteprima esce dallo stesso codice dell'export.** Non reimplementare
  l'impaginazione in JavaScript per farla andare piu' veloce: il risultato
  sarebbe che le due immagini divergono su qualche dettaglio, e trovare il
  perche' costa giorni.
- **Whisper esce dalla memoria prima che entri l'allineatore.** La sequenza sta
  in `pipeline.rs` ed e' esplicita. Se smettesse di esserlo, su una scheda da
  8 GB l'applicazione fallirebbe con un errore che sembra casuale.
