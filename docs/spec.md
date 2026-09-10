# Verba — sottotitoli automatici in locale

Specifica per la costruzione dell'applicazione. Il nome è provvisorio: cambialo dove
compare se ne trovi uno migliore, ma cambialo ovunque.

**Cosa fa**: prende un file audio o video, lo trascrive con timestamp parola per
parola, e ne esporta i sottotitoli — come file di sottotitoli, come video con i
sottotitoli impressi, o come overlay su sfondo trasparente da montare altrove.

**Cosa la distingue**: gira interamente in locale. Nessun file lasciato su un
server, nessun abbonamento, nessun limite di minuti. È il motivo per cui esiste, e
va detto nella prima riga del README.

I riferimenti visivi sono due immagini allegate a questa spec:
**`verba-1-carica.png`** (sezione Carica, stato completato) e
**`verba-2-modifica.png`** (sezione Modifica). Proporzioni, gerarchia, etichette e
palette vanno rispettate; i testi mostrati sono di esempio.

---

## 1. Le due modalità

L'applicazione si comporta in modo diverso a seconda di cosa viene caricato, e
questa distinzione va resa visibile nell'interfaccia (il badge accanto al nome del
file nel mockup).

### Modalità audio

Input: `.mp3`, `.wav`, `.m4a`, `.flac`, `.ogg`, `.opus`

L'anteprima video non c'è: al suo posto compare la forma d'onda a tutta larghezza
con il testo trascritto sotto, scorrevole. Le schede **Stile** e **Posizione** sono
disabilitate — non c'è niente da disegnare.

Export disponibili: `.srt`, `.vtt`, `.json` (parola per parola), `.txt` (solo testo).

### Modalità video

Input: `.mp4`, `.mov`, `.mkv`, `.webm`, `.avi`

L'audio viene estratto e trascritto; il video serve da anteprima e da base per
l'export. Tutte le schede sono attive.

Export disponibili: tutti quelli della modalità audio, più:

- **video sottotitolato** — i sottotitoli impressi sul video di partenza
- **overlay trasparente** — solo i sottotitoli su sfondo trasparente, stessa
  durata e risoluzione del sorgente, da sovrapporre in un programma di montaggio

L'overlay trasparente è la modalità che serve a chi monta; il video sottotitolato è
quella che serve a chi non monta. **Il default è il video sottotitolato**, perché è
quello che si aspetta chi apre l'app per la prima volta.

---

## 2. Perimetro della prima versione

Questa spec descrive due versioni. **Costruisci solo la 0.1.** La 0.2 è qui perché
alcune scelte di architettura della 0.1 devono lasciarle spazio, non perché vada
implementata adesso.

### v0.1 — quello che si costruisce ora

- Caricamento audio o video, anche per trascinamento
- Trascrizione con timestamp parola per parola
- Anteprima con i sottotitoli sovrapposti, navigabile
- Tutti i controlli di stile, posizione, tempi e modello descritti sotto
- Preset salvabili e ricaricabili
- Tutti gli export elencati sopra
- Interfaccia a riga di comando con le stesse funzioni

### v0.2 — quello a cui va lasciato spazio

- Correzione del testo: cliccare una parola, modificarla, dividerla, unirla,
  cancellarla
- Correzione dei tempi trascinando i limiti sulla forma d'onda
- Dizionario di sostituzioni automatiche applicato dopo la trascrizione
- Ritrascrizione di un solo intervallo invece dell'intero file

**Conseguenza sull'architettura della 0.1**: la sequenza di parole va tenuta come
struttura dati mutabile e separata dal risultato grezzo del modello, con un
identificativo stabile per parola. Se la 0.1 tratta la trascrizione come un blocco
immutabile, la 0.2 impone una riscrittura.

La striscia di parole in basso nel mockup esiste già nella 0.1, in sola lettura:
mostra la finestra di parole intorno alla posizione corrente e evidenzia quella
attiva. Nella 0.2 diventa modificabile.

---

## 3. Architettura

Tre crate in un workspace Cargo.

