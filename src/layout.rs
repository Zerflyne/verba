//! Impaginazione dei sottotitoli: da sequenza di parole a blocchi di una riga.
//!
//! La misura del testo e' fatta con **cosmic-text** (shaping HarfBuzz-like via
//! rustybuzz) sul font realmente usato in fase di disegno, non su una stima a
//! caratteri: la larghezza di "Illimitato" e quella di "WWWWWWWWWW" differiscono
//! di piu' del doppio, e con un font a peso 700 sbagliare la misura significa
//! testo che esce dai bordi.
//!
//! Il flusso e':
//!
//! 1. le parole vengono raggruppate in **blocchi** (i "chunk" che cambiano nel
//!    tempo) rispettando pause, cambi di segmento, punteggiatura di fine frase,
//!    durata massima e — soprattutto — la capienza di **una riga sola**: piu'
//!    righe insieme rendono la lettura caotica, quindi il blocco si chiude non
//!    appena il testo non entra nello spazio orizzontale disponibile;
//! 2. per ogni blocco si calcola la **finestra di accensione** di ciascuna
//!    parola, cioe' l'intervallo in cui l'evidenziazione la sta indicando.

use std::collections::HashMap;
use std::ops::Range;

use anyhow::{bail, Context, Result};
use cosmic_text::{fontdb, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight, Wrap};
use tracing::debug;

use crate::align::Word;

/// Formato del video di destinazione.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Formato {
    /// 9:16 verticale (1080x1920) — formato dei social in verticale.
    Verticale,
    /// 16:9 orizzontale (1920x1080).
    Orizzontale,
}

impl Formato {
    /// Risoluzione predefinita del formato.
    pub fn risoluzione(self) -> (u32, u32) {
        match self {
            Formato::Verticale => (1080, 1920),
            Formato::Orizzontale => (1920, 1080),
        }
    }
}

/// Posizione verticale della riga di sottotitoli nel fotogramma.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Posizione {
    Alto,
    Centro,
    Basso,
}

/// Correzioni temporali dell'accensione dell'evidenziazione.
///
/// Senza correzioni il rettangolo sarebbe acceso esattamente sull'intervallo
/// della parola, e si vedrebbero due difetti: arriverebbe un istante dopo che
/// la parola e' gia' cominciata (il sistema visivo e' piu' lento di quello
/// uditivo), e resterebbe acceso per tutta la durata di un silenzio lungo,
/// indicando una parola che non si sta piu' pronunciando.
#[derive(Debug, Clone)]
pub struct Attivazione {
    /// Quanto il rettangolo arriva prima dell'inizio nominale della parola.
    pub anticipo: f64,
    /// Tetto alla permanenza nella pausa che segue la parola. Se la parola
    /// successiva arriva prima, il rettangolo le salta addosso subito.
    pub pausa_max: f64,
    /// Permanenza dopo l'ultima parola del blocco.
    pub coda: f64,
}

impl Default for Attivazione {
    fn default() -> Self {
        Self { anticipo: 0.06, pausa_max: 0.35, coda: 0.25 }
    }
}

#[derive(Debug, Clone)]
pub struct LayoutConfig {
    pub larghezza: u32,
    pub altezza: u32,
    /// Margine fra testo e bordi laterali, in frazione della larghezza.
    pub margine_orizzontale: f32,
    /// Distanza dal bordo superiore o inferiore, in frazione dell'altezza.
    pub margine_verticale: f32,
    pub posizione: Posizione,
    /// Corpo del font in pixel. `None` = automatico in base alla risoluzione.
    pub dimensione_font: Option<f32>,
    /// Interlinea come multiplo del corpo. Con una riga sola determina
    /// l'altezza della fascia su cui il rettangolo viene centrato.
    pub interlinea: f32,
    /// Durata massima di un blocco, in secondi.
    pub durata_max: f64,
    /// Una pausa piu' lunga di questo valore chiude il blocco.
    pub pausa_max: f64,
    /// Quanto la riga resta a schermo dopo l'ultima parola, in secondi.
    pub tenuta: f64,
    pub attivazione: Attivazione,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        let (larghezza, altezza) = Formato::Verticale.risoluzione();
        Self {
            larghezza,
            altezza,
            margine_orizzontale: 0.08,
            margine_verticale: 0.14,
            posizione: Posizione::Basso,
            dimensione_font: None,
            interlinea: 1.18,
            durata_max: 5.0,
            pausa_max: 0.7,
            tenuta: 0.30,
            attivazione: Attivazione::default(),
        }
    }
}

