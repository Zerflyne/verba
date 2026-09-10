//! La sequenza di parole: struttura dati mutabile, separata dal risultato
//! grezzo del modello.
//!
//! L'allineatore produce un vettore di [`Parola`]; da quel momento in poi il
//! resto del programma non lavora piu' sul risultato del modello ma su una
//! [`Trascrizione`], che tiene **entrambe** le versioni — quella grezza, cosi'
//! com'e' uscita, e quella normalizzata da [`crate::pulizia::ripulisci`].
//!
//! La separazione non e' un vezzo: serve a poter correggere il testo e i tempi
//! a mano senza perdere il riferimento a cio' che il modello aveva davvero
//! detto, e a poter rinormalizzare dopo ogni correzione. Ogni parola porta un
//! [`IdParola`] che **non cambia** quando la sequenza viene modificata: e' cio'
//! che permette all'interfaccia di tenere il segno su una parola mentre le
//! altre intorno si spostano.

use serde::{Deserialize, Serialize};

use crate::pulizia::{self, DURATA_MINIMA_PAROLA};

/// Identificativo stabile di una parola all'interno di una trascrizione.
///
/// E' unico nella trascrizione che lo ha emesso e non viene mai riusato: una
/// parola cancellata si porta via il suo identificativo per sempre. Non ha
/// significato fuori dalla trascrizione di provenienza e non va serializzato
/// come se fosse un indice.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct IdParola(u64);

impl IdParola {
    /// Valore grezzo, per le chiavi delle mappe dell'interfaccia.
    pub fn valore(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for IdParola {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "p{}", self.0)
    }
}

/// Una parola con i suoi tempi assoluti nel file originale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parola {
    /// Identificativo stabile. Vale [`IdParola`] zero finche' la parola non
    /// entra in una [`Trascrizione`], che e' l'unica a poterli assegnare.
    #[serde(default = "id_provvisorio")]
    pub id: IdParola,
    /// Testo come mostrato all'utente (punteggiatura e maiuscole preservate).
    pub testo: String,
    pub inizio: f64,
    pub fine: f64,
    /// Confidenza media dell'allineamento, in [0, 1].
    pub confidenza: f32,
    /// Indice del segmento di provenienza.
    pub segmento: usize,
}

fn id_provvisorio() -> IdParola {
    IdParola(0)
}

impl Parola {
    /// Una parola non ancora inserita in una trascrizione.
    pub fn nuova(testo: impl Into<String>, inizio: f64, fine: f64) -> Self {
        Self {
            id: id_provvisorio(),
            testo: testo.into(),
            inizio,
            fine,
            confidenza: 1.0,
            segmento: 0,
        }
    }

    /// Durata in secondi. Puo' essere zero, mai negativa dopo la pulizia.
    pub fn durata(&self) -> f64 {
        (self.fine - self.inizio).max(0.0)
    }

    /// Vero se la confidenza sta sotto la soglia: e' la parola su cui conviene
    /// che l'utente vada a guardare.
    pub fn incerta(&self, soglia: f32) -> bool {
        self.confidenza < soglia
    }
}

/// La sequenza di parole di un file, nella versione grezza e in quella pulita.
///
/// Le due versioni hanno gli stessi identificativi: `grezze` conserva i tempi e
/// il testo cosi' come li ha prodotti l'allineatore, `parole` e' quella su cui
/// lavora tutto il resto del programma e su cui vale l'invariante stabilita da
/// [`crate::pulizia::ripulisci`] — ordinata, senza buchi, senza sovrapposizioni.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trascrizione {
    parole: Vec<Parola>,
    grezze: Vec<Parola>,
    prossimo_id: u64,
    durata_audio: f64,
    durata_minima: f64,
}

impl Trascrizione {
    /// Costruisce la trascrizione dal risultato dell'allineatore: assegna gli
    /// identificativi, conserva l'originale e normalizza.
    pub fn nuova(grezze: Vec<Parola>, durata_audio: f64) -> Self {
        Self::con_durata_minima(grezze, durata_audio, DURATA_MINIMA_PAROLA)
    }

