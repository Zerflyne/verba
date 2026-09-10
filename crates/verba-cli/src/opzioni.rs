//! Le opzioni della riga di comando.
//!
//! Sono divise in tre gruppi condivisi fra i sottocomandi:
//!
//! - [`Comuni`]: modelli, dispositivo, trascrizione, audio. Servono a tutti e
//!   tre i comandi, perche' tutti e tre trascrivono.
//! - [`Aspetto`]: come i sottotitoli vengono composti e disegnati. Anche
//!   `trascrivi` ne ha bisogno: un SRT «a blocchi» ricalca esattamente le
//!   righe che comparirebbero nel video, e quelle dipendono dal carattere e
//!   dalla larghezza della colonna.
//! - [`Codifica`]: frame rate, durata e qualita' del file video.
//!
//! Qui ci sono solo le dichiarazioni; la stratificazione fra preset e opzioni
//! scritte a mano sta in [`crate::aspetto`].

use std::path::PathBuf;

use clap::{Args, ValueEnum};

use verba_core::caratteri;
use verba_core::encoder::FormatoVideo;
use verba_core::gpu;
use verba_core::modelli::Dimensione;
use verba_core::progetto::{self, Preset};
use verba_core::prompt;

// ---------------------------------------------------------------- enumerazioni

/// La dimensione del modello di trascrizione, sulla riga di comando.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum DimensioneArg {
    /// Il piu' rapido: 465 MB, per una bozza o per una macchina modesta.
    Small,
    /// A meta' strada: 1,4 GB.
    Medium,
    /// La qualita' di riferimento: 2,9 GB.
    #[value(name = "large-v3", alias = "large")]
    LargeV3,
}