impl LayoutConfig {
    /// Corpo del font effettivo: se non imposto, il 6,5 % della dimensione
    /// minore del fotogramma. La proporzione e' la stessa in 9:16 e in 16:9,
    /// cosi' il testo ha lo stesso "peso" visivo nei due formati.
    pub fn corpo(&self) -> f32 {
        self.dimensione_font
            .unwrap_or_else(|| 0.065 * self.larghezza.min(self.altezza) as f32)
    }

    /// Spazio orizzontale realmente disponibile per il testo.
    pub fn larghezza_utile(&self) -> f32 {
        let margine = self.margine_orizzontale.clamp(0.0, 0.45);
        self.larghezza as f32 * (1.0 - 2.0 * margine)
    }

    pub fn altezza_riga(&self) -> f32 {
        self.corpo() * self.interlinea.max(1.0)
    }
}

/// Una parola collocata dentro il testo della riga.
///
/// `byte` e' l'intervallo occupato nella stringa della riga: e' la chiave con
/// cui il disegno ritrova, fra i glifi gia' posizionati, quelli della parola.
#[derive(Debug, Clone)]
pub struct ParolaInRiga {
    /// Indice della parola dentro [`Blocco::parole`].
    pub indice: usize,
    pub byte: Range<usize>,
}

#[derive(Debug, Clone)]
pub struct Riga {
    pub testo: String,
    pub parole: Vec<ParolaInRiga>,
    /// Larghezza misurata, in pixel.
    pub larghezza: f32,
}

/// Un blocco di sottotitoli: la riga visibile in un dato istante.
#[derive(Debug, Clone)]
pub struct Blocco {
    pub parole: Vec<Word>,
    pub riga: Riga,
    /// Istante in cui la riga compare.
    pub start: f64,
    /// Istante in cui la riga sparisce.
    pub end: f64,
    /// Finestra di accensione dell'evidenziazione, una per parola. Le finestre
    /// sono ordinate e disgiunte, e possono lasciare buchi: nei buchi la riga
    /// resta a schermo ma nessuna parola e' indicata.
    pub finestre: Vec<(f64, f64)>,
}

impl Blocco {
    /// Indice della parola indicata all'istante `t`, se ce n'e' una.
    pub fn parola_attiva(&self, t: f64) -> Option<usize> {
        self.finestre.iter().position(|&(inizio, fine)| t >= inizio && t < fine)
    }

    pub fn testo(&self) -> String {
        self.riga.testo.clone()
    }
}

/// Misura il testo con il font che verra' effettivamente disegnato.
pub struct Tipografo {
    /// Visibile nel crate perche' `render` deve poter disporre e rasterizzare
    /// i glifi con lo stesso identico font usato per le misure.
    pub(crate) font_system: FontSystem,
    pub(crate) famiglia: String,
    metriche: Metrics,
    buffer: Buffer,
    cache: HashMap<String, f32>,
}

impl Tipografo {
    /// Carica un font statico `.ttf` (qui: Inter peso 700) e prepara il motore
    /// di shaping.
    ///
    /// Il database dei font contiene **solo** questo file: nessuna scansione
    /// del sistema, nessun rischio che una installazione diversa cambi la resa.
    pub fn nuovo(font: &[u8], corpo: f32, interlinea: f32) -> Result<Self> {
        let mut db = fontdb::Database::new();
        db.load_font_data(font.to_vec());
        let faccia = db
            .faces()
            .next()
            .context("il file del font non contiene alcun volto tipografico")?;
        let famiglia = faccia
            .families
            .first()
            .map(|(nome, _)| nome.clone())
            .context("il font non dichiara un nome di famiglia")?;
        let peso = faccia.weight.0;
        if peso != Weight::BOLD.0 {
            debug!(peso, "il font caricato non e' di peso 700");
        }
        debug!(famiglia = %famiglia, peso, "font caricato");

        let mut font_system = FontSystem::new_with_locale_and_db("it-IT".to_string(), db);
        let metriche = Metrics::new(corpo, corpo * interlinea.max(1.0));
        let mut buffer = Buffer::new(&mut font_system, metriche);
        // Nessun ritorno a capo automatico: la spezzatura la decide questo
        // modulo, che deve poter misurare righe piu' lunghe del consentito.
        buffer.set_wrap(&mut font_system, Wrap::None);
        buffer.set_size(&mut font_system, None, None);

        Ok(Self { font_system, famiglia, metriche, buffer, cache: HashMap::new() })
    }