```
verba/
├─ crates/
│  ├─ verba-core/     libreria: tutta la logica, nessuna dipendenza da UI
│  ├─ verba-cli/      binario a riga di comando
│  └─ verba-app/      applicazione Tauri v2
├─ ui/                frontend dell'app (React + Vite)
├─ assets/
└─ README.md
```

`verba-core` non deve sapere che esistono né la CLI né Tauri. Espone la pipeline
come funzioni pure più un canale di eventi di avanzamento.

### Moduli di verba-core

| Modulo | Responsabilità |
|---|---|
| `media` | apertura di qualsiasi formato, estrazione audio, downmix mono, ricampionamento a 16 kHz, normalizzazione — tutto in RAM, nessun file temporaneo |
| `asr` | Whisper large-v3 in ONNX, segmentazione con pyannote, initial prompt |
| `align` | allineamento forzato wav2vec2 italiano, timestamp per parola |
| `clean` | normalizzazione della sequenza (vedi sotto) |
| `layout` | composizione con cosmic-text: spezzatura in chunk, righe, geometria della parola attiva |
| `render` | disegno dei fotogrammi e codifica con libavcodec |
| `project` | stato del progetto, preset, serializzazione |

### La funzione di pulizia

Va tenuta separata e testata, perché è quella su cui si regge tutto il resto. Dopo
l'allineamento la sequenza di parole viene normalizzata così:

- scarta le parole vuote
- riempie i timestamp mancanti — l'allineatore non aggancia numeri e simboli —
  interpolando fra i vicini con tempo noto, e spartendo equamente l'intervallo fra
  più parole consecutive senza tempo
- impone monotonia: nessuna parola inizia prima che finisca la precedente
- impone una durata minima per parola
- tronca alla durata effettiva dell'audio

A valle di questa funzione il resto del programma può assumere una sequenza
ordinata, senza buchi e senza sovrapposizioni. Scrivi i test su questa funzione
prima di scrivere il resto.

---

## 4. Tre decisioni che evitano guai

Queste vanno prese così e non altrimenti, perché ognuna corrisponde a un modo
tipico di far fallire la distribuzione dell'applicazione.

### I modelli si scaricano, non si impacchettano

Whisper large-v3 in ONNX supera abbondantemente il gigabyte. Un `.deb` o un `.exe`
di quella dimensione non è distribuibile.

Al primo avvio l'applicazione mostra una schermata di preparazione, scarica i
modelli con una barra di avanzamento e li mette nella cartella dati dell'utente
(`~/.local/share/verba/models` su Linux, `%LOCALAPPDATA%\verba\models` su Windows).
Verifica l'hash di ogni file scaricato. Se il download si interrompe, riprende;
non ricomincia.

Offri anche la scelta della dimensione del modello: `large-v3` come default,
`medium` e `small` per chi ha poca VRAM o poca pazienza. La differenza di qualità va
spiegata in una riga sotto il selettore, non lasciata indovinare.

### La GPU è opzionale, la CPU è il default

Se l'applicazione richiede CUDA per partire, metà delle persone che la scaricano
non riescono ad aprirla e tu passi il tempo a rispondere a issue di installazione.

Il comportamento corretto: all'avvio prova a inizializzare l'execution provider
CUDA; se fallisce per qualsiasi motivo, ricadi su CPU **senza errori bloccanti** e
scrivi quale provider è attivo nella barra di stato (nel mockup:
"GPU NVIDIA (CUDA)"). Un selettore nella scheda Modello permette di forzare la CPU.

Documenta nel README che senza GPU funziona lo stesso, solo più lentamente, con un
ordine di grandezza indicativo.

### Whisper esce dalla memoria prima che entri l'allineatore

I due modelli insieme non stanno in 8 GB di VRAM. La sequenza deve essere:
trascrivi, scarica Whisper dalla GPU, verifica che la memoria sia stata liberata,
carica l'allineatore. Se questa sequenza non è esplicita nel codice, su una scheda
da 8 GB l'applicazione fallisce con un errore di memoria che sembra casuale.

---

## 5. Interfaccia

Riferimenti di contenuto: `verba-1-carica.png` e `verba-2-modifica.png`.

