# I termini noti

Whisper accetta un *initial prompt*: un pezzo di testo che legge prima di
trascrivere e che gli suggerisce di che cosa si parla. Non e' un dizionario e
non e' una regola — e' un contesto. Un nome proprio che compare nel prompt ha
molte piu' probabilita' di essere scritto giusto; un nome che non c'e' verra'
reso con la parola comune che gli somiglia di piu'.

E' il modo migliore per far scrivere *Anthropic* invece di *antropico*, e
*Costantini* invece di *Costantino*.

## Il file

Un CSV, una colonna, un termine per riga.

```csv
termine
Anthropic
Claude Opus
Verba
cosmic-text
whisper.cpp
Costantini
ProRes 4444
```

L'intestazione e' facoltativa: Verba se ne accorge da sola. Come si accorge da
sola del delimitatore (virgola, punto e virgola, tabulazione) e delle
virgolette. Se il file ha piu' colonne viene letta la prima, e se ne serve
un'altra si indica per nome o per posizione:

```bash
verba trascrivi intervista.m4a --termini glossario.csv --termini-colonna nome
verba trascrivi intervista.m4a --termini glossario.csv --termini-colonna 2
```

Nell'applicazione il CSV si carica dalla scheda **Impostazioni**, e sotto
compaiono quanti termini sono stati letti e i primi cinque: se il file e' stato
interpretato male si vede subito, prima di spendere mezz'ora di trascrizione.

## Cosa metterci

- **Nomi propri** di persone, aziende, prodotti, luoghi.
- **Sigle** che vanno scritte maiuscole.
- **Termini tecnici** e parole straniere che nel contesto ricorrono.
- **Grafie particolari** che vuoi conservare: `whisper.cpp`, non `Whisper CPP`.

Cosa **non** metterci:

- Parole comuni. Non aiutano, e occupano posto.
- Frasi intere. Il prompt orienta lo stile, non detta il testo.
- Elenchi di centinaia di voci: vedi sotto.

## Il limite

Whisper accetta circa **224 token** di contesto — grosso modo 700 caratteri di
italiano. Oltre quel punto i termini in eccesso vengono scartati, **a termine
intero**: non ti ritrovi mai mezza parola nel prompt.

Se hai un glossario grosso, mettici in cima i termini che ricorrono davvero in
*questo* file. Un elenco di trecento voci di cui ne compaiono quattro e' peggio
di un elenco di quattro: le altre duecentonovantasei diluiscono il contesto.

Per vedere cosa verrebbe usato senza trascrivere niente:

```bash
verba trascrivi intervista.m4a --termini glossario.csv --solo-prompt
```

```
Intervista tecnica in italiano. Termini ricorrenti: Anthropic, Claude Opus,
Verba, cosmic-text, whisper.cpp, Costantini, ProRes 4444.
```

Il limite si sposta con `--prompt-max-caratteri`, ma spostarlo oltre il
contesto del modello non serve: i token in piu' vengono troncati da whisper.cpp
e basta.

## Il preambolo

La riga che apre il prompt e' modificabile:

```bash
verba trascrivi lezione.mp3 --termini glossario.csv \
    --termini-preambolo "Lezione universitaria di diritto privato in italiano."
```

Vale la pena cambiarla quando la registrazione ha un registro particolare —
una lezione, una riunione tecnica, un'intervista informale. Whisper adegua
punteggiatura e maiuscole a quello che si aspetta di sentire.

Un prompt libero, senza CSV, si passa con `--prompt`:

```bash
verba trascrivi vocale.opus --prompt "Messaggio vocale, italiano parlato, frasi brevi."
```

I due si sommano: `--prompt` va davanti, i termini del CSV dopo.