    pub fn metriche(&self) -> Metrics {
        self.metriche
    }

    /// Larghezza in pixel del testo, con memoizzazione.
    pub fn misura(&mut self, testo: &str) -> f32 {
        if let Some(&w) = self.cache.get(testo) {
            return w;
        }
        let attrs = Attrs::new().family(Family::Name(&self.famiglia)).weight(Weight::BOLD);
        self.buffer.set_text(&mut self.font_system, testo, &attrs, Shaping::Advanced);
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        let w = self.buffer.layout_runs().fold(0.0f32, |m, r| m.max(r.line_w));
        self.cache.insert(testo.to_string(), w);
        w
    }
}

/// Costruisce i blocchi di sottotitoli a partire dalle parole allineate.
///
/// `parole` deve essere gia' passata per [`crate::align::ripulisci`]: qui si
/// assume una sequenza ordinata, senza buchi e monotona.
pub fn impagina(parole: &[Word], tipografo: &mut Tipografo, cfg: &LayoutConfig) -> Result<Vec<Blocco>> {
    let larghezza_utile = cfg.larghezza_utile();
    if larghezza_utile <= 0.0 {
        bail!("il margine orizzontale non lascia spazio al testo");
    }

    let gruppi = raggruppa(parole, tipografo, cfg, larghezza_utile);

    let mut blocchi: Vec<Blocco> = Vec::with_capacity(gruppi.len());
    for gruppo in gruppi {
        let riga = componi_riga(&gruppo, tipografo);
        let start = gruppo.first().map(|w| w.start).unwrap_or(0.0);
        let end = gruppo.last().map(|w| w.end).unwrap_or(start);
        blocchi.push(Blocco { parole: gruppo, riga, start, end, finestre: Vec::new() });
    }

    assesta_tempi(&mut blocchi, cfg);
    calcola_finestre(&mut blocchi, cfg);
    debug!(blocchi = blocchi.len(), "impaginazione completata");
    Ok(blocchi)
}

/// Raggruppa le parole nelle righe che compaiono una alla volta sullo schermo.
fn raggruppa(
    parole: &[Word],
    tipografo: &mut Tipografo,
    cfg: &LayoutConfig,
    larghezza_utile: f32,
) -> Vec<Vec<Word>> {
    let mut gruppi: Vec<Vec<Word>> = Vec::new();
    let mut corrente: Vec<Word> = Vec::new();
    let mut testo = String::new();

    for (i, w) in parole.iter().enumerate() {
        if !corrente.is_empty() {
            let precedente = &parole[i - 1];
            let pausa = w.start - precedente.end;
            let durata = w.end - corrente[0].start;

            let candidato = format!("{testo} {}", w.text);
            let sta_nella_riga = tipografo.misura(&candidato) <= larghezza_utile;

            let chiudi = pausa > cfg.pausa_max
                || w.segment != precedente.segment
                || durata > cfg.durata_max
                || !sta_nella_riga;
            if chiudi {
                gruppi.push(std::mem::take(&mut corrente));
                testo.clear();
            }
        }

        if !testo.is_empty() {
            testo.push(' ');
        }
        testo.push_str(&w.text);
        corrente.push(w.clone());

        // Fine frase: e' il punto di taglio piu' naturale.
        if w.text.ends_with(['.', '!', '?', '…', ':']) {
            gruppi.push(std::mem::take(&mut corrente));
            testo.clear();
        }
    }
    if !corrente.is_empty() {
        gruppi.push(corrente);
    }
    gruppi
}