    /// Come [`Trascrizione::nuova`], con una durata minima per parola diversa
    /// da quella predefinita.
    pub fn con_durata_minima(
        mut grezze: Vec<Parola>,
        durata_audio: f64,
        durata_minima: f64,
    ) -> Self {
        let mut prossimo_id = 1u64;
        for p in grezze.iter_mut() {
            p.id = IdParola(prossimo_id);
            prossimo_id += 1;
        }
        let parole = pulizia::ripulisci(grezze.clone(), durata_audio, durata_minima);
        Self { parole, grezze, prossimo_id, durata_audio, durata_minima }
    }

    /// Una trascrizione senza parole: e' il caso di un audio in cui non e'
    /// stato riconosciuto parlato, e non e' un errore.
    pub fn vuota(durata_audio: f64) -> Self {
        Self {
            parole: Vec::new(),
            grezze: Vec::new(),
            prossimo_id: 1,
            durata_audio,
            durata_minima: DURATA_MINIMA_PAROLA,
        }
    }

    /// La sequenza normalizzata: e' questa che alimenta impaginazione, disegno
    /// ed export.
    pub fn parole(&self) -> &[Parola] {
        &self.parole
    }

    /// La sequenza come l'ha prodotta il modello, mai modificata.
    pub fn grezze(&self) -> &[Parola] {
        &self.grezze
    }

    pub fn len(&self) -> usize {
        self.parole.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parole.is_empty()
    }

    pub fn durata_audio(&self) -> f64 {
        self.durata_audio
    }

    pub fn durata_minima(&self) -> f64 {
        self.durata_minima
    }

    /// La parola con questo identificativo, se c'e' ancora.
    pub fn parola(&self, id: IdParola) -> Option<&Parola> {
        self.parole.iter().find(|p| p.id == id)
    }

    /// Posizione nella sequenza normalizzata. Cambia quando la sequenza viene
    /// modificata: non conservarla, conserva l'[`IdParola`].
    pub fn posizione(&self, id: IdParola) -> Option<usize> {
        self.parole.iter().position(|p| p.id == id)
    }

    /// La parola pronunciata al tempo `t`, se ce n'e' una.
    pub fn al_tempo(&self, t: f64) -> Option<&Parola> {
        self.parole.iter().find(|p| t >= p.inizio && t < p.fine)
    }

    /// Le parole sotto la soglia di confidenza: quelle da segnalare
    /// nell'interfaccia perche' e' li' che conviene guardare.
    pub fn incerte(&self, soglia: f32) -> impl Iterator<Item = &Parola> {
        self.parole.iter().filter(move |p| p.incerta(soglia))
    }

    /// Il testo intero, parole separate da uno spazio.
    pub fn testo(&self) -> String {
        self.parole.iter().map(|p| p.testo.as_str()).collect::<Vec<_>>().join(" ")
    }

    /// Emette un identificativo mai usato prima in questa trascrizione.
    ///
    /// Serve a chi modifica la sequenza: dividere una parola produce due parole
    /// nuove, e le parole nuove non ereditano mai un identificativo esistente.
    pub fn conia_id(&mut self) -> IdParola {
        let id = IdParola(self.prossimo_id);
        self.prossimo_id += 1;
        id
    }

    /// Sostituisce la sequenza e la rinormalizza.
    ///
    /// E' il punto di ingresso di ogni modifica: chi cambia testo o tempi passa
    /// di qui, cosi' l'invariante della pulizia non puo' essere aggirata.
    pub fn sostituisci(&mut self, parole: Vec<Parola>) {
        self.parole = pulizia::ripulisci(parole, self.durata_audio, self.durata_minima);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parole_di_prova() -> Vec<Parola> {
        vec![
            Parola::nuova("ciao", 0.0, 0.5),
            Parola::nuova("mondo", 0.6, 1.2),
            Parola::nuova("bello", 1.3, 2.0),
        ]
    }

    #[test]
    fn gli_identificativi_sono_assegnati_e_distinti() {
        let t = Trascrizione::nuova(parole_di_prova(), 3.0);
        let ids: Vec<u64> = t.parole().iter().map(|p| p.id.valore()).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.iter().all(|&v| v > 0), "nessun identificativo provvisorio: {ids:?}");
        let mut unici = ids.clone();
        unici.sort_unstable();
        unici.dedup();
        assert_eq!(unici.len(), ids.len(), "identificativi ripetuti: {ids:?}");
    }