impl From<DimensioneArg> for Dimensione {
    fn from(d: DimensioneArg) -> Self {
        match d {
            DimensioneArg::Small => Dimensione::Small,
            DimensioneArg::Medium => Dimensione::Medium,
            DimensioneArg::LargeV3 => Dimensione::LargeV3,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum NormalizzaArg {
    Niente,
    Picco,
    Rms,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum SrtStrutturaArg {
    /// Un blocco per riga mostrata: e' cio' che si vedrebbe nel video.
    Blocchi,
    /// Una battuta per parola.
    Parola,
    /// Battute riempite fino al limite di caratteri.
    Riga,
    /// Come `riga`, ma con la parola in corso marcata.
    Karaoke,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum FormatoArg {
    /// 9:16 verticale, 1080x1920.
    #[value(name = "9:16", alias = "verticale")]
    Verticale,
    /// 16:9 orizzontale, 1920x1080.
    #[value(name = "16:9", alias = "orizzontale")]
    Orizzontale,
    /// Le proporzioni del file di partenza, se e' un video.
    #[value(name = "dal-sorgente")]
    DalSorgente,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum PosizioneArg {
    Alto,
    Centro,
    Basso,
}

impl PosizioneArg {
    /// La frazione di altezza su cui centrare il blocco.
    pub fn frazione(self) -> f32 {
        match self {
            PosizioneArg::Alto => 0.18,
            PosizioneArg::Centro => 0.50,
            PosizioneArg::Basso => 0.82,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum PresetArg {
    Verticale,
    Orizzontale,
    Sobrio,
}

impl PresetArg {
    pub fn preset(self) -> Preset {
        match self {
            PresetArg::Verticale => progetto::verticale(),
            PresetArg::Orizzontale => progetto::orizzontale(),
            PresetArg::Sobrio => progetto::sobrio(),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum EvidenziazioneArg {
    Rettangolo,
    Sottolineatura,
    SoloColore,
    Nessuna,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum AllineamentoArg {
    Sinistra,
    Centro,
    Destra,
}

// ------------------------------------------------------------------- comuni

/// Modelli, dispositivo, trascrizione e audio: cio' che serve per arrivare
/// dalle onde alle parole. Uguale per tutti e tre i comandi.
#[derive(Args, Debug, Clone)]
pub struct Comuni {
    // ---- modelli ----
    /// Dimensione del modello di trascrizione. Piu' piccolo = piu' veloce e
    /// meno memoria, con qualche nome proprio in meno.
    #[arg(long, value_enum, default_value_t = DimensioneArg::LargeV3,
          value_name = "DIMENSIONE", help_heading = "Modelli")]
    pub modello: DimensioneArg,

    /// Cartella dei modelli (default: la cartella dati di Verba, oppure
    /// `./models` se ci si trova nel repository).
    #[arg(long, value_name = "CARTELLA", help_heading = "Modelli")]
    pub cartella_modelli: Option<PathBuf>,

    /// Scarica i modelli mancanti invece di fermarsi.
    #[arg(long, help_heading = "Modelli")]
    pub scarica_modelli: bool,

    /// Un file Whisper GGML/GGUF preciso, al posto di quello del catalogo.
    #[arg(long, alias = "whisper-model", value_name = "FILE", help_heading = "Modelli")]
    pub modello_whisper: Option<PathBuf>,

    /// Il modello di segmentazione pyannote in ONNX.
    #[arg(long, alias = "segmentation-model", value_name = "FILE", help_heading = "Modelli")]
    pub modello_segmentazione: Option<PathBuf>,

    /// Il modello wav2vec2-italian (testa CTC) in ONNX.
    #[arg(long, alias = "align-model", value_name = "FILE", help_heading = "Modelli")]
    pub modello_allineamento: Option<PathBuf>,

    /// Il vocabolario del tokenizer wav2vec2 (vocab.json).
    #[arg(long, alias = "align-vocab", value_name = "FILE", help_heading = "Modelli")]
    pub vocabolario_allineamento: Option<PathBuf>,

    // ---- dispositivo ----
    /// VRAM totale minima (MiB) perche' una GPU sia usata. Il criterio e' la
    /// memoria *totale*, non quella libera.
    #[arg(long, alias = "min-vram-mib", default_value_t = gpu::DEFAULT_MIN_VRAM_MIB,
          value_name = "MIB", help_heading = "Dispositivo")]
    pub vram_minima_mib: u64,

    /// Forza un indice GPU specifico, saltando la selezione automatica.
    #[arg(long, alias = "gpu-index", value_name = "INDICE", help_heading = "Dispositivo")]
    pub gpu: Option<u32>,

    /// Forza l'esecuzione su CPU.
    #[arg(long, help_heading = "Dispositivo")]
    pub cpu: bool,

    /// Thread CPU per ONNX Runtime, whisper.cpp e l'encoder video.
    #[arg(long, alias = "threads", value_name = "N", help_heading = "Dispositivo")]
    pub thread: Option<usize>,

    // ---- trascrizione ----
    /// Lingua ISO-639-1 (`auto` per il rilevamento automatico).
    #[arg(long, alias = "language", default_value = "it", help_heading = "Trascrizione")]
    pub lingua: String,

    /// Ampiezza del beam search di Whisper.
    #[arg(long, alias = "beam-size", default_value_t = 5, value_name = "N",
          help_heading = "Trascrizione")]
    pub beam: i32,

    /// Prompt iniziale libero, per orientare Whisper su stile e punteggiatura.
    /// Viene anteposto ai termini letti dal CSV.
    #[arg(long, value_name = "TESTO", help_heading = "Trascrizione")]
    pub prompt: Option<String>,

    /// File CSV con i termini noti (nomi propri, sigle, parole tecniche) con
    /// cui inizializzare il modello. Delimitatore, intestazione e virgolette
    /// sono riconosciuti automaticamente.
    #[arg(long, alias = "prompt-csv", value_name = "FILE", help_heading = "Trascrizione")]
    pub termini: Option<PathBuf>,

    /// Colonna del CSV da leggere: nome dell'intestazione oppure indice base 0
    /// (default: la prima colonna).
    #[arg(long, alias = "prompt-column", value_name = "NOME|INDICE",
          help_heading = "Trascrizione")]
    pub termini_colonna: Option<String>,

    /// Forza il delimitatore del CSV invece di dedurlo (es. ";").
    #[arg(long, alias = "prompt-delimiter", value_name = "CARATTERE",
          help_heading = "Trascrizione")]
    pub termini_delimitatore: Option<char>,

    /// Testo introduttivo posto davanti all'elenco dei termini.
    #[arg(long, alias = "prompt-preamble", value_name = "TESTO",
          help_heading = "Trascrizione")]
    pub termini_preambolo: Option<String>,

    /// Lunghezza massima dell'initial prompt, in caratteri. Whisper accetta al
    /// massimo ~224 token di contesto: oltre il limite i termini in eccesso
    /// vengono scartati (a termine intero).
    #[arg(long, alias = "prompt-max-chars", default_value_t = prompt::DEFAULT_MAX_CHARS,
          value_name = "N", help_heading = "Trascrizione")]
    pub prompt_max_caratteri: usize,

    // ---- audio ----
    /// Strategia di normalizzazione dell'ampiezza.
    #[arg(long, alias = "normalize", value_enum, default_value_t = NormalizzaArg::Rms,
          help_heading = "Audio")]
    pub normalizza: NormalizzaArg,

    /// Target RMS in dBFS per la normalizzazione.
    #[arg(long, alias = "target-dbfs", default_value_t = -20.0, value_name = "DBFS",
          help_heading = "Audio")]
    pub dbfs_obiettivo: f32,

    /// Disattiva il fallback su ffmpeg per i formati non gestiti da Symphonia.
    #[arg(long, alias = "no-ffmpeg-fallback", help_heading = "Audio")]
    pub senza_ffmpeg: bool,

    // ---- segmentazione ----
    /// Soglia di attivazione del parlato.
    #[arg(long, alias = "onset", default_value_t = 0.50, value_name = "0-1",
          help_heading = "Segmentazione")]
    pub soglia_attacco: f32,

    /// Soglia di disattivazione del parlato (isteresi).
    #[arg(long, alias = "offset", default_value_t = 0.60, value_name = "0-1",
          help_heading = "Segmentazione")]
    pub soglia_rilascio: f32,

    /// Salta pyannote e usa finestre uniformi di 25 secondi.
    #[arg(long, alias = "no-segmentation", help_heading = "Segmentazione")]
    pub senza_segmentazione: bool,

    // ---- diagnostica ----
    /// Stampa l'initial prompt che verrebbe usato ed esce.
    #[arg(long, help_heading = "Diagnostica")]
    pub solo_prompt: bool,

    /// Esegue solo la pre-elaborazione audio e stampa le statistiche, senza
    /// caricare alcun modello.
    #[arg(long, help_heading = "Diagnostica")]
    pub solo_audio: bool,
}