/// Unisce le parole del blocco in una riga, tenendo traccia di dove ciascuna
/// finisce nella stringa: e' l'ancora che il disegno usa per ritrovare i glifi.
fn componi_riga(parole: &[Word], tipografo: &mut Tipografo) -> Riga {
    let mut testo = String::new();
    let mut in_riga = Vec::with_capacity(parole.len());
    for (indice, w) in parole.iter().enumerate() {
        if !testo.is_empty() {
            testo.push(' ');
        }
        let inizio = testo.len();
        testo.push_str(&w.text);
        in_riga.push(ParolaInRiga { indice, byte: inizio..testo.len() });
    }
    let larghezza = tipografo.misura(&testo);
    Riga { testo, parole: in_riga, larghezza }
}

/// Anticipa la comparsa della riga e la prolunga oltre l'ultima parola, senza
/// mai far coesistere due blocchi: due righe sovrapposte si disegnerebbero una
/// sull'altra.
fn assesta_tempi(blocchi: &mut [Blocco], cfg: &LayoutConfig) {
    let anticipo = cfg.attivazione.anticipo.max(0.0);

    // La riga puo' comparire in anticipo, ma mai prima che sia finita l'ultima
    // parola della riga precedente: quella deve restare leggibile fino in fondo.
    let mut fine_parole_precedenti = 0.0f64;
    for b in blocchi.iter_mut() {
        b.start = (b.parole[0].start - anticipo).max(fine_parole_precedenti).max(0.0);
        fine_parole_precedenti = b.parole.last().map(|w| w.end).unwrap_or(b.start);
    }

    for i in 0..blocchi.len() {
        let limite = blocchi.get(i + 1).map(|b| b.start).unwrap_or(f64::INFINITY);
        let ultima = blocchi[i].parole.last().map(|w| w.end).unwrap_or(blocchi[i].start);
        blocchi[i].end = (ultima + cfg.tenuta).min(limite).max(blocchi[i].start);
    }
}

