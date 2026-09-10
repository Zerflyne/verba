//! Avanzamento e annullamento.
//!
//! Il motore non stampa e non disegna: emette eventi. Chi lo usa decide se
//! diventano righe in un terminale, una barra in una finestra o JSON su stderr.
//! E' cio' che permette a `verba-core` di non sapere che esistono ne' la riga
//! di comando ne' l'applicazione.
//!
//! Due cose viaggiano insieme, perche' servono negli stessi punti: la
//! comunicazione di quanto manca, e la possibilita' di fermarsi. Una barra di
//! avanzamento senza un pulsante *Annulla* che funziona davvero e' peggio che
//! inutile: promette un controllo che non c'e'.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Le fasi della pipeline, nell'ordine in cui si svolgono.
///
/// Sono quelle che l'interfaccia elenca durante l'elaborazione: mostrarle una
/// per una, con il tempo che ciascuna ha richiesto, dice a chi aspetta a che
/// punto e' — cosa che una sola barra indefinita non fa.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fase {
    Preparazione,
    Segmentazione,
    Trascrizione,
    Allineamento,
    Pulizia,
    Impaginazione,
    Codifica,
}

impl Fase {
    /// L'ordine di svolgimento. Non tutte le fasi vengono sempre eseguite.
    pub const TUTTE: [Fase; 7] = [
        Fase::Preparazione,
        Fase::Segmentazione,
        Fase::Trascrizione,
        Fase::Allineamento,
        Fase::Pulizia,
        Fase::Impaginazione,
        Fase::Codifica,
    ];

    /// Come la fase va scritta all'utente.
    pub fn etichetta(self) -> &'static str {
        match self {
            Fase::Preparazione => "Preparazione dell'audio",
            Fase::Segmentazione => "Rilevamento del parlato",
            Fase::Trascrizione => "Trascrizione",
            Fase::Allineamento => "Allineamento delle parole",
            Fase::Pulizia => "Pulizia dei tempi",
            Fase::Impaginazione => "Impaginazione",
            Fase::Codifica => "Codifica del video",
        }
    }

    /// Nome breve, per i log e per il JSON.
    pub fn nome(self) -> &'static str {
        match self {
            Fase::Preparazione => "preparazione",
            Fase::Segmentazione => "segmentazione",
            Fase::Trascrizione => "trascrizione",
            Fase::Allineamento => "allineamento",
            Fase::Pulizia => "pulizia",
            Fase::Impaginazione => "impaginazione",
            Fase::Codifica => "codifica",
        }
    }
}

impl std::fmt::Display for Fase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.nome())
    }
}

/// Cosa succede durante l'elaborazione.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "evento", rename_all = "snake_case")]
pub enum Evento {
    /// Una fase e' cominciata.
    Iniziata { fase: Fase },
    /// Avanzamento dentro una fase, in [0, 1]. Non tutte le fasi lo emettono:
    /// alcune sono atomiche e passano direttamente da `Iniziata` a `Conclusa`.
    Avanzamento { fase: Fase, frazione: f32 },
    /// Una fase e' finita, con il tempo che ha richiesto.
    Conclusa { fase: Fase, secondi: f64 },
    /// Qualcosa merita di essere detto ma non ferma l'elaborazione: un font
    /// mancante sostituito, un segmento allineato per ripartizione.
    Avviso { messaggio: String },
    /// L'elaborazione e' stata fermata su richiesta.
    Annullata,
}

/// Chi riceve gli eventi.
type Ascoltatore = dyn Fn(Evento) + Send + Sync;

/// Richiesta di fermarsi, condivisa fra chi elabora e chi guarda.
///
/// Si clona liberamente: l'interfaccia ne tiene una copia per il pulsante
/// *Annulla*, la pipeline ne tiene un'altra e la interroga fra un passo e
/// l'altro.
#[derive(Clone, Debug, Default)]
pub struct Interruttore(Arc<AtomicBool>);

impl Interruttore {
    pub fn nuovo() -> Self {
        Self::default()
    }