    #[test]
    fn la_versione_grezza_resta_accanto_a_quella_pulita() {
        // Una sequenza che la pulizia deve correggere: sovrapposizione.
        let grezze = vec![Parola::nuova("a", 0.0, 1.0), Parola::nuova("b", 0.5, 1.5)];
        let t = Trascrizione::nuova(grezze, 3.0);
        assert!((t.grezze()[1].inizio - 0.5).abs() < 1e-9, "il grezzo e' stato toccato");
        assert!(t.parole()[1].inizio >= t.parole()[0].fine, "la pulizia non ha imposto l'ordine");
    }

    #[test]
    fn gli_identificativi_sopravvivono_alla_pulizia() {
        let grezze = vec![Parola::nuova("a", 0.0, 1.0), Parola::nuova("b", 0.5, 1.5)];
        let t = Trascrizione::nuova(grezze, 3.0);
        let da_grezze: Vec<u64> = t.grezze().iter().map(|p| p.id.valore()).collect();
        let da_pulite: Vec<u64> = t.parole().iter().map(|p| p.id.valore()).collect();
        assert_eq!(da_grezze, da_pulite);
    }

    #[test]
    fn una_parola_si_ritrova_per_identificativo_non_per_posizione() {
        let mut t = Trascrizione::nuova(parole_di_prova(), 3.0);
        let id = t.parole()[2].id;

        // Si toglie la prima parola: la posizione della terza cambia,
        // l'identificativo no.
        let restanti: Vec<Parola> = t.parole()[1..].to_vec();
        t.sostituisci(restanti);

        assert_eq!(t.posizione(id), Some(1), "la posizione doveva scalare");
        assert_eq!(t.parola(id).map(|p| p.testo.as_str()), Some("bello"));
    }

    #[test]
    fn un_identificativo_coniato_non_e_mai_gia_in_uso() {
        let mut t = Trascrizione::nuova(parole_di_prova(), 3.0);
        let nuovo = t.conia_id();
        assert!(t.parola(nuovo).is_none());
        assert_ne!(nuovo, t.conia_id(), "coniati due identificativi uguali");
    }

    #[test]
    fn ogni_modifica_rinormalizza() {
        let mut t = Trascrizione::nuova(parole_di_prova(), 3.0);
        // Si reintroduce a mano una sovrapposizione, come farebbe una
        // correzione dei tempi fatta trascinando i limiti.
        let mut parole = t.parole().to_vec();
        parole[1].inizio = 0.1;
        t.sostituisci(parole);
        assert!(t.parole()[1].inizio >= t.parole()[0].fine);
    }

    #[test]
    fn la_parola_al_tempo_e_quella_giusta() {
        let t = Trascrizione::nuova(parole_di_prova(), 3.0);
        assert_eq!(t.al_tempo(0.7).map(|p| p.testo.as_str()), Some("mondo"));
        assert!(t.al_tempo(2.5).is_none(), "oltre l'ultima parola non c'e' nulla");
    }

    #[test]
    fn le_parole_incerte_sono_quelle_sotto_soglia() {
        let mut parole = parole_di_prova();
        parole[1].confidenza = 0.3;
        let t = Trascrizione::nuova(parole, 3.0);
        let segnalate: Vec<&str> = t.incerte(0.5).map(|p| p.testo.as_str()).collect();
        assert_eq!(segnalate, vec!["mondo"]);
    }

    #[test]
    fn una_trascrizione_vuota_non_e_un_errore() {
        let t = Trascrizione::vuota(12.0);
        assert!(t.is_empty());
        assert_eq!(t.testo(), "");
        assert!((t.durata_audio() - 12.0).abs() < 1e-9);
    }
}