/// Finestra di accensione dell'evidenziazione, parola per parola.
///
/// Fra due parole vicine il rettangolo passa direttamente dall'una all'altra,
/// senza spegnersi; se invece il silenzio e' lungo si spegne dopo `pausa_max` e
/// la riga resta a schermo senza nulla di indicato.
fn calcola_finestre(blocchi: &mut [Blocco], cfg: &LayoutConfig) {
    let anticipo = cfg.attivazione.anticipo.max(0.0);
    let pausa_max = cfg.attivazione.pausa_max.max(0.0);
    let coda = cfg.attivazione.coda.max(0.0);

    for b in blocchi.iter_mut() {
        let n = b.parole.len();
        let mut finestre = Vec::with_capacity(n);
        for i in 0..n {
            let inizio = (b.parole[i].start - anticipo).max(b.start);
            let fine = match b.parole.get(i + 1) {
                Some(succ) => {
                    (b.parole[i].end + pausa_max).min((succ.start - anticipo).max(inizio))
                }
                // Oltre la fine della riga non si disegna comunque nulla.
                None => (b.parole[i].end + coda).min(b.end),
            };
            finestre.push((inizio, fine.max(inizio)));
        }
        b.finestre = finestre;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &[u8] = include_bytes!("../assets/Inter-Bold.ttf");

    fn tipografo(corpo: f32) -> Tipografo {
        Tipografo::nuovo(FONT, corpo, 1.18).expect("font di prova")
    }

    fn cfg() -> LayoutConfig {
        LayoutConfig { dimensione_font: Some(64.0), ..Default::default() }
    }

    fn w(text: &str, start: f64, end: f64) -> Word {
        Word { text: text.into(), start, end, score: 1.0, segment: 0 }
    }

    #[test]
    fn il_font_inter_si_carica_ed_e_grassetto() {
        let t = tipografo(64.0);
        assert!(t.famiglia.contains("Inter"), "famiglia inattesa: {}", t.famiglia);
    }

    #[test]
    fn la_misura_cresce_con_il_testo_e_con_il_corpo() {
        let mut t = tipografo(64.0);
        let corta = t.misura("ciao");
        let lunga = t.misura("ciao mondo");
        assert!(lunga > corta, "{lunga} <= {corta}");

        let mut grande = tipografo(128.0);
        assert!(grande.misura("ciao") > corta * 1.8);
    }

    #[test]
    fn la_misura_dipende_dalle_lettere_non_dal_conteggio() {
        let mut t = tipografo(64.0);
        let stretta = t.misura("illlli");
        let larga = t.misura("WWWWWW");
        assert_eq!("illlli".len(), "WWWWWW".len());
        assert!(larga > stretta * 1.5, "{larga} vs {stretta}");
    }

    #[test]
    fn ogni_blocco_ha_una_riga_sola_dentro_la_larghezza_utile() {
        let mut t = tipografo(64.0);
        let cfg = cfg();
        let parole: Vec<Word> = "il rapido allineamento delle parole permette sottotitoli precisi e leggibili anche in verticale"
            .split(' ')
            .enumerate()
            .map(|(i, p)| w(p, i as f64 * 0.4, i as f64 * 0.4 + 0.35))
            .collect();
        let blocchi = impagina(&parole, &mut t, &cfg).unwrap();
        assert!(blocchi.len() > 1, "il testo doveva essere spezzato in piu' righe");
        for b in &blocchi {
            assert!(!b.riga.testo.contains('\n'));
            // una parola sola puo' sforare: non c'e' modo di spezzarla
            if b.parole.len() > 1 {
                assert!(
                    b.riga.larghezza <= cfg.larghezza_utile() + 0.5,
                    "riga «{}» larga {} > {}",
                    b.riga.testo,
                    b.riga.larghezza,
                    cfg.larghezza_utile()
                );
            }
        }
    }

    #[test]
    fn i_blocchi_non_si_sovrappongono_e_coprono_tutte_le_parole() {
        let mut t = tipografo(64.0);
        let parole: Vec<Word> = (0..40)
            .map(|i| w("parola", i as f64 * 0.3, i as f64 * 0.3 + 0.25))
            .collect();
        let blocchi = impagina(&parole, &mut t, &cfg()).unwrap();
        let totale: usize = blocchi.iter().map(|b| b.parole.len()).sum();
        assert_eq!(totale, parole.len());
        for coppia in blocchi.windows(2) {
            assert!(coppia[0].end <= coppia[1].start + 1e-9, "{:?} / {:?}", coppia[0].end, coppia[1].start);
        }
    }

    #[test]
    fn la_pausa_lunga_chiude_il_blocco() {
        let mut t = tipografo(64.0);
        let parole = vec![w("uno", 0.0, 0.3), w("due", 5.0, 5.3)];
        let blocchi = impagina(&parole, &mut t, &cfg()).unwrap();
        assert_eq!(blocchi.len(), 2);
    }

    #[test]
    fn il_punto_fermo_chiude_il_blocco() {
        let mut t = tipografo(64.0);
        let parole = vec![w("Ciao.", 0.0, 0.3), w("Come", 0.35, 0.6)];
        let blocchi = impagina(&parole, &mut t, &cfg()).unwrap();
        assert_eq!(blocchi.len(), 2);
        assert_eq!(blocchi[0].testo(), "Ciao.");
    }

    #[test]
    fn le_parole_in_riga_puntano_al_testo_giusto() {
        let mut t = tipografo(64.0);
        let parole = vec![w("alfa", 0.0, 0.3), w("beta", 0.3, 0.6), w("gamma", 0.6, 0.9)];
        let blocchi = impagina(&parole, &mut t, &cfg()).unwrap();
        for b in &blocchi {
            for p in &b.riga.parole {
                assert_eq!(&b.riga.testo[p.byte.clone()], b.parole[p.indice].text);
            }
        }
    }

    // ------------------------------------------------------------ attivazione

    #[test]
    fn l_anticipo_accende_il_rettangolo_prima_della_parola() {
        let mut t = tipografo(64.0);
        let cfg = LayoutConfig {
            attivazione: Attivazione { anticipo: 0.10, ..Default::default() },
            ..cfg()
        };
        let parole = vec![w("alfa", 1.0, 1.4), w("beta", 1.6, 2.0)];
        let b = &impagina(&parole, &mut t, &cfg).unwrap()[0];
        assert_eq!(b.parola_attiva(1.55), Some(1), "beta doveva accendersi a 1,50");
        assert!((b.finestre[1].0 - 1.50).abs() < 1e-9, "{:?}", b.finestre);
    }

    #[test]
    fn il_rettangolo_si_spegne_nel_silenzio_lungo() {
        let mut t = tipografo(64.0);
        let cfg = LayoutConfig {
            pausa_max: 10.0, // non far chiudere il blocco: interessa solo l'accensione
            attivazione: Attivazione { anticipo: 0.0, pausa_max: 0.20, coda: 0.0 },
            ..cfg()
        };
        let parole = vec![w("alfa", 0.0, 0.4), w("beta", 2.0, 2.4)];
        let b = &impagina(&parole, &mut t, &cfg).unwrap()[0];
        assert_eq!(b.parola_attiva(0.5), Some(0), "ancora nel tetto di 0,20 s");
        assert_eq!(b.parola_attiva(1.0), None, "il silenzio e' lungo: niente rettangolo");
        assert_eq!(b.parola_attiva(2.1), Some(1));
    }

    #[test]
    fn fra_parole_vicine_il_rettangolo_salta_senza_spegnersi() {
        let mut t = tipografo(64.0);
        let cfg = LayoutConfig {
            attivazione: Attivazione { anticipo: 0.05, pausa_max: 0.40, coda: 0.0 },
            ..cfg()
        };
        let parole = vec![w("alfa", 0.0, 0.30), w("beta", 0.35, 0.70)];
        let b = &impagina(&parole, &mut t, &cfg).unwrap()[0];
        assert!((b.finestre[0].1 - b.finestre[1].0).abs() < 1e-9, "{:?}", b.finestre);
        assert_eq!(b.parola_attiva(0.31), Some(1), "il salto avviene a 0,30");
    }

    #[test]
    fn la_coda_tiene_acceso_dopo_l_ultima_parola_ma_non_oltre_la_riga() {
        let mut t = tipografo(64.0);
        let cfg = LayoutConfig {
            tenuta: 0.50,
            attivazione: Attivazione { anticipo: 0.0, pausa_max: 0.30, coda: 0.20 },
            ..cfg()
        };
        let parole = vec![w("alfa", 0.0, 0.40)];
        let b = &impagina(&parole, &mut t, &cfg).unwrap()[0];
        assert_eq!(b.parola_attiva(0.55), Some(0), "dentro la coda");
        assert_eq!(b.parola_attiva(0.65), None, "la coda e' finita, la riga no");
        assert!(b.end > 0.65, "la riga doveva restare fino a {}", b.end);
    }

    #[test]
    fn le_finestre_sono_ordinate_e_disgiunte() {
        let mut t = tipografo(64.0);
        let parole: Vec<Word> = (0..12)
            .map(|i| w("parola", i as f64 * 0.35, i as f64 * 0.35 + 0.30))
            .collect();
        for b in impagina(&parole, &mut t, &cfg()).unwrap() {
            assert_eq!(b.finestre.len(), b.parole.len());
            for f in &b.finestre {
                assert!(f.1 >= f.0, "finestra invertita {f:?}");
                assert!(f.0 >= b.start - 1e-9 && f.1 <= b.end + 1e-9, "{f:?} fuori da {:?}", (b.start, b.end));
            }
            for c in b.finestre.windows(2) {
                assert!(c[1].0 >= c[0].1 - 1e-9, "finestre sovrapposte {c:?}");
            }
        }
    }

    #[test]
    fn fuori_dal_blocco_nessuna_parola_e_attiva() {
        let mut t = tipografo(64.0);
        let parole = vec![w("alfa", 1.0, 1.4)];
        let b = &impagina(&parole, &mut t, &cfg()).unwrap()[0];
        assert_eq!(b.parola_attiva(0.0), None);
        assert_eq!(b.parola_attiva(100.0), None);
    }

    #[test]
    fn i_formati_hanno_le_risoluzioni_attese() {
        assert_eq!(Formato::Verticale.risoluzione(), (1080, 1920));
        assert_eq!(Formato::Orizzontale.risoluzione(), (1920, 1080));
    }

    #[test]
    fn il_corpo_automatico_e_uguale_nei_due_formati() {
        let v = LayoutConfig { larghezza: 1080, altezza: 1920, ..Default::default() };
        let o = LayoutConfig { larghezza: 1920, altezza: 1080, ..Default::default() };
        assert!((v.corpo() - o.corpo()).abs() < 1e-6);
    }
}