// ------------------------------------------------------------------ aspetto

/// Come i sottotitoli vengono composti e disegnati.
///
/// Ogni campo qui ha un corrispondente nel preset: e' l'insieme di cio' che si
/// puo' salvare e ricaricare.
#[derive(Args, Debug, Clone)]
pub struct Aspetto {
    // ---- preset ----
    /// Carica l'aspetto dei sottotitoli da un preset. Le opzioni scritte a
    /// mano hanno comunque la precedenza su cio' che il preset dice.
    #[arg(long, value_name = "FILE", help_heading = "Preset")]
    pub preset: Option<PathBuf>,

    /// Parte da uno dei preset di serie invece che dai valori predefiniti.
    #[arg(long, value_enum, conflicts_with = "preset", help_heading = "Preset")]
    pub preset_di_serie: Option<PresetArg>,

    /// Salva in un preset l'aspetto risultante da queste opzioni.
    #[arg(long, value_name = "FILE", help_heading = "Preset")]
    pub salva_preset: Option<PathBuf>,

    // ---- carattere ----
    /// Famiglia del carattere. `verba caratteri` elenca quelle disponibili.
    #[arg(long, default_value = caratteri::FAMIGLIA_PREDEFINITA, value_name = "NOME",
          help_heading = "Carattere")]
    pub carattere: String,

    /// Peso del carattere, da 100 a 900. Se la famiglia non ha quel peso viene
    /// usato il piu' vicino, e lo si dice.
    #[arg(long, default_value_t = caratteri::PESO_PREDEFINITO, value_name = "100-900",
          help_heading = "Carattere")]
    pub peso: u16,

    /// Un file `.ttf` o `.otf` da usare, senza doverlo installare. Ha la
    /// precedenza su --carattere.
    #[arg(long, value_name = "FILE", help_heading = "Carattere")]
    pub font: Option<PathBuf>,

    /// Cartella con altri caratteri da aggiungere all'elenco. Ripetibile.
    #[arg(long, value_name = "CARTELLA", help_heading = "Carattere")]
    pub cartella_caratteri: Vec<PathBuf>,

    /// Cerca anche fra i caratteri installati sul sistema, oltre a quelli di
    /// serie.
    #[arg(long, help_heading = "Carattere")]
    pub caratteri_di_sistema: bool,

    /// Corpo del font in pixel, riferito all'altezza del fotogramma prodotto
    /// (default: 6,5 % del lato minore).
    #[arg(long, value_name = "PIXEL", help_heading = "Carattere")]
    pub dimensione_font: Option<f32>,

    /// Disegna il testo in maiuscolo.
    #[arg(long, help_heading = "Carattere")]
    pub maiuscole: bool,

    /// Interlinea come multiplo del corpo. Con una riga sola determina
    /// l'altezza della fascia su cui il rettangolo viene centrato.
    #[arg(long, default_value_t = 1.18, value_name = "MULTIPLO", help_heading = "Carattere")]
    pub interlinea: f32,

    // ---- posizione ----
    /// Proporzioni del fotogramma. Con un file video il predefinito e'
    /// `dal-sorgente`; con un file audio, che non ha proporzioni, e' `9:16`.
    #[arg(long, value_enum, default_value_t = FormatoArg::DalSorgente,
          help_heading = "Posizione")]
    pub formato: FormatoArg,

    /// Risoluzione esplicita `LARGHEZZAxALTEZZA` (sovrascrive --formato).
    #[arg(long, value_name = "LxA", help_heading = "Posizione")]
    pub risoluzione: Option<String>,

    /// Distanza minima dai bordi del fotogramma, in frazione della dimensione
    /// corrispondente. E' un limite invalicabile: nessuna posizione fa uscire
    /// il testo di qui.
    #[arg(long, default_value_t = 0.05, value_name = "0-0,45", help_heading = "Posizione")]
    pub margine: f32,

    /// Larghezza della colonna di testo, in frazione della larghezza del
    /// fotogramma.
    #[arg(long, default_value_t = 0.80, value_name = "0-1", help_heading = "Posizione")]
    pub larghezza_massima: f32,

    /// Centro verticale del blocco, in frazione dell'altezza: 0 in alto,
    /// 1 in basso.
    #[arg(long, default_value_t = 0.82, value_name = "0-1", help_heading = "Posizione")]
    pub posizione_verticale: f32,

    /// Centro orizzontale della colonna, in frazione della larghezza.
    #[arg(long, default_value_t = 0.50, value_name = "0-1", help_heading = "Posizione")]
    pub posizione_orizzontale: f32,

    /// Posizione verticale per nome, comoda al posto di
    /// --posizione-verticale: alto = 18 %, centro = 50 %, basso = 82 %.
    #[arg(long, value_enum, help_heading = "Posizione")]
    pub posizione: Option<PosizioneArg>,

    /// Righe che possono comparire insieme, da 1 a 3. Il valore predefinito e'
    /// una sola: piu' righe per volta rendono la lettura caotica.
    #[arg(long, default_value_t = 1, value_name = "1-3", help_heading = "Posizione")]
    pub righe_massime: usize,

    /// Allineamento delle righe dentro la colonna.
    #[arg(long, value_enum, default_value_t = AllineamentoArg::Centro,
          help_heading = "Posizione")]
    pub allineamento: AllineamentoArg,

    // ---- tempi ----
    /// Durata massima di una riga, in secondi.
    #[arg(long, default_value_t = 5.0, value_name = "SECONDI", help_heading = "Tempi")]
    pub durata_blocco: f64,

    /// Una pausa piu' lunga di questo valore chiude la riga.
    #[arg(long, default_value_t = 0.7, value_name = "SECONDI", help_heading = "Tempi")]
    pub pausa_blocco: f64,

    /// Permanenza della riga dopo l'ultima parola, in secondi.
    #[arg(long, default_value_t = 0.30, value_name = "SECONDI", help_heading = "Tempi")]
    pub tenuta: f64,

    /// Quanto l'evidenziazione arriva prima dell'inizio nominale della parola.
    #[arg(long, default_value_t = 0.06, value_name = "SECONDI", help_heading = "Tempi")]
    pub anticipo: f64,

    /// Tetto alla permanenza dell'evidenziazione nella pausa che segue la
    /// parola: oltre questo silenzio si spegne e resta la sola riga.
    #[arg(long, default_value_t = 0.60, value_name = "SECONDI", help_heading = "Tempi")]
    pub pausa_massima: f64,

    /// Permanenza dell'evidenziazione dopo l'ultima parola della riga. Non
    /// puo' superare --tenuta, oltre la quale la riga sparisce.
    #[arg(long, default_value_t = 0.40, value_name = "SECONDI", help_heading = "Tempi")]
    pub coda: f64,

    /// Durata minima attribuita a una parola: sotto questa soglia
    /// l'evidenziazione lampeggerebbe.
    #[arg(long, default_value_t = verba_core::pulizia::DURATA_MINIMA_PAROLA,
          value_name = "SECONDI", help_heading = "Tempi")]
    pub durata_minima_parola: f64,

    // ---- stile ----
    /// Colore del testo, `#RRGGBB` o `#RRGGBBAA`.
    #[arg(long, default_value = "#FFFFFF", value_name = "#RRGGBB", help_heading = "Stile")]
    pub colore: String,

    /// Colore del testo della parola in corso. Con la forma a rettangolo di
    /// norma coincide con --colore: a indicare la parola e' il rettangolo
    /// dietro, non un cambio di colore.
    #[arg(long, default_value = "#FFFFFF", value_name = "#RRGGBB", help_heading = "Stile")]
    pub colore_attivo: String,

    /// Forma con cui si segnala la parola in corso.
    #[arg(long, value_enum, default_value_t = EvidenziazioneArg::Rettangolo,
          help_heading = "Stile")]
    pub evidenziazione: EvidenziazioneArg,

    /// Non segnalare in alcun modo la parola in corso. Equivale a
    /// --evidenziazione nessuna.
    #[arg(long, help_heading = "Stile")]
    pub senza_evidenziazione: bool,

    /// Colore della forma che segnala la parola in corso.
    #[arg(long, default_value = "#7C3AED", value_name = "#RRGGBB", help_heading = "Stile")]
    pub colore_evidenziazione: String,

    /// Margine orizzontale del rettangolo oltre la parola, in frazione del corpo.
    #[arg(long, default_value_t = 0.18, value_name = "FRAZIONE", help_heading = "Stile")]
    pub padding_evidenziazione: f32,

    /// Altezza del rettangolo, in frazione del corpo.
    #[arg(long, default_value_t = 1.12, value_name = "FRAZIONE", help_heading = "Stile")]
    pub altezza_evidenziazione: f32,

    /// Raggio degli angoli del rettangolo, in frazione del corpo.
    #[arg(long, default_value_t = 0.20, value_name = "FRAZIONE", help_heading = "Stile")]
    pub raggio_evidenziazione: f32,

    /// Spessore della sottolineatura, in frazione del corpo.
    #[arg(long, default_value_t = 0.10, value_name = "FRAZIONE", help_heading = "Stile")]
    pub spessore_sottolineatura: f32,

    /// Colore del contorno del testo.
    #[arg(long, default_value = "#000000", value_name = "#RRGGBB", help_heading = "Stile")]
    pub colore_bordo: String,

    /// Spessore del contorno del testo in pixel (0 = nessun contorno).
    #[arg(long, default_value_t = 0.0, value_name = "PIXEL", help_heading = "Stile")]
    pub bordo: f32,

    /// Non disegnare l'ombra sotto il testo. L'ombra e' accesa di default
    /// perche' tiene i sottotitoli leggibili anche sopra un'immagine chiara.
    #[arg(long, help_heading = "Stile")]
    pub senza_ombra: bool,

    /// Colore dell'ombra.
    #[arg(long, default_value = "#000000A0", value_name = "#RRGGBBAA", help_heading = "Stile")]
    pub colore_ombra: String,

    /// Spostamento dell'ombra verso il basso, in frazione del corpo.
    #[arg(long, default_value_t = 0.05, value_name = "FRAZIONE", help_heading = "Stile")]
    pub ombra_spostamento: f32,

    /// Sfocatura dell'ombra, in frazione del corpo.
    #[arg(long, default_value_t = 0.08, value_name = "FRAZIONE", help_heading = "Stile")]
    pub ombra_sfocatura: f32,
}

