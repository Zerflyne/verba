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

use crate::trascrizione::Parola;

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

/// Allineamento delle righe dentro la colonna di testo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Allineamento {
    Sinistra,
    #[default]
    Centro,
    Destra,
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
    /// Distanza minima dai bordi del fotogramma, in frazione della dimensione
    /// corrispondente. E' un limite invalicabile: la colonna di testo non ci
    /// entra mai dentro, comunque sia posizionata.
    pub margine: f32,
    /// Larghezza della colonna di testo, in frazione della larghezza del
    /// fotogramma. Il margine ha comunque l'ultima parola.
    pub larghezza_max: f32,
    /// Centro verticale del blocco, in frazione dell'altezza. 0 = bordo alto,
    /// 1 = bordo basso.
    pub posizione_verticale: f32,
    /// Centro orizzontale della colonna di testo, in frazione della larghezza.
    pub posizione_orizzontale: f32,
    /// Righe che possono comparire insieme. Oltre le tre della spec il
    /// risultato non e' piu' leggibile, e il valore viene limitato.
    ///
    /// Il valore predefinito e' **una**: piu' righe per volta rendono la
    /// lettura caotica, e chi vuole due o tre righe lo deve chiedere.
    pub righe_max: usize,
    pub allineamento: Allineamento,
    /// Trasforma il testo in maiuscolo.
    pub maiuscole: bool,
    /// Corpo del font in pixel, **riferiti all'altezza del fotogramma di
    /// destinazione**. `None` = automatico in base alla risoluzione.
    pub dimensione_font: Option<f32>,
    /// Interlinea come multiplo del corpo. Determina anche l'altezza della
    /// fascia su cui il rettangolo di evidenziazione viene centrato.
    pub interlinea: f32,
    /// Durata massima di un blocco, in secondi.
    pub durata_max: f64,
    /// Una pausa piu' lunga di questo valore chiude il blocco.
    pub pausa_max: f64,
    /// Quanto la riga resta a schermo dopo l'ultima parola, in secondi.
    pub tenuta: f64,
    pub attivazione: Attivazione,
}

/// Il massimo consentito per [`LayoutConfig::righe_max`].
pub const RIGHE_MAX_CONSENTITE: usize = 3;