### Direzione visiva

Minimalismo moderno: molto spazio vuoto, poche linee, nessuna decorazione. La base è
fatta di **grigi neutri**, senza dominante di colore — non grigi virati al viola. Il
violetto è un dettaglio d'accento, non il colore dell'interfaccia: se togliessi il
violetto, quello che resta deve sembrare un'applicazione finita in bianco e nero.

Il violetto marca **una cosa alla volta**: la voce di navigazione attiva, il valore
che stai modificando, la parola evidenziata, il pulsante primario di quella
schermata. Se compare in tre punti insieme smette di guidare l'occhio e diventa
decorazione.

Niente gradienti sui pannelli, niente ombre sui singoli controlli, niente bagliori,
niente icone colorate. I bordi sono capelli da 1 px. I raggi degli angoli stanno tra
8 e 14 px e vanno usati in modo coerente: stesso raggio per elementi dello stesso
livello gerarchico.

```css
--bg:      #121212;   --surface: #171717;   --surface2: #202020;
--border:  #2A2A2A;   --text:    #EDEDED;   --dim:      #8C8C8C;
--accent:  #A78BFA;   --strong:  #8B5CF6;   --lilac:    #C4B5FD;
--ok:      #6FCF97;   --warn:    #E0B25C;   --err:      #E07A7A;
```

Il violetto compare in cinque posti e basta: la barra della voce di navigazione
attiva, il riempimento dei cursori, il pulsante primario, la parola evidenziata
nell'anteprima e il bordo delle parole segnalate. Ovunque altro, grigio.

**Meno cornici possibile.** I gruppi si separano con lo spazio e con intestazioni
testuali, non con riquadri: nell'interfaccia c'è un solo bordo verticale, quello che
divide la barra laterale, più il contorno dell'anteprima. Il pannello dei controlli
e la barra di stato non hanno fondo proprio.

Tema chiaro: non nella 0.1. Meglio un tema scuro fatto bene che due fatti a metà.

### Finestra

**1600×980, dimensione fissa.** Non ridimensionabile, non a schermo intero: il
layout è disegnato per quelle proporzioni, e una finestra fissa garantisce che
l'applicazione appaia identica in ogni ripresa e su ogni schermo. Su un monitor 4K
occupa un quarto dello schermo e in montaggio la si scala — è anche molto più
leggera da riprodurre in anteprima di un video 4K a pieno schermo.

### Impianto

```
┌──────────┬──────────────────────────────────────────────────┐
│          │                                                  │
│ barra    │            area della sezione attiva             │
│ laterale │                                                  │
│  220px   │                                                  │
│          │                                                  │
├──────────┴──────────────────────────────────────────────────┤
│  barra di stato — 38px                                      │
└─────────────────────────────────────────────────────────────┘
```

### Barra laterale

220 px fissi, fondo `--surface`, separata dal contenuto da un bordo capello. Nello
spirito della navigazione laterale del sito di Anthropic: sobria, testuale, senza
riquadri intorno alle voci.

Struttura dall'alto:

- **Identità**: quadratino violetto con la "V" e il nome accanto, 24 px di padding
- **Le quattro voci**, ognuna con una piccola icona a tratto 1,5 px e l'etichetta:

| # | Voce | Cosa contiene |
|---|---|---|
| 1 | Carica | caricamento del file, elaborazione, anteprima del risultato |
| 2 | Modifica | tutta la personalizzazione dei sottotitoli |
| 3 | Esporta | scelta del formato e generazione |
| 4 | Impostazioni | modello, dispositivo, cartelle, aggiornamenti |

- **In fondo**: versione dell'applicazione e collegamento al repository, in
  `--dim`, 11 px

Stati delle voci:

- **a riposo**: testo `--dim`, icona `--dim`
- **sotto il puntatore**: testo `--text`, fondo `--surface2`
- **attiva**: testo `--text`, icona `--accent`, fondo `--surface2`, e una barra
  verticale di 3 px in `--accent` a filo del bordo sinistro
- **non ancora disponibile**: testo al 40% di opacità, non cliccabile