// ----------------------------------------------------------------- codifica

/// Frame rate, durata e qualita' del file video.
#[derive(Args, Debug, Clone)]
pub struct Codifica {
    /// Frame rate: intero (`30`), decimale (`29.97`) o frazione (`30000/1001`).
    /// Da un file video, se non lo si scrive, viene preso quello del sorgente.
    #[arg(long, default_value = "30", value_name = "FPS", help_heading = "Codifica")]
    pub fps: String,

    /// Durata del video in secondi (default: durata dell'audio).
    #[arg(long, value_name = "SECONDI", help_heading = "Codifica")]
    pub durata: Option<f64>,

    /// Quantizzatore per i ProRes, CRF per H.264 e VP9: in entrambi i casi
    /// piu' basso = piu' qualita' e file piu' grande. 0 = il valore
    /// consigliato per il formato.
    #[arg(long, default_value_t = 0, value_name = "N", help_heading = "Codifica")]
    pub qualita: u32,
}

// ------------------------------------------------------- uscite accessorie

/// I file di testo che si possono chiedere in piu' al comando video.
#[derive(Args, Debug, Clone)]
pub struct UsciteTestuali {
    /// Scrive anche i sottotitoli in SRT.
    #[arg(long, value_name = "FILE", help_heading = "Uscite accessorie")]
    pub srt: Option<PathBuf>,