    /// Chiede alla pipeline di fermarsi appena puo'.
    pub fn annulla(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn annullato(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// L'errore che la pipeline restituisce quando e' stata fermata.
///
/// E' un tipo suo e non un messaggio qualsiasi perche' chi chiama deve poter
/// distinguere "l'utente ha premuto Annulla" da "qualcosa e' andato storto": il
/// primo caso non e' un errore da mostrare in rosso.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Annullato;

impl std::fmt::Display for Annullato {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("elaborazione annullata")
    }
}

impl std::error::Error for Annullato {}

/// Il canale di avanzamento che la pipeline riceve.
#[derive(Clone)]
pub struct Progresso {
    ascoltatore: Option<Arc<Ascoltatore>>,
    interruttore: Interruttore,
}

impl Default for Progresso {
    fn default() -> Self {
        Self::silenzioso()
    }
}

impl std::fmt::Debug for Progresso {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Progresso")
            .field("ascoltatore", &self.ascoltatore.is_some())
            .field("annullato", &self.interruttore.annullato())
            .finish()
    }
}

impl Progresso {
    /// Nessuno ascolta. E' il caso dei test e degli usi non interattivi.
    pub fn silenzioso() -> Self {
        Self { ascoltatore: None, interruttore: Interruttore::nuovo() }
    }

    /// Gli eventi vanno alla funzione data, dal thread che li emette.
    pub fn con(f: impl Fn(Evento) + Send + Sync + 'static) -> Self {
        Self { ascoltatore: Some(Arc::new(f)), interruttore: Interruttore::nuovo() }
    }

    /// Gli eventi vanno su un canale, da consumare altrove.
    pub fn canale() -> (Self, Receiver<Evento>) {
        let (tx, rx) = channel();
        let p = Self::con(move |e| {
            // Se il ricevitore e' sparito l'elaborazione continua lo stesso:
            // nessuno ascolta piu', ma non e' un motivo per fallire.
            let _ = tx.send(e);
        });
        (p, rx)
    }

    /// L'interruttore associato, da consegnare a chi puo' premere *Annulla*.
    pub fn interruttore(&self) -> Interruttore {
        self.interruttore.clone()
    }

    /// Vero se e' stato chiesto di fermarsi.
    pub fn annullato(&self) -> bool {
        self.interruttore.annullato()
    }

    /// Da chiamare fra un passo e l'altro nei cicli lunghi: se l'utente ha
    /// annullato, interrompe con [`Annullato`].
    pub fn verifica(&self) -> Result<(), Annullato> {
        if self.annullato() {
            self.emetti(Evento::Annullata);
            Err(Annullato)
        } else {
            Ok(())
        }
    }

    pub fn emetti(&self, evento: Evento) {
        if let Some(a) = &self.ascoltatore {
            a(evento);
        }
    }

    /// Annuncia l'inizio di una fase e restituisce il cronometro che, quando
    /// esce di scena, ne annuncia la fine con il tempo trascorso.
    ///
    /// Legare la chiusura al ciclo di vita di un valore evita il modo tipico di
    /// sbagliare qui: un ritorno anticipato che lascia la fase accesa per
    /// sempre nell'interfaccia.
    pub fn inizia(&self, fase: Fase) -> Cronometro<'_> {
        self.emetti(Evento::Iniziata { fase });
        Cronometro { progresso: self, fase, inizio: Instant::now() }
    }

    /// Avanzamento dentro la fase corrente, in [0, 1].
    pub fn passo(&self, fase: Fase, frazione: f32) {
        self.emetti(Evento::Avanzamento { fase, frazione: frazione.clamp(0.0, 1.0) });
    }

    pub fn avviso(&self, messaggio: impl Into<String>) {
        self.emetti(Evento::Avviso { messaggio: messaggio.into() });
    }
}

/// Misura la durata di una fase e ne annuncia la conclusione.
pub struct Cronometro<'a> {
    progresso: &'a Progresso,
    fase: Fase,
    inizio: Instant,
}