L'ultimo stato è importante: **Modifica ed Esporta restano disabilitate finché non
c'è una trascrizione**. È il modo più semplice per far capire l'ordine delle cose
senza spiegarlo. Al termine dell'elaborazione si accendono, con una transizione di
opacità di 200 ms.

La navigazione è libera in avanti e indietro una volta sbloccata: non è una
procedura guidata, è un'applicazione con tre stanze.

---

### Sezione 1 — Carica

Ha tre stati distinti che occupano la stessa area.

**Stato vuoto.** Al centro, un'area di trascinamento: rettangolo con bordo
tratteggiato `--border`, raggio 14 px, che occupa circa il 60% della larghezza e
metà dell'altezza. Dentro, un'icona a tratto, la riga *Trascina qui un file audio o
video*, sotto in `--dim` *oppure* e un pulsante secondario *Scegli un file*. Sotto
l'area, una riga sola con i formati accettati.

Quando un file viene trascinato sopra, il bordo passa a `--accent` continuo e il
fondo a `--accent-soft`. Nessun'altra animazione.

**Stato in elaborazione.** L'area di trascinamento è sostituita da una scheda
centrata che mostra il nome del file, le sue caratteristiche, e l'avanzamento per
fasi:

```
  Preparazione dell'audio          ✓  2s
  Trascrizione                     ▸  1m 04s
  Allineamento delle parole        ·
  Pulizia dei tempi                ·
```

Le fasi completate hanno un segno di spunta in `--ok` e il tempo impiegato; quella
corrente ha un indicatore in `--accent` e il tempo che scorre; quelle future sono in
`--dim`. Una barra di avanzamento complessiva sotto.

Mostrare le fasi con i loro tempi invece di una sola barra indefinita ha due
vantaggi: chi aspetta capisce a che punto è, e chi guarda il video capisce come è
fatta la pipeline senza che tu debba spiegarlo.

Un pulsante *Annulla* interrompe davvero l'elaborazione.

**Stato completato.** L'anteprima del video con i sottotitoli sovrapposti, con
sotto il trasporto e la striscia di parole. È la schermata del mockup, meno il
pannello destro.

- **Anteprima**: il fotogramma corrente con i sottotitoli disegnati sopra, così come
  usciranno. Due guide attivabili da un pulsante discreto in un angolo: il rettangolo
  dell'area sicura e la linea di base dei sottotitoli.
- **Trasporto**: riproduzione, posizione e durata in monospaziato, cursore di
  scorrimento; sotto, la forma d'onda dell'intero file con colorata in `--accent` la
  porzione del chunk attualmente a schermo.
- **Striscia di parole**: la finestra di parole intorno alla posizione corrente, in
  sola lettura nella 0.1. Tre stati — normale (`--surface2` / `--dim`), attiva
  (`--strong` / bianco), segnalata (bordo tratteggiato `--accent`, testo `--lilac`,
  per confidenza sotto la soglia).

Le parole segnalate sono utili già nella 0.1 anche se non si possono ancora
correggere: dicono dove guardare.

In alto a destra dell'area, un pulsante secondario *Cambia file*.

**L'anteprima deve essere generata dallo stesso codice dell'export.** Non
reimplementare il layout in JavaScript per farlo andare più veloce: il risultato
sarà che anteprima ed export divergono su qualche dettaglio e passerai giorni a
cercare il perché. Il core espone un comando *disegna il fotogramma al tempo t* che
restituisce un buffer RGBA, e il frontend lo mostra. Se lo scorrimento risulta
lento, si limita la frequenza di aggiornamento durante il trascinamento e si
ridisegna a risoluzione piena al rilascio.

---

### Sezione 2 — Modifica

Due colonne: a sinistra l'anteprima, sempre visibile e sempre aggiornata; a destra i
controlli in un pannello da 412 px.

L'anteprima qui è più piccola che nella sezione 1 ma resta il soggetto principale.
Sotto ha solo il cursore di scorrimento e il tempo, senza forma d'onda: serve a
verificare l'effetto delle modifiche in punti diversi del filmato.