    /// Scrive anche i sottotitoli in WebVTT.
    #[arg(long, value_name = "FILE", help_heading = "Uscite accessorie")]
    pub vtt: Option<PathBuf>,

    /// Scrive anche il solo testo, una battuta per riga.
    #[arg(long, value_name = "FILE", help_heading = "Uscite accessorie")]
    pub txt: Option<PathBuf>,

    /// Scrive anche la mappatura parola-per-parola in JSON.
    #[arg(long, alias = "json-parole", value_name = "FILE",
          help_heading = "Uscite accessorie")]
    pub mappa: Option<PathBuf>,

    /// Struttura dell'SRT accessorio.
    #[arg(long, alias = "srt-mode", value_enum, default_value_t = SrtStrutturaArg::Blocchi,
          help_heading = "Uscite accessorie")]
    pub srt_struttura: SrtStrutturaArg,

    /// Caratteri massimi per battuta nelle strutture `riga` e `karaoke`.
    #[arg(long, alias = "srt-max-chars", default_value_t = 84, value_name = "N",
          help_heading = "Uscite accessorie")]
    pub srt_caratteri_max: usize,
}

/// Il formato video di un comando, dedotto dall'estensione del file di uscita.
///
/// I due comandi accettano estensioni diverse perche' producono cose diverse:
/// `rendi` un video finito, `overlay` un file con il canale alfa.
pub fn formato_da_estensione(percorso: &std::path::Path, alfa: bool) -> Option<FormatoVideo> {
    let ext = percorso.extension()?.to_str()?.to_ascii_lowercase();
    match (ext.as_str(), alfa) {
        ("mp4", false) => Some(FormatoVideo::H264),
        ("mov", false) => Some(FormatoVideo::Prores422),
        ("mov", true) => Some(FormatoVideo::Prores4444),
        ("webm", true) => Some(FormatoVideo::Vp9Alpha),
        _ => None,
    }
}