impl Cronometro<'_> {
    pub fn trascorso(&self) -> Duration {
        self.inizio.elapsed()
    }

    /// Avanzamento dentro questa fase, senza doverla ripetere.
    pub fn passo(&self, frazione: f32) {
        self.progresso.passo(self.fase, frazione);
    }
}

impl Drop for Cronometro<'_> {
    fn drop(&mut self) {
        self.progresso.emetti(Evento::Conclusa {
            fase: self.fase,
            secondi: self.inizio.elapsed().as_secs_f64(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raccogli() -> (Progresso, Arc<std::sync::Mutex<Vec<Evento>>>) {
        let visti = Arc::new(std::sync::Mutex::new(Vec::new()));
        let copia = visti.clone();
        (Progresso::con(move |e| copia.lock().unwrap().push(e)), visti)
    }

    #[test]
    fn una_fase_annuncia_inizio_e_fine() {
        let (p, visti) = raccogli();
        {
            let _c = p.inizia(Fase::Trascrizione);
        }
        let v = visti.lock().unwrap();
        assert!(matches!(v[0], Evento::Iniziata { fase: Fase::Trascrizione }));
        assert!(matches!(v[1], Evento::Conclusa { fase: Fase::Trascrizione, .. }));
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn la_fase_si_chiude_anche_uscendo_prima() {
        let (p, visti) = raccogli();

        fn fallisce(p: &Progresso) -> Result<(), &'static str> {
            let _c = p.inizia(Fase::Allineamento);
            Err("qualcosa e' andato storto")
        }

        assert!(fallisce(&p).is_err());
        let v = visti.lock().unwrap();
        assert!(
            matches!(v.last(), Some(Evento::Conclusa { fase: Fase::Allineamento, .. })),
            "un ritorno anticipato ha lasciato la fase accesa: {v:?}"
        );
    }

    #[test]
    fn l_avanzamento_resta_fra_zero_e_uno() {
        let (p, visti) = raccogli();
        p.passo(Fase::Codifica, -3.0);
        p.passo(Fase::Codifica, 42.0);
        let v = visti.lock().unwrap();
        let frazioni: Vec<f32> = v
            .iter()
            .filter_map(|e| match e {
                Evento::Avanzamento { frazione, .. } => Some(*frazione),
                _ => None,
            })
            .collect();
        assert_eq!(frazioni, vec![0.0, 1.0]);
    }

    #[test]
    fn l_interruttore_ferma_la_pipeline() {
        let p = Progresso::silenzioso();
        let i = p.interruttore();
        assert!(p.verifica().is_ok());
        i.annulla();
        assert_eq!(p.verifica(), Err(Annullato));
    }

    #[test]
    fn l_annullamento_viene_annunciato_una_volta_per_verifica() {
        let (p, visti) = raccogli();
        p.interruttore().annulla();
        assert!(p.verifica().is_err());
        let v = visti.lock().unwrap();
        assert!(matches!(v.last(), Some(Evento::Annullata)));
    }

    #[test]
    fn gli_eventi_arrivano_sul_canale() {
        let (p, rx) = Progresso::canale();
        p.avviso("il carattere scelto non e' disponibile");
        match rx.recv().unwrap() {
            Evento::Avviso { messaggio } => assert!(messaggio.contains("carattere")),
            altro => panic!("evento inatteso: {altro:?}"),
        }
    }

    #[test]
    fn senza_ricevitore_l_elaborazione_continua() {
        let (p, rx) = Progresso::canale();
        drop(rx);
        p.avviso("nessuno ascolta");
        p.passo(Fase::Codifica, 0.5);
    }

    #[test]
    fn ogni_fase_ha_un_nome_e_un_etichetta_distinti() {
        let nomi: Vec<&str> = Fase::TUTTE.iter().map(|f| f.nome()).collect();
        let mut unici = nomi.clone();
        unici.sort_unstable();
        unici.dedup();
        assert_eq!(unici.len(), nomi.len(), "nomi ripetuti: {nomi:?}");
        assert!(Fase::TUTTE.iter().all(|f| !f.etichetta().is_empty()));
    }
}