I controlli sono raggruppati sotto intestazioni testuali in `--dim` da 12 px, senza
riquadri. **Nessuna modifica in questa sezione rilancia il modello**: tutto si
applica sull'anteprima entro un fotogramma.

**Testo**

| Controllo | Tipo | Default |
|---|---|---|
| Carattere | font di sistema + quelli in `assets/fonts` | Inter |
| Peso | pesi disponibili nel font scelto | 700 |
| Corpo | cursore 24–160 px | 64 px |
| Maiuscole | interruttore | spento |

Il corpo è in pixel **riferiti all'altezza del video sorgente**, non
dell'anteprima: 64 px su un 4K e 64 px su un 1080p danno risultati molto diversi.
Accanto al cursore, mostra la percentuale rispetto all'altezza del fotogramma — è il
numero che conta davvero.

**Colori**

| Controllo | Default |
|---|---|
| Testo | `#FFFFFF` |
| Parola attiva | `#8B5CF6` |
| Testo attivo | `#FFFFFF` |
| Contorno | 3 px |
| Ombra | acceso |

Ogni colore è una riga con il codice esadecimale modificabile e un quadratino
campione cliccabile che apre il selettore.

**Evidenziazione**

| Controllo | Tipo | Default |
|---|---|---|
| Forma | rettangolo / sottolineatura / solo colore | rettangolo |
| Raggio angoli | cursore 0–40 px | 10 px |

**Posizione**

| Controllo | Tipo | Default |
|---|---|---|
| Formato | 16:9 / 9:16 / dal sorgente | dal sorgente |
| Posizione verticale | cursore 0–100% | 82% |
| Posizione orizzontale | cursore 0–100% | 50% |
| Larghezza massima | cursore 40–100% | 80% |
| Righe massime | 1 / 2 / 3 | 2 |
| Allineamento | sinistra / centro / destra | centro |
| Margine dai bordi | cursore | 5% |

La spezzatura in chunk è automatica: le parole si accumulano finché entrano nella
larghezza massima e nel numero di righe consentito. **Non esporre un controllo
"parole per chunk"**: è il tipo di parametro che sembra utile e produce risultati
peggiori di quelli automatici.

**Tempi**

| Controllo | Tipo | Default |
|---|---|---|
| Anticipo evidenziazione | cursore 0–200 ms | 60 ms |
| Tetto alla pausa | cursore 0–2000 ms | 600 ms |
| Coda dopo l'ultima parola | cursore 0–2000 ms | 400 ms |
| Durata minima parola | cursore 20–300 ms | 80 ms |

Ognuno con una riga di spiegazione sotto, in `--dim` da 11 px. *Tetto alla pausa*
non significa niente a chi non ha scritto il codice: la riga dice che impedisce
all'evidenziazione di restare accesa per tutta la durata di un silenzio.

**Preset**

In fondo al pannello, salvataggio e caricamento in JSON. Un preset contiene testo,
colori, evidenziazione, posizione e tempi; **non** contiene le impostazioni del
modello né riferimenti a file.

Tre preset di serie, che sono anche la vetrina di cosa sa fare l'applicazione:
`Verticale`, `Orizzontale`, `Sobrio` — quest'ultimo senza evidenziazione, solo testo
bianco con contorno.

---

### Sezione 3 — Esporta

Una sola colonna centrata, larga circa 700 px, molto arieggiata. Niente finestre
modali: l'export è una sezione, non una finestra che si apre.

**Scelta del formato.** Un elenco di righe selezionabili, una per formato, ognuna con
nome, estensione e una riga di descrizione in `--dim`. La riga selezionata ha il
fondo `--surface2` e la barra `--accent` a sinistra, come le voci di navigazione.

I formati disponibili dipendono dalla modalità, e quelli non applicabili non vanno
mostrati disabilitati: vanno nascosti. In modalità audio l'elenco contiene solo i
quattro formati testuali.

**Destinazione.** Percorso proposto — stessa cartella del sorgente, stesso nome con
un suffisso — e un pulsante per cambiarlo.