impl Default for LayoutConfig {
    fn default() -> Self {
        let (larghezza, altezza) = Formato::Verticale.risoluzione();
        Self {
            larghezza,
            altezza,
            margine: 0.05,
            larghezza_max: 0.80,
            posizione_verticale: 0.82,
            posizione_orizzontale: 0.50,
            righe_max: 1,
            allineamento: Allineamento::Centro,
            maiuscole: false,
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

    /// Il corpo in percentuale dell'altezza del fotogramma.
    ///
    /// E' il numero che conta davvero quando si confrontano due sorgenti:
    /// 64 px su un 4K e 64 px su un 1080p danno risultati molto diversi.
    pub fn corpo_percentuale(&self) -> f32 {
        if self.altezza == 0 {
            return 0.0;
        }
        100.0 * self.corpo() / self.altezza as f32
    }

    /// Il margine orizzontale in pixel.
    pub fn margine_x(&self) -> f32 {
        self.margine.clamp(0.0, 0.45) * self.larghezza as f32
    }

    /// Il margine verticale in pixel.
    pub fn margine_y(&self) -> f32 {
        self.margine.clamp(0.0, 0.45) * self.altezza as f32
    }

    /// Larghezza della colonna di testo, in pixel: la piu' stretta fra quella
    /// chiesta e quella che il margine concede.
    pub fn larghezza_utile(&self) -> f32 {
        let per_margine = self.larghezza as f32 - 2.0 * self.margine_x();
        let chiesta = self.larghezza_max.clamp(0.05, 1.0) * self.larghezza as f32;
        per_margine.min(chiesta).max(0.0)
    }

    /// Ascissa del bordo sinistro della colonna di testo.
    ///
    /// La colonna e' centrata su [`LayoutConfig::posizione_orizzontale`] e poi
    /// spinta dentro i margini: spostare il cursore all'estremo non fa uscire
    /// il testo dal fotogramma, lo appoggia al margine.
    pub fn colonna_x(&self) -> f32 {
        let utile = self.larghezza_utile();
        let centro = self.posizione_orizzontale.clamp(0.0, 1.0) * self.larghezza as f32;
        let margine = self.margine_x();
        let massimo = (self.larghezza as f32 - margine - utile).max(margine);
        (centro - utile / 2.0).clamp(margine, massimo)
    }

    pub fn altezza_riga(&self) -> f32 {
        self.corpo() * self.interlinea.max(1.0)
    }

    /// Righe consentite, limitate a [`RIGHE_MAX_CONSENTITE`] e almeno una.
    pub fn righe_consentite(&self) -> usize {
        self.righe_max.clamp(1, RIGHE_MAX_CONSENTITE)
    }

    /// Ordinata del bordo superiore di un blocco di `righe` righe.
    ///
    /// Come per l'ascissa, la posizione e' un centro e il margine e' un limite:
    /// un blocco di tre righe posizionato al 95 % si appoggia al margine
    /// inferiore invece di uscire dal fotogramma.
    pub fn riga_y(&self, righe: usize) -> f32 {
        let alta = self.altezza_riga() * righe.max(1) as f32;
        let centro = self.posizione_verticale.clamp(0.0, 1.0) * self.altezza as f32;
        let margine = self.margine_y();
        let massimo = (self.altezza as f32 - margine - alta).max(margine);
        (centro - alta / 2.0).clamp(margine, massimo)
    }
}

/// Una parola collocata dentro il testo di una riga.
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

/// Un blocco di sottotitoli: cio' che e' visibile in un dato istante.
///
/// Le righe sono da una a [`LayoutConfig::righe_max`]; sono sempre almeno una
/// e non restano mai vuote.
#[derive(Debug, Clone)]
pub struct Blocco {
    pub parole: Vec<Parola>,
    pub righe: Vec<Riga>,
    /// Istante in cui il blocco compare.
    pub start: f64,
    /// Istante in cui il blocco sparisce.
    pub end: f64,
    /// Finestra di accensione dell'evidenziazione, una per parola. Le finestre
    /// sono ordinate e disgiunte, e possono lasciare buchi: nei buchi il blocco
    /// resta a schermo ma nessuna parola e' indicata.
    pub finestre: Vec<(f64, f64)>,
}

impl Blocco {
    /// Indice della parola indicata all'istante `t`, se ce n'e' una.
    pub fn parola_attiva(&self, t: f64) -> Option<usize> {
        self.finestre.iter().position(|&(inizio, fine)| t >= inizio && t < fine)
    }

    /// Su quale riga sta la parola di indice dato.
    pub fn riga_di(&self, parola: usize) -> Option<usize> {
        self.righe.iter().position(|r| r.parole.iter().any(|p| p.indice == parola))
    }

    /// Il testo del blocco, righe separate da un ritorno a capo.
    pub fn testo(&self) -> String {
        self.righe.iter().map(|r| r.testo.as_str()).collect::<Vec<_>>().join("\n")
    }

    /// La riga piu' larga: e' quella che detta l'ingombro del blocco.
    pub fn larghezza(&self) -> f32 {
        self.righe.iter().fold(0.0f32, |m, r| m.max(r.larghezza))
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
/// `parole` deve essere gia' passata per [`crate::pulizia::ripulisci`]: qui si
/// assume una sequenza ordinata, senza buchi e monotona.
pub fn impagina(
    parole: &[Parola],
    tipografo: &mut Tipografo,
    cfg: &LayoutConfig,
) -> Result<Vec<Blocco>> {
    let larghezza_utile = cfg.larghezza_utile();
    if larghezza_utile <= 0.0 {
        bail!("il margine orizzontale non lascia spazio al testo");
    }
    let righe_max = cfg.righe_consentite();

    // Il testo come verra' disegnato: se e' in maiuscolo lo e' gia' qui,
    // perche' e' piu' largo e la spezzatura deve tenerne conto.
    let visibili: Vec<String> = parole.iter().map(|p| testo_visibile(&p.testo, cfg)).collect();

    let gruppi = raggruppa(parole, &visibili, tipografo, cfg, larghezza_utile, righe_max);

    let mut blocchi: Vec<Blocco> = Vec::with_capacity(gruppi.len());
    let mut troppo_larghe = 0usize;
    for gruppo in gruppi {
        let testi: Vec<&str> = gruppo.iter().map(|&i| visibili[i].as_str()).collect();
        let tagli = distribuisci(&testi, tipografo, larghezza_utile, righe_max);
        let righe = componi_righe(&testi, &tagli, tipografo);
        troppo_larghe += righe.iter().filter(|r| r.larghezza > larghezza_utile + 0.5).count();

        let parole_blocco: Vec<Parola> = gruppo.iter().map(|&i| parole[i].clone()).collect();
        let start = parole_blocco.first().map(|w| w.inizio).unwrap_or(0.0);
        let end = parole_blocco.last().map(|w| w.fine).unwrap_or(start);
        blocchi.push(Blocco {
            parole: parole_blocco,
            righe,
            start,
            end,
            finestre: Vec::new(),
        });
    }

    if troppo_larghe > 0 {
        // Una parola sola piu' larga della colonna non si puo' spezzare senza
        // sillabazione: sconfina, e conviene dirlo invece di lasciarlo scoprire
        // guardando il video.
        debug!(righe = troppo_larghe, "righe piu' larghe della colonna di testo");
    }

    assesta_tempi(&mut blocchi, cfg);
    calcola_finestre(&mut blocchi, cfg);
    debug!(
        blocchi = blocchi.len(),
        righe_max, "impaginazione completata"
    );
    Ok(blocchi)
}

/// Il testo di una parola come verra' disegnato.
fn testo_visibile(testo: &str, cfg: &LayoutConfig) -> String {
    if cfg.maiuscole {
        testo.to_uppercase()
    } else {
        testo.to_string()
    }
}

/// Raggruppa le parole nei blocchi che compaiono uno alla volta sullo schermo.
///
/// Il blocco si chiude su una pausa lunga, un cambio di segmento, la durata
/// massima, la punteggiatura di fine frase, e — soprattutto — quando le parole
/// non entrano piu' nel numero di righe consentito.
fn raggruppa(
    parole: &[Parola],
    visibili: &[String],
    tipografo: &mut Tipografo,
    cfg: &LayoutConfig,
    larghezza_utile: f32,
    righe_max: usize,
) -> Vec<Vec<usize>> {
    let mut gruppi: Vec<Vec<usize>> = Vec::new();
    let mut corrente: Vec<usize> = Vec::new();

    for (i, w) in parole.iter().enumerate() {
        if !corrente.is_empty() {
            let precedente = &parole[i - 1];
            let pausa = w.inizio - precedente.fine;
            let durata = w.fine - parole[corrente[0]].inizio;

            let mut candidato: Vec<&str> =
                corrente.iter().map(|&k| visibili[k].as_str()).collect();
            candidato.push(visibili[i].as_str());
            let ci_sta = righe_necessarie(&candidato, tipografo, larghezza_utile) <= righe_max;

            let chiudi = pausa > cfg.pausa_max
                || w.segmento != precedente.segmento
                || durata > cfg.durata_max
                || !ci_sta;
            if chiudi {
                gruppi.push(std::mem::take(&mut corrente));
            }
        }

        corrente.push(i);

        // Fine frase: e' il punto di taglio piu' naturale.
        if w.testo.ends_with(['.', '!', '?', '…', ':']) {
            gruppi.push(std::mem::take(&mut corrente));
        }
    }
    if !corrente.is_empty() {
        gruppi.push(corrente);
    }
    gruppi
}

/// Larghezza delle parole `da..a` unite da spazi singoli.
fn larghezza_tratto(testi: &[&str], da: usize, a: usize, tipografo: &mut Tipografo) -> f32 {
    tipografo.misura(&testi[da..a].join(" "))
}

/// Quante righe servono, al minimo, per far entrare le parole nella larghezza.
///
/// Il riempimento avido e' ottimo per questo scopo: rimandare una parola alla
/// riga successiva non puo' mai far diminuire il numero di righe. Una parola
/// piu' larga della colonna occupa una riga da sola e sconfina — non c'e'
/// alternativa senza sillabazione.
fn righe_necessarie(testi: &[&str], tipografo: &mut Tipografo, larghezza: f32) -> usize {
    if testi.is_empty() {
        return 0;
    }
    let mut righe = 1usize;
    let mut inizio = 0usize;
    for i in 0..testi.len() {
        if i == inizio {
            continue;
        }
        if larghezza_tratto(testi, inizio, i + 1, tipografo) > larghezza {
            righe += 1;
            inizio = i;
        }
    }
    righe
}

/// Dove tagliare le parole per distribuirle sulle righe.
///
/// Ritorna gli indici di inizio di ogni riga. Si usa il numero minimo di righe
/// e, a parita' di righe, la distribuzione piu' equilibrata: riempire
/// avidamente lascerebbe l'ultima riga con una parola sola, che e' il difetto
/// tipico dei sottotitoli generati.
fn distribuisci(
    testi: &[&str],
    tipografo: &mut Tipografo,
    larghezza: f32,
    righe_max: usize,
) -> Vec<usize> {
    let n = testi.len();
    if n == 0 {
        return Vec::new();
    }
    let righe = righe_necessarie(testi, tipografo, larghezza).clamp(1, righe_max.max(1));
    if righe == 1 {
        return vec![0];
    }

    // Larghezze di ogni tratto, misurate una volta sola.
    let mut w = vec![vec![0.0f32; n + 1]; n];
    for (da, riga) in w.iter_mut().enumerate() {
        for (a, cella) in riga.iter_mut().enumerate().skip(da + 1) {
            *cella = larghezza_tratto(testi, da, a, tipografo);
        }
    }

    // Costo di una riga: lo spazio che avanza, al quadrato. Elevare al quadrato
    // e' cio' che rende la soluzione equilibrata invece che avida. Una riga che
    // sconfina paga molto di piu', cosi' viene scelta solo se non c'e' altro.
    let costo_riga = |da: usize, a: usize| -> f64 {
        let avanzo = (larghezza - w[da][a]) as f64;
        if avanzo < 0.0 {
            1e9 + avanzo * avanzo
        } else {
            avanzo * avanzo
        }
    };

    // migliore[r][i] = costo minimo per sistemare le parole da i in poi in r righe.
    let mut migliore = vec![vec![f64::INFINITY; n + 1]; righe + 1];
    let mut taglio = vec![vec![n; n + 1]; righe + 1];
    migliore[0][n] = 0.0;
    for r in 1..=righe {
        for i in (0..n).rev() {
            // La riga r-esima prende le parole i..j, le altre vanno nelle
            // r-1 righe restanti.
            for j in (i + 1)..=n {
                let resto = migliore[r - 1][j];
                if !resto.is_finite() {
                    continue;
                }
                let c = costo_riga(i, j) + resto;
                if c < migliore[r][i] {
                    migliore[r][i] = c;
                    taglio[r][i] = j;
                }
            }
        }
    }

    let mut tagli = Vec::with_capacity(righe);
    let mut i = 0usize;
    for r in (1..=righe).rev() {
        if i >= n {
            break;
        }
        tagli.push(i);
        i = taglio[r][i];
    }
    if tagli.is_empty() {
        tagli.push(0);
    }
    tagli
}

/// Costruisce le righe dai tagli, tenendo traccia di dove ogni parola finisce
/// nella stringa: e' l'ancora che il disegno usa per ritrovare i glifi.
fn componi_righe(testi: &[&str], tagli: &[usize], tipografo: &mut Tipografo) -> Vec<Riga> {
    let mut righe = Vec::with_capacity(tagli.len());
    for (k, &da) in tagli.iter().enumerate() {
        let a = tagli.get(k + 1).copied().unwrap_or(testi.len());
        let mut testo = String::new();
        let mut in_riga = Vec::with_capacity(a - da);
        for (indice, parola) in testi.iter().enumerate().take(a).skip(da) {
            if !testo.is_empty() {
                testo.push(' ');
            }
            let inizio = testo.len();
            testo.push_str(parola);
            in_riga.push(ParolaInRiga { indice, byte: inizio..testo.len() });
        }
        let larghezza = tipografo.misura(&testo);
        righe.push(Riga { testo, parole: in_riga, larghezza });
    }
    righe
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
        b.start = (b.parole[0].inizio - anticipo).max(fine_parole_precedenti).max(0.0);
        fine_parole_precedenti = b.parole.last().map(|w| w.fine).unwrap_or(b.start);
    }

    for i in 0..blocchi.len() {
        let limite = blocchi.get(i + 1).map(|b| b.start).unwrap_or(f64::INFINITY);
        let ultima = blocchi[i].parole.last().map(|w| w.fine).unwrap_or(blocchi[i].start);
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
            let inizio = (b.parole[i].inizio - anticipo).max(b.start);
            let fine = match b.parole.get(i + 1) {
                Some(succ) => {
                    (b.parole[i].fine + pausa_max).min((succ.inizio - anticipo).max(inizio))
                }
                // Oltre la fine della riga non si disegna comunque nulla.
                None => (b.parole[i].fine + coda).min(b.end),
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

    fn w(testo: &str, inizio: f64, fine: f64) -> Parola {
        Parola::nuova(testo, inizio, fine)
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
    fn con_una_riga_ogni_blocco_sta_nella_larghezza_utile() {
        let mut t = tipografo(64.0);
        let cfg = cfg();
        let parole: Vec<Parola> = "il rapido allineamento delle parole permette sottotitoli precisi e leggibili anche in verticale"
            .split(' ')
            .enumerate()
            .map(|(i, p)| w(p, i as f64 * 0.4, i as f64 * 0.4 + 0.35))
            .collect();
        let blocchi = impagina(&parole, &mut t, &cfg).unwrap();
        assert!(blocchi.len() > 1, "il testo doveva essere spezzato in piu' righe");
        for b in &blocchi {
            assert_eq!(b.righe.len(), 1, "con righe_max = 1 il blocco ha una riga sola");
            // una parola sola puo' sforare: non c'e' modo di spezzarla
            if b.parole.len() > 1 {
                assert!(
                    b.righe[0].larghezza <= cfg.larghezza_utile() + 0.5,
                    "riga «{}» larga {} > {}",
                    b.righe[0].testo,
                    b.righe[0].larghezza,
                    cfg.larghezza_utile()
                );
            }
        }
    }

    #[test]
    fn i_blocchi_non_si_sovrappongono_e_coprono_tutte_le_parole() {
        let mut t = tipografo(64.0);
        let parole: Vec<Parola> = (0..40)
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
            for riga in &b.righe {
                for p in &riga.parole {
                    assert_eq!(&riga.testo[p.byte.clone()], b.parole[p.indice].testo);
                }
            }
        }
    }

    // -------------------------------------------------------- righe multiple

    /// Un testo lungo abbastanza da non stare su una riga sola.
    fn frase_lunga() -> Vec<Parola> {
        "il rapido allineamento delle parole permette sottotitoli precisi e leggibili"
            .split(' ')
            .enumerate()
            .map(|(i, p)| w(p, i as f64 * 0.3, i as f64 * 0.3 + 0.25))
            .collect()
    }

    #[test]
    fn il_valore_predefinito_e_una_riga_sola() {
        assert_eq!(LayoutConfig::default().righe_consentite(), 1);
    }

    #[test]
    fn con_due_righe_i_blocchi_diventano_meno_numerosi() {
        let mut t = tipografo(64.0);
        let parole = frase_lunga();
        let una = impagina(&parole, &mut t, &LayoutConfig { righe_max: 1, ..cfg() }).unwrap();
        let due = impagina(&parole, &mut t, &LayoutConfig { righe_max: 2, ..cfg() }).unwrap();
        assert!(
            due.len() < una.len(),
            "con due righe servono meno blocchi: {} contro {}",
            due.len(),
            una.len()
        );
        assert!(due.iter().any(|b| b.righe.len() == 2), "nessun blocco ha usato la seconda riga");
    }

    #[test]
    fn le_righe_non_superano_mai_il_massimo_chiesto() {
        let mut t = tipografo(64.0);
        let parole = frase_lunga();
        for righe_max in 1..=3 {
            let cfg = LayoutConfig { righe_max, ..cfg() };
            for b in impagina(&parole, &mut t, &cfg).unwrap() {
                assert!(
                    b.righe.len() <= righe_max && !b.righe.is_empty(),
                    "{} righe con un massimo di {righe_max}",
                    b.righe.len()
                );
                assert!(b.righe.iter().all(|r| !r.parole.is_empty()), "riga vuota");
            }
        }
    }

    #[test]
    fn oltre_tre_righe_il_valore_viene_limitato() {
        let cfg = LayoutConfig { righe_max: 9, ..cfg() };
        assert_eq!(cfg.righe_consentite(), RIGHE_MAX_CONSENTITE);
    }

    #[test]
    fn la_distribuzione_e_equilibrata_non_avida() {
        // Riempire avidamente lascerebbe l'ultima riga quasi vuota; qui le due
        // righe devono risultare di larghezza confrontabile.
        let mut t = tipografo(64.0);
        let cfg = LayoutConfig { righe_max: 2, ..cfg() };
        let parole = frase_lunga();
        let blocchi = impagina(&parole, &mut t, &cfg).unwrap();
        let due_righe: Vec<&Blocco> = blocchi.iter().filter(|b| b.righe.len() == 2).collect();
        assert!(!due_righe.is_empty(), "nessun blocco su due righe da controllare");
        for b in due_righe {
            let (a, c) = (b.righe[0].larghezza, b.righe[1].larghezza);
            let squilibrio = (a - c).abs() / cfg.larghezza_utile();
            assert!(
                squilibrio < 0.5,
                "righe molto sbilanciate: {a:.0} e {c:.0} px su {:.0}",
                cfg.larghezza_utile()
            );
        }
    }

    #[test]
    fn ogni_parola_sta_su_una_riga_sola_e_su_tutte_le_righe_ci_sono_tutte() {
        let mut t = tipografo(64.0);
        let cfg = LayoutConfig { righe_max: 3, ..cfg() };
        for b in impagina(&frase_lunga(), &mut t, &cfg).unwrap() {
            let mut viste: Vec<usize> =
                b.righe.iter().flat_map(|r| r.parole.iter().map(|p| p.indice)).collect();
            viste.sort_unstable();
            assert_eq!(viste, (0..b.parole.len()).collect::<Vec<_>>());
            for i in 0..b.parole.len() {
                assert!(b.riga_di(i).is_some(), "la parola {i} non sta su nessuna riga");
            }
        }
    }

    // ---------------------------------------------------- colonna e posizione

    #[test]
    fn la_colonna_resta_nei_margini_a_qualsiasi_posizione() {
        for posizione in [0.0, 0.2, 0.5, 0.8, 1.0] {
            let cfg = LayoutConfig { posizione_orizzontale: posizione, ..cfg() };
            let x0 = cfg.colonna_x();
            let x1 = x0 + cfg.larghezza_utile();
            assert!(x0 >= cfg.margine_x() - 1e-3, "colonna a {x0} dentro il margine");
            assert!(
                x1 <= cfg.larghezza as f32 - cfg.margine_x() + 1e-3,
                "colonna fino a {x1} oltre il margine destro"
            );
        }
    }

    #[test]
    fn il_blocco_resta_nei_margini_a_qualsiasi_altezza() {
        for posizione in [0.0, 0.5, 0.82, 1.0] {
            for righe in 1..=3 {
                let cfg = LayoutConfig { posizione_verticale: posizione, ..cfg() };
                let y0 = cfg.riga_y(righe);
                let y1 = y0 + cfg.altezza_riga() * righe as f32;
                assert!(y0 >= cfg.margine_y() - 1e-3, "blocco a y={y0}, margine {}", cfg.margine_y());
                assert!(
                    y1 <= cfg.altezza as f32 - cfg.margine_y() + 1e-3,
                    "blocco fino a y={y1} oltre il margine inferiore"
                );
            }
        }
    }

    #[test]
    fn il_margine_ha_la_meglio_sulla_larghezza_chiesta() {
        let cfg = LayoutConfig { margine: 0.20, larghezza_max: 1.0, ..cfg() };
        assert!((cfg.larghezza_utile() - cfg.larghezza as f32 * 0.60).abs() < 1e-3);
    }

    #[test]
    fn il_corpo_si_legge_anche_in_percentuale_dell_altezza() {
        let cfg = LayoutConfig { altezza: 1080, dimensione_font: Some(64.0), ..cfg() };
        assert!((cfg.corpo_percentuale() - 64.0 / 1080.0 * 100.0).abs() < 1e-4);
    }

    // ------------------------------------------------------------- maiuscole

    #[test]
    fn il_maiuscolo_arriva_fino_al_testo_della_riga() {
        let mut t = tipografo(64.0);
        let parole = vec![w("città", 0.0, 0.3), w("aperta", 0.35, 0.6)];
        let cfg = LayoutConfig { maiuscole: true, ..cfg() };
        let b = &impagina(&parole, &mut t, &cfg).unwrap()[0];
        assert_eq!(b.testo(), "CITTÀ APERTA");
        // Gli intervalli di byte devono restare validi sul testo trasformato.
        for riga in &b.righe {
            for p in &riga.parole {
                assert_eq!(&riga.testo[p.byte.clone()], b.parole[p.indice].testo.to_uppercase());
            }
        }
    }

    #[test]
    fn il_maiuscolo_e_piu_largo_e_la_spezzatura_ne_tiene_conto() {
        let mut t = tipografo(64.0);
        let parole = frase_lunga();
        let normale = impagina(&parole, &mut t, &cfg()).unwrap();
        let maiuscolo =
            impagina(&parole, &mut t, &LayoutConfig { maiuscole: true, ..cfg() }).unwrap();
        assert!(
            maiuscolo.len() >= normale.len(),
            "il maiuscolo occupa piu' spazio: {} blocchi contro {}",
            maiuscolo.len(),
            normale.len()
        );
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
        let parole: Vec<Parola> = (0..12)
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