**Opzioni del formato scelto**, se ce ne sono: qualità per l'H.264, profilo per il
ProRes. Poche, e solo quelle che cambiano davvero il risultato.

**Pulsante primario** *Esporta*, unico elemento violetto pieno della schermata.

**Durante l'export** la schermata si sostituisce con l'avanzamento: barra, percentuale,
tempo residuo stimato, fotogrammi al secondo, e un pulsante *Annulla* che interrompe
davvero la codifica e cancella il file parziale.

**Al termine**: un segno di spunta in `--ok`, il percorso del file prodotto, e due
pulsanti — *Apri cartella* e *Esporta di nuovo*, che riporta alla scelta del formato
mantenendo tutto il resto.

---

### Sezione 4 — Impostazioni

Una colonna centrata, gruppi separati da intestazioni testuali.

**Modello**

| Controllo | Tipo | Default |
|---|---|---|
| Dimensione | large-v3 / medium / small | large-v3 |
| Lingua | elenco + rilevamento automatico | italiano |
| Dispositivo | automatico / GPU / CPU | automatico |
| Termini noti | caricamento di un CSV | vuoto |
| Soglia di segnalazione | cursore 0–1 | 0,5 |

Sotto la dimensione, una riga che spiega il compromesso in termini concreti: spazio
occupato, memoria richiesta, velocità indicativa. Non lasciare che l'utente
indovini.

I termini noti vengono passati come initial prompt. Dopo il caricamento, mostra
quanti termini sono stati letti e i primi cinque, così si vede che il file è stato
interpretato.

**Modificare qualcosa in questo gruppo è l'unica cosa che richiede di rilanciare il
modello.** Quando ci sono modifiche pendenti compare in fondo una barra fissa con la
riga *Le modifiche richiedono una nuova trascrizione* e il pulsante *Ritrascrivi*,
che riporta alla sezione 1 in stato di elaborazione.

**Modelli scaricati.** Elenco dei modelli presenti con la loro dimensione su disco e
un pulsante per rimuoverli. Un pulsante per scaricare quelli mancanti.

**Cartelle.** Percorso dei modelli e cartella di export predefinita, entrambi
modificabili.

**Informazioni.** Versione, collegamento al repository, licenza, e la riga che dice
quale provider di calcolo è attivo in questo momento.

---

### Barra di stato

38 px, sempre visibile, in fondo alla finestra su tutta la larghezza. Un pallino di
stato e una riga di testo in `--dim` da 12 px:

- a riposo: nome del file e sue caratteristiche
- dopo una trascrizione: pallino `--ok` e riepilogo — *Trascrizione completata —
  2.184 parole in 1m 18s · GPU NVIDIA (CUDA) · large-v3*
- durante un'operazione: pallino `--accent` e la fase corrente
- in errore: pallino `--err` e il messaggio

Gli errori vivono qui, non in finestre modali, tranne quelli fatali.
## 6. Formati di export

Tabella di riferimento per la sezione 3 dell'interfaccia. L'applicazione ricorda
l'ultima scelta.

| Formato | Estensione | Codec | Note |
|---|---|---|---|
| Video sottotitolato | `.mp4` | H.264, CRF 18, yuv420p | default in modalità video |
| Video sottotitolato senza perdita | `.mov` | ProRes 422 | per chi rimonta |
| Overlay trasparente | `.mov` | ProRes 4444, `yuva444p10le` | canale alpha |
| Overlay trasparente compatto | `.webm` | VP9 con alpha | molto più leggero |
| Sottotitoli | `.srt` | — | un chunk per blocco |
| Sottotitoli | `.vtt` | — | |
| Parola per parola | `.json` | — | testo, inizio, fine, confidenza |
| Solo testo | `.txt` | — | |

Il nome proposto è quello del sorgente con un suffisso (`_sub`, `_overlay`), nella
stessa cartella del sorgente.

In modalità audio si mostrano solo i quattro formati testuali: gli altri non vanno
disabilitati, vanno nascosti.

---

## 7. Riga di comando

Stesse funzioni, per chi automatizza. Almeno:

```
verba trascrivi input.mp3 --out sottotitoli.srt --lingua it --termini glossario.csv
verba rendi input.mp4 --out video_sub.mp4 --preset orizzontale.json
verba overlay input.mp4 --out overlay.mov --preset verticale.json
```

`--json` su qualsiasi comando fa scrivere l'avanzamento in JSON su stderr, così è
integrabile in altri script.

---

## 8. Errori

Ogni errore dice cosa è successo e cosa fare, nella barra di stato in
`--err`, senza finestre modali tranne che per gli errori fatali.

I casi da gestire esplicitamente, perché sono quelli che capitano davvero:

| Situazione | Messaggio |
|---|---|
| File senza traccia audio | Il file non contiene audio. Serve un file con una traccia audio. |
| Formato non riconosciuto | Formato non supportato. Formati accettati: … |
| Modelli non ancora scaricati | Preparazione al primo avvio, con barra di avanzamento |
| VRAM insufficiente | Memoria GPU insufficiente per large-v3. Passa a medium o forza la CPU dalla scheda Modello. |
| Download interrotto | Download interrotto. Riprendi. |
| Nessun parlato rilevato | Nessun parlato riconosciuto nell'audio. |
| Font mancante | Il carattere scelto non è disponibile. Ne è stato usato un altro. |

---

## 9. Distribuzione

Tre pacchetti generati dal CI su tag:

- `.deb` per Debian e Ubuntu
- `.AppImage` per le altre distribuzioni
- `.exe` installer per Windows

Due cose da mettere in conto:

**L'eseguibile Windows non firmato fa comparire l'avviso di SmartScreen.** Un
certificato costa qualche centinaio di euro l'anno e non ha senso adesso: scrivi nel
README come procedere oltre l'avviso, con uno screenshot. Se non lo scrivi, riceverai
la stessa domanda decine di volte.

**Il primo avvio scarica più di un gigabyte.** Dillo nel README prima delle
istruzioni di installazione, non dopo.

---

## 10. Repository

Da preparare **prima di registrare il video**, così a fine video si mostra la pagina
vera e non un segnaposto.

```
README.md          cosa fa, screenshot, installazione, primo avvio, uso, limiti noti
LICENSE            MIT
CHANGELOG.md
CONTRIBUTING.md    breve
.github/workflows/ build e release sui tag
assets/            screenshot, GIF di dimostrazione, preset di esempio
docs/
  termini.md       come si scrive il CSV dei termini noti, con esempio
  tempi.md         cosa fanno i quattro parametri temporali, con confronti visivi
```

Il README apre con uno screenshot dell'applicazione e una frase sola su cosa fa e
dove gira. Poi installazione, poi uso. La sezione dei limiti noti sta in alto, non
in fondo: dire subito che senza GPU è lento e che l'italiano funziona meglio
dell'inglese fa risparmiare tempo a tutti e ti fa sembrare uno che sa cosa ha
costruito.

Argomenti GitHub: `subtitles`, `whisper`, `rust`, `tauri`, `speech-to-text`,
`video-editing`, `offline`, `local-first`.

---

## 11. Ordine di costruzione

Costruisci in quest'ordine e verifica ogni passo prima del successivo. Ogni riga
produce qualcosa di osservabile.

1. `verba-core`: media + asr + align + clean, con la CLI che esporta un SRT
2. Test della funzione di pulizia sui casi limite: numeri, simboli, silenzi lunghi,
   parole senza tempo in coda
3. `layout` + `render`: dalla CLI, un overlay ProRes 4444 da un file video
4. Export del video sottotitolato
5. Guscio Tauri: caricamento file, anteprima statica, barra di stato
6. Pannello destro con applicazione istantanea sull'anteprima
7. Riproduzione e striscia di parole
8. Preset
9. Finestra di export con avanzamento e annullamento
10. Primo avvio con scaricamento dei modelli
11. Gestione degli errori dell'elenco al punto 8
12. CI e pacchetti

I punti da 1 a 4 sono l'applicazione vera; dal 5 in poi è l'involucro. Se il tempo
finisce, un progetto fermo al punto 4 con una buona CLI è comunque pubblicabile.
