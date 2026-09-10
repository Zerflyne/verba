//! AutoSubtitler — trascrizione audio con mappatura parola-per-parola e
//! sottotitoli grafici su sfondo trasparente.
//!
//! Pipeline a fasi, con un solo modello alla volta residente sul dispositivo.
//! La trascrizione non passa da un file intermedio: resta in RAM e alimenta
//! direttamente l'impaginazione e il disegno.
//!
//! ```text
//!   audio (qualsiasi formato) ──► audio.rs  ─► PCM mono 16 kHz normalizzato (RAM)
//!                                     │
//!                                     ├─► pyannote ONNX     ─► segmenti di parlato
//!                                     │      (rilasciato)
//!                                     ├─► Whisper large-v3  ─► testo per segmento
//!                                     │      (SCARICATO dalla GPU)
//!                                     ├─► wav2vec2 ONNX CTC ─► tempi per parola
//!                                     │                            (in RAM)
//!                                     ├─► layout.rs         ─► blocchi e righe
//!                                     │                        (misura cosmic-text)
//!                                     ├─► render.rs         ─► fotogrammi RGBA
//!                                     └─► encoder.cpp       ─► MOV ProRes 4444
//!                                                              con canale alfa
//! ```

mod avanzamento;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use verba_core::audio::{AudioInput, NormalizeMode, PreprocessConfig};
use verba_core::caratteri::{self, Catalogo, Richiesta};
use verba_core::eventi::Fase;
use verba_core::layout::{Allineamento, Attivazione, Formato, LayoutConfig, Tipografo};
use verba_core::pipeline::{self, ConfigTrascrizione, PercorsiModelli};
use verba_core::prompt::{self, PromptConfig};
use verba_core::render::{Colore, Evidenziazione, Rasterizzatore, Stile};
use verba_core::scena::Scena;
use verba_core::segmentation::SegmentationConfig;
use verba_core::srt::{SrtConfig, SrtMode};
use verba_core::transcribe::WhisperConfig;
use verba_core::video::{self, VideoConfig};
use verba_core::{align, audio, gpu, layout, srt};

#[derive(Parser, Debug)]
#[command(
    name = "verba",
    about = "Sottotitoli grafici a sfondo trasparente (ProRes 4444) da un file audio: Whisper large-v3 + pyannote (ONNX) + wav2vec2-italian (ONNX)",
    version
)]
struct Cli {
    /// Sorgenti audio: percorsi di qualsiasi formato, oppure `-` per stdin.
    /// Piu' sorgenti vengono concatenate.
    /// Non serve con le opzioni che si limitano a mostrare qualcosa
    /// (`--caratteri`, `--solo-prompt`).
    #[arg(num_args = 1..)]
    input: Vec<String>,

    /// File video di uscita, MOV con ProRes 4444 (default: <primo input>.mov).
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Esporta anche i sottotitoli in formato SRT (uscita accessoria).
    #[arg(long, value_name = "FILE")]
    srt: Option<PathBuf>,

    /// Struttura dell'SRT accessorio. `blocchi` ricalca esattamente cio' che
    /// compare nel video; le altre modalita' derivano dalle sole parole.
    #[arg(long, value_enum, default_value_t = SrtModeArg::Blocchi)]
    srt_mode: SrtModeArg,

    /// Caratteri massimi per battuta nelle modalita' `line` e `karaoke`.
    #[arg(long, default_value_t = 84)]
    srt_max_chars: usize,

    /// Esporta anche la mappatura parola-per-parola in JSON.
    #[arg(long)]
    json: Option<PathBuf>,

    /// Come mostrare l'avanzamento delle fasi su stderr. `json` scrive un
    /// oggetto per riga, ed e' la forma da usare quando Verba e' dentro un
    /// altro script.
    #[arg(long, value_enum, default_value_t = avanzamento::Formato::Testo)]
    progresso: avanzamento::Formato,

    // ---- modelli ----
    /// Modello Whisper large-v3 in formato GGML/GGUF (whisper.cpp).
    #[arg(long, default_value = "models/ggml-large-v3.bin")]
    whisper_model: PathBuf,

    /// Modello di segmentazione pyannote esportato in ONNX.
    #[arg(long, default_value = "models/pyannote-segmentation-3.0.onnx")]
    segmentation_model: PathBuf,

    /// Modello wav2vec2-italian (testa CTC) esportato in ONNX.
    #[arg(long, default_value = "models/wav2vec2-italian.onnx")]
    align_model: PathBuf,

    /// Vocabolario del tokenizer wav2vec2 (vocab.json).
    #[arg(long, default_value = "models/wav2vec2-italian.vocab.json")]
    align_vocab: PathBuf,

    // ---- dispositivo ----
    /// VRAM totale minima (MiB) perche' una GPU sia usata. Il criterio e' la
    /// memoria *totale*, non quella libera.
    #[arg(long, default_value_t = gpu::DEFAULT_MIN_VRAM_MIB)]
    min_vram_mib: u64,

    /// Forza un indice GPU specifico, saltando la selezione automatica.
    #[arg(long)]
    gpu_index: Option<u32>,

    /// Forza l'esecuzione su CPU.
    #[arg(long)]
    cpu: bool,

    /// Thread CPU per ONNX Runtime, whisper.cpp e l'encoder video.
    #[arg(long)]
    threads: Option<usize>,

    // ---- trascrizione ----
    /// Lingua ISO-639-1 (`auto` per il rilevamento automatico).
    #[arg(long, default_value = "it")]
    language: String,

    /// Ampiezza del beam search di Whisper.
    #[arg(long, default_value_t = 5)]
    beam_size: i32,

    /// Prompt iniziale libero, per orientare Whisper su stile e punteggiatura.
    /// Viene anteposto ai termini letti dal CSV.
    #[arg(long)]
    prompt: Option<String>,

    /// File CSV con le parole con cui inizializzare il modello (nomi propri,
    /// sigle, termini tecnici). Delimitatore, intestazione e virgolette sono
    /// riconosciuti automaticamente.
    #[arg(long, value_name = "FILE")]
    prompt_csv: Option<PathBuf>,

    /// Colonna del CSV da leggere: nome dell'intestazione oppure indice base 0
    /// (default: la prima colonna).
    #[arg(long, value_name = "NOME|INDICE")]
    prompt_column: Option<String>,

    /// Forza il delimitatore del CSV invece di dedurlo (es. ";").
    #[arg(long, value_name = "CARATTERE")]
    prompt_delimiter: Option<char>,

    /// Testo introduttivo posto davanti all'elenco dei termini.
    #[arg(long, value_name = "TESTO")]
    prompt_preamble: Option<String>,

    /// Lunghezza massima dell'initial prompt, in caratteri. Whisper accetta al
    /// massimo ~224 token di contesto: oltre il limite i termini in eccesso
    /// vengono scartati (a termine intero).
    #[arg(long, default_value_t = prompt::DEFAULT_MAX_CHARS)]
    prompt_max_chars: usize,

    /// Stampa l'initial prompt che verrebbe usato ed esce.
    #[arg(long)]
    solo_prompt: bool,

    // ---- audio ----
    /// Strategia di normalizzazione dell'ampiezza.
    #[arg(long, value_enum, default_value_t = NormalizeArg::Rms)]
    normalize: NormalizeArg,

    /// Target RMS in dBFS per la normalizzazione.
    #[arg(long, default_value_t = -20.0)]
    target_dbfs: f32,

    /// Disattiva il fallback su ffmpeg per i formati non gestiti da Symphonia.
    #[arg(long)]
    no_ffmpeg_fallback: bool,

    // ---- segmentazione ----
    /// Soglia di attivazione del parlato.
    #[arg(long, default_value_t = 0.50)]
    onset: f32,

    /// Soglia di disattivazione del parlato (isteresi).
    #[arg(long, default_value_t = 0.35)]
    offset: f32,

    /// Salta pyannote e usa finestre uniformi di N secondi.
    #[arg(long)]
    no_segmentation: bool,

    // ---- formato del video ----
    /// Proporzioni del fotogramma.
    #[arg(long, value_enum, default_value_t = FormatoArg::Verticale)]
    formato: FormatoArg,

    /// Risoluzione esplicita `LARGHEZZAxALTEZZA` (sovrascrive --formato).
    #[arg(long, value_name = "LxA")]
    risoluzione: Option<String>,

    /// Frame rate: intero (`30`), decimale (`29.97`) o frazione (`30000/1001`).
    #[arg(long, default_value = "30")]
    fps: String,

    /// Durata del video in secondi (default: durata dell'audio).
    #[arg(long, value_name = "SECONDI")]
    durata: Option<f64>,

    /// Quantizzatore ProRes: piu' basso = piu' qualita' e file piu' grande.
    #[arg(long, default_value_t = 4)]
    qualita: u32,

    // ---- tipografia e impaginazione ----
    /// Famiglia del carattere. Usa --caratteri per vedere quali ci sono.
    #[arg(long, default_value = caratteri::FAMIGLIA_PREDEFINITA)]
    carattere: String,

    /// Peso del carattere, da 100 a 900. Se la famiglia non ha quel peso viene
    /// usato il piu' vicino, e lo si dice.
    #[arg(long, default_value_t = caratteri::PESO_PREDEFINITO)]
    peso: u16,

    /// Un file `.ttf` o `.otf` da usare, senza doverlo installare. Ha la
    /// precedenza su --carattere.
    #[arg(long, value_name = "FILE")]
    font: Option<PathBuf>,

    /// Cartella con altri caratteri da aggiungere all'elenco. Ripetibile.
    #[arg(long, value_name = "CARTELLA")]
    cartella_caratteri: Vec<PathBuf>,

    /// Cerca anche fra i caratteri installati sul sistema, oltre a quelli di
    /// serie.
    #[arg(long)]
    caratteri_di_sistema: bool,

    /// Elenca i caratteri disponibili con i loro pesi ed esce.
    #[arg(long)]
    caratteri: bool,

    /// Corpo del font in pixel (default: 6,5 % del lato minore del fotogramma).
    #[arg(long, value_name = "PIXEL")]
    dimensione_font: Option<f32>,

    /// Distanza minima dai bordi del fotogramma, in frazione della dimensione
    /// corrispondente. E' un limite invalicabile: nessuna posizione fa uscire
    /// il testo di qui.
    #[arg(long, default_value_t = 0.05)]
    margine: f32,

    /// Larghezza della colonna di testo, in frazione della larghezza del
    /// fotogramma.
    #[arg(long, default_value_t = 0.80)]
    larghezza_massima: f32,

    /// Centro verticale del blocco, in frazione dell'altezza: 0 in alto,
    /// 1 in basso.
    #[arg(long, default_value_t = 0.82)]
    posizione_verticale: f32,

    /// Centro orizzontale della colonna, in frazione della larghezza.
    #[arg(long, default_value_t = 0.50)]
    posizione_orizzontale: f32,

    /// Posizione verticale per nome, comoda al posto di
    /// --posizione-verticale: alto = 18 %, centro = 50 %, basso = 82 %.
    #[arg(long, value_enum)]
    posizione: Option<PosizioneArg>,

    /// Righe che possono comparire insieme, da 1 a 3. Il valore predefinito e'
    /// una sola: piu' righe per volta rendono la lettura caotica.
    #[arg(long, default_value_t = 1)]
    righe_massime: usize,

    /// Allineamento delle righe dentro la colonna.
    #[arg(long, value_enum, default_value_t = AllineamentoArg::Centro)]
    allineamento: AllineamentoArg,

    /// Disegna il testo in maiuscolo.
    #[arg(long)]
    maiuscole: bool,

    /// Interlinea come multiplo del corpo. Con una riga sola determina
    /// l'altezza della fascia su cui il rettangolo viene centrato.
    #[arg(long, default_value_t = 1.18)]
    interlinea: f32,

    /// Durata massima di una riga, in secondi.
    #[arg(long, default_value_t = 5.0)]
    durata_blocco: f64,

    /// Una pausa piu' lunga di questo valore chiude la riga.
    #[arg(long, default_value_t = 0.7)]
    pausa_blocco: f64,

    /// Permanenza della riga dopo l'ultima parola, in secondi.
    #[arg(long, default_value_t = 0.30)]
    tenuta: f64,

    // ---- accensione dell'evidenziazione ----
    /// Quanto il rettangolo arriva prima dell'inizio nominale della parola.
    #[arg(long, default_value_t = 0.06, value_name = "SECONDI")]
    anticipo: f64,

    /// Tetto alla permanenza del rettangolo nella pausa che segue la parola:
    /// oltre questo silenzio il rettangolo si spegne e resta la sola riga.
    #[arg(long, default_value_t = 0.35, value_name = "SECONDI")]
    pausa_massima: f64,

    /// Permanenza del rettangolo dopo l'ultima parola della riga. Non puo'
    /// superare `--tenuta`, oltre la quale la riga sparisce.
    #[arg(long, default_value_t = 0.25, value_name = "SECONDI")]
    coda: f64,

    // ---- stile ----
    /// Colore del testo, `#RRGGBB` o `#RRGGBBAA`.
    #[arg(long, default_value = "#FFFFFF")]
    colore: String,

    /// Colore del testo della parola in corso. Con la forma a rettangolo di
    /// norma coincide con --colore: a indicare la parola e' il rettangolo
    /// dietro, non un cambio di colore.
    #[arg(long, default_value = "#FFFFFF")]
    colore_attivo: String,

    /// Forma con cui si segnala la parola in corso.
    #[arg(long, value_enum, default_value_t = EvidenziazioneArg::Rettangolo)]
    evidenziazione: EvidenziazioneArg,

    /// Colore della forma che segnala la parola in corso.
    #[arg(long, default_value = "#7C3AED")]
    colore_evidenziazione: String,

    /// Margine orizzontale del rettangolo oltre la parola, in frazione del corpo.
    #[arg(long, default_value_t = 0.18)]
    padding_evidenziazione: f32,

    /// Altezza del rettangolo, in frazione del corpo.
    #[arg(long, default_value_t = 1.12)]
    altezza_evidenziazione: f32,

    /// Raggio degli angoli del rettangolo, in frazione del corpo.
    #[arg(long, default_value_t = 0.20)]
    raggio_evidenziazione: f32,

    /// Spessore della sottolineatura, in frazione del corpo.
    #[arg(long, default_value_t = 0.10)]
    spessore_sottolineatura: f32,

    /// Colore del contorno del testo.
    #[arg(long, default_value = "#000000")]
    colore_bordo: String,

    /// Spessore del contorno del testo in pixel (0 = nessun contorno).
    #[arg(long, default_value_t = 0.0)]
    bordo: f32,

    /// Non segnalare in alcun modo la parola in corso. Equivale a
    /// --evidenziazione nessuna.
    #[arg(long)]
    senza_evidenziazione: bool,

    /// Non disegnare l'ombra sotto il testo. L'ombra e' accesa di default
    /// perche' tiene i sottotitoli leggibili anche sopra un'immagine chiara.
    #[arg(long)]
    senza_ombra: bool,

    /// Colore dell'ombra.
    #[arg(long, default_value = "#000000A0")]
    colore_ombra: String,

    /// Spostamento dell'ombra verso il basso, in frazione del corpo.
    #[arg(long, default_value_t = 0.05)]
    ombra_spostamento: f32,

    /// Sfocatura dell'ombra, in frazione del corpo.
    #[arg(long, default_value_t = 0.08)]
    ombra_sfocatura: f32,

    // ---- diagnostica ----
    /// Esegue solo la pre-elaborazione audio e stampa le statistiche
    /// (utile per verificare decodifica, downmix e normalizzazione senza
    /// caricare alcun modello).
    #[arg(long)]
    solo_audio: bool,

    /// Verbosita' del log.
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum NormalizeArg {
    None,
    Peak,
    Rms,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum SrtModeArg {
    Blocchi,
    Parola,
    Riga,
    Karaoke,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum FormatoArg {
    /// 9:16 verticale, 1080x1920.
    #[value(name = "9:16", alias = "verticale")]
    Verticale,
    /// 16:9 orizzontale, 1920x1080.
    #[value(name = "16:9", alias = "orizzontale")]
    Orizzontale,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum PosizioneArg {
    Alto,
    Centro,
    Basso,
}

impl PosizioneArg {
    /// La frazione di altezza su cui centrare il blocco.
    fn frazione(self) -> f32 {
        match self {
            PosizioneArg::Alto => 0.18,
            PosizioneArg::Centro => 0.50,
            PosizioneArg::Basso => 0.82,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum EvidenziazioneArg {
    Rettangolo,
    Sottolineatura,
    SoloColore,
    Nessuna,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum AllineamentoArg {
    Sinistra,
    Centro,
    Destra,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let default_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(format!("verba={default_level},verba_core={default_level},warn"))),
        )
        .with_target(false)
        .init();

    // Il motore emette eventi; qui si decide che aspetto prendono.
    let progresso = avanzamento::progresso(cli.progresso);

    // Ctrl-C non uccide il processo: chiede alla pipeline di fermarsi al primo
    // punto utile, cosi' il file video parziale viene cancellato invece di
    // restare li' a sembrare un export riuscito.
    let interruttore = progresso.interruttore();
    if let Err(e) = ctrlc::set_handler(move || interruttore.annulla()) {
        warn!(errore = %e, "Ctrl-C non intercettato: l'interruzione sara' brusca");
    }

    let threads = cli
        .threads
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4))
        .clamp(1, 32);

    // Initial prompt: costruito per primo, cosi' un CSV malformato viene
    // segnalato prima di spendere tempo su decodifica e modelli.
    let prompt_cfg = PromptConfig {
        csv: cli.prompt_csv.clone(),
        column: cli.prompt_column.clone(),
        delimiter: cli.prompt_delimiter,
        preamble: cli.prompt_preamble.clone(),
        free_text: cli.prompt.clone(),
        max_chars: cli.prompt_max_chars,
    };
    let initial_prompt = prompt::build(&prompt_cfg)?;

    if cli.caratteri {
        elenca_caratteri(&cli);
        return Ok(());
    }

    if cli.solo_prompt {
        match &initial_prompt {
            Some(p) => println!("{p}"),
            None => println!("(nessun initial prompt configurato)"),
        }
        return Ok(());
    }

    if cli.input.is_empty() {
        bail!("serve almeno un file da trascrivere (oppure `-` per leggere da stdin)");
    }

    // Le impostazioni grafiche vengono validate subito: un colore scritto male
    // non deve emergere dopo mezz'ora di trascrizione.
    let layout_cfg = configura_layout(&cli)?;
    let stile = configura_stile(&cli)?;
    let (fps_num, fps_den) = analizza_fps(&cli.fps)?;

    // ---------------------------------------------------------------- fase 0
    // Pre-elaborazione audio: tutto in RAM, nessun file temporaneo.
    let inputs: Vec<AudioInput> = cli.input.iter().map(|a| AudioInput::from_cli_arg(a)).collect();
    let pre_cfg = PreprocessConfig {
        normalize: match cli.normalize {
            NormalizeArg::None => NormalizeMode::None,
            NormalizeArg::Peak => NormalizeMode::Peak,
            NormalizeArg::Rms => NormalizeMode::Rms,
        },
        target_dbfs: cli.target_dbfs,
        ffmpeg_fallback: !cli.no_ffmpeg_fallback,
        ..Default::default()
    };
    let pcm = {
        let _c = progresso.inizia(Fase::Preparazione);
        audio::load_and_preprocess(&inputs, &pre_cfg)?
    };

    if cli.solo_audio {
        let peak = pcm.samples.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        let rms = (pcm.samples.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>()
            / pcm.samples.len() as f64)
            .sqrt();
        println!("durata      : {:.3} s", pcm.duration_secs());
        println!("campioni    : {}", pcm.samples.len());
        println!("sample rate : {} Hz (mono)", pcm.sample_rate);
        println!("picco       : {:.4} ({:.1} dBFS)", peak, 20.0 * peak.max(1e-9).log10());
        println!("RMS         : {:.4} ({:.1} dBFS)", rms, 20.0 * rms.max(1e-9).log10());
        return Ok(());
    }

    // ---------------------------------------------------------------- fase 0b
    // Scelta del dispositivo: GPU se la VRAM *totale* raggiunge la soglia.
    let device = gpu::select(cli.min_vram_mib, cli.cpu, cli.gpu_index);
    info!(device = %device.describe(), "dispositivo di calcolo");

    // ------------------------------------------------------- fasi 1, 2 e 3
    // Rilevamento del parlato, trascrizione e allineamento: l'ordine e la
    // sequenza di caricamento e rilascio dei modelli stanno in verba-core,
    // gli stessi per la riga di comando e per l'applicazione.
    let cfg_trascrizione = ConfigTrascrizione {
        modelli: PercorsiModelli {
            whisper: cli.whisper_model.clone(),
            segmentazione: cli.segmentation_model.clone(),
            allineamento: cli.align_model.clone(),
            vocabolario: cli.align_vocab.clone(),
        },
        segmentazione: SegmentationConfig {
            onset: cli.onset,
            offset: cli.offset,
            ..Default::default()
        },
        whisper: WhisperConfig {
            language: cli.language.clone(),
            beam_size: cli.beam_size,
            threads: threads as i32,
            initial_prompt: initial_prompt.clone(),
            ..Default::default()
        },
        allineamento: align::AlignConfig::default(),
        thread: threads,
        finestre_uniformi: cli.no_segmentation.then_some(25.0),
    };
    let trascrizione = pipeline::trascrivi(&pcm, &device, &cfg_trascrizione, &progresso)?;
    let parole = trascrizione.parole();

    // ---------------------------------------------------------------- fase 4
    // Impaginazione: le parole diventano righe — una alla volta a schermo —
    // misurate sul font che verra' effettivamente disegnato.
    let (tipografo, esito_carattere) = costruisci_tipografo(&cli, &layout_cfg)?;
    if let Some(avviso) = esito_carattere.avviso() {
        warn!("{avviso}");
        progresso.avviso(avviso);
    }
    let mut tipografo = tipografo;

    let blocchi = {
        let _c = progresso.inizia(Fase::Impaginazione);
        layout::impagina(parole, &mut tipografo, &layout_cfg)?
    };
    info!(
        righe = blocchi.len(),
        parole = parole.len(),
        righe_max = layout_cfg.righe_consentite(),
        corpo = format!(
            "{:.0} px ({:.1} % dell'altezza)",
            layout_cfg.corpo(),
            layout_cfg.corpo_percentuale()
        ),
        larghezza_utile = format!("{:.0} px", layout_cfg.larghezza_utile()),
        "impaginazione completata"
    );

    // ---------------------------------------------------------------- fase 5
    // Disegno e codifica: MOV ProRes 4444 con canale alfa.
    let out_path = output_path(&cli);
    let vcfg = VideoConfig {
        fps_num,
        fps_den,
        qualita: cli.qualita,
        thread: threads,
        durata: cli.durata.unwrap_or_else(|| pcm.duration_secs()),
    };
    info!(
        file = %out_path.display(),
        risoluzione = format!("{}x{}", layout_cfg.larghezza, layout_cfg.altezza),
        fps = format!("{fps_num}/{fps_den}"),
        durata = format!("{:.2} s", vcfg.durata),
        "codifica del video dei sottotitoli"
    );
    let rasterizzatore = Rasterizzatore::nuovo(tipografo, layout_cfg.clone(), stile);
    let mut scena = Scena::nuova(blocchi, rasterizzatore);
    let stat = {
        let _c = progresso.inizia(Fase::Codifica);
        video::esporta(&mut scena, &vcfg, &out_path, &progresso)?
    };
    info!(
        file = %out_path.display(),
        fotogrammi = stat.fotogrammi,
        disegnati = stat.fotogrammi_disegnati,
        secondi = format!("{:.2}", stat.secondi),
        "video scritto"
    );

    // ---------------------------------------------------------------- fase 6
    // Uscite accessorie, solo se richieste esplicitamente.
    if let Some(srt_path) = &cli.srt {
        let cues = match cli.srt_mode {
            SrtModeArg::Blocchi => srt::cues_da_blocchi(scena.blocchi()),
            altro => {
                let srt_cfg = SrtConfig {
                    mode: match altro {
                        SrtModeArg::Parola => SrtMode::Parola,
                        SrtModeArg::Karaoke => SrtMode::Karaoke,
                        _ => SrtMode::Riga,
                    },
                    max_chars: cli.srt_max_chars,
                    ..Default::default()
                };
                srt::build_cues(parole, &srt_cfg)
            }
        };
        std::fs::write(srt_path, srt::render(&cues))
            .with_context(|| format!("scrittura di {}", srt_path.display()))?;
        info!(file = %srt_path.display(), battute = cues.len(), "SRT scritto");
    }
    if let Some(json_path) = &cli.json {
        std::fs::write(json_path, srt::render_json(parole)?)
            .with_context(|| format!("scrittura di {}", json_path.display()))?;
        info!(file = %json_path.display(), parole = parole.len(), "mappatura JSON scritta");
    }

    gpu::log_vram(&device, "finale");
    Ok(())
}

/// Il catalogo dei caratteri secondo le opzioni date.
fn catalogo(cli: &Cli) -> Catalogo {
    let mut cartelle = caratteri::cartelle_predefinite();
    cartelle.extend(cli.cartella_caratteri.iter().cloned());
    Catalogo::nuovo(&cartelle, cli.caratteri_di_sistema)
}

/// Stampa i caratteri disponibili con i loro pesi.
fn elenca_caratteri(cli: &Cli) {
    let c = catalogo(cli);
    println!("Caratteri disponibili ({}):\n", c.famiglie().len());
    for f in c.famiglie() {
        let pesi: Vec<String> = f.pesi.iter().map(|p| p.to_string()).collect();
        println!(
            "  {:<28} {:<24} {}",
            f.nome,
            pesi.join(" "),
            if f.di_serie { "di serie" } else { "" }
        );
    }
    if !cli.caratteri_di_sistema {
        println!("\nCon --caratteri-di-sistema si aggiungono quelli installati sulla macchina.");
    }
    println!(
        "Per usarne un altro: scaricare il .ttf e passarlo con --font FILE, oppure metterlo in\n\
         una cartella e passarla con --cartella-caratteri CARTELLA."
    );
}

/// Sceglie il carattere e prepara il motore di composizione.
fn costruisci_tipografo(
    cli: &Cli,
    layout_cfg: &LayoutConfig,
) -> Result<(Tipografo, caratteri::Esito)> {
    let mut cat = catalogo(cli);
    let richiesta = Richiesta {
        famiglia: cli.carattere.clone(),
        peso: cli.peso,
        file: cli.font.clone(),
    };
    let esito = cat.risolvi(&richiesta)?;
    let tipografo =
        Tipografo::dal_catalogo(cat, &esito, layout_cfg.corpo(), layout_cfg.interlinea)
            .context("preparazione del carattere")?;
    Ok((tipografo, esito))
}

fn configura_layout(cli: &Cli) -> Result<LayoutConfig> {
    let formato = match cli.formato {
        FormatoArg::Verticale => Formato::Verticale,
        FormatoArg::Orizzontale => Formato::Orizzontale,
    };
    let (larghezza, altezza) = match &cli.risoluzione {
        Some(s) => analizza_risoluzione(s)?,
        None => formato.risoluzione(),
    };
    if !(0.0..0.45).contains(&cli.margine) {
        bail!("--margine deve stare fra 0 e 0,45 (ricevuto {})", cli.margine);
    }
    if !(0.05..=1.0).contains(&cli.larghezza_massima) {
        bail!(
            "--larghezza-massima deve stare fra 0,05 e 1 (ricevuto {})",
            cli.larghezza_massima
        );
    }
    for (nome, valore) in
        [("--posizione-verticale", cli.posizione_verticale), ("--posizione-orizzontale", cli.posizione_orizzontale)]
    {
        if !(0.0..=1.0).contains(&valore) {
            bail!("{nome} deve stare fra 0 e 1 (ricevuto {valore})");
        }
    }
    if !(1..=layout::RIGHE_MAX_CONSENTITE).contains(&cli.righe_massime) {
        bail!(
            "--righe-massime deve stare fra 1 e {} (ricevuto {})",
            layout::RIGHE_MAX_CONSENTITE,
            cli.righe_massime
        );
    }
    Ok(LayoutConfig {
        larghezza,
        altezza,
        margine: cli.margine,
        larghezza_max: cli.larghezza_massima,
        // --posizione, se c'e', ha la precedenza: e' la forma per nome della
        // stessa grandezza.
        posizione_verticale: cli
            .posizione
            .map(PosizioneArg::frazione)
            .unwrap_or(cli.posizione_verticale),
        posizione_orizzontale: cli.posizione_orizzontale,
        righe_max: cli.righe_massime,
        allineamento: match cli.allineamento {
            AllineamentoArg::Sinistra => Allineamento::Sinistra,
            AllineamentoArg::Centro => Allineamento::Centro,
            AllineamentoArg::Destra => Allineamento::Destra,
        },
        maiuscole: cli.maiuscole,
        dimensione_font: cli.dimensione_font,
        interlinea: cli.interlinea,
        durata_max: cli.durata_blocco,
        pausa_max: cli.pausa_blocco,
        tenuta: cli.tenuta,
        attivazione: Attivazione {
            anticipo: cli.anticipo.max(0.0),
            pausa_max: cli.pausa_massima.max(0.0),
            coda: cli.coda.max(0.0),
        },
    })
}

fn configura_stile(cli: &Cli) -> Result<Stile> {
    let leggi = |nome: &str, valore: &str| -> Result<Colore> {
        Colore::da_esadecimale(valore).map_err(|e| anyhow::anyhow!("{nome}: {e}"))
    };
    Ok(Stile {
        colore: leggi("--colore", &cli.colore)?,
        colore_attivo: leggi("--colore-attivo", &cli.colore_attivo)?,
        colore_evidenziazione: leggi("--colore-evidenziazione", &cli.colore_evidenziazione)?,
        colore_bordo: leggi("--colore-bordo", &cli.colore_bordo)?,
        bordo: cli.bordo.max(0.0),
        // --senza-evidenziazione e' la forma breve di --evidenziazione nessuna
        // e ha la precedenza.
        evidenziazione: if cli.senza_evidenziazione {
            Evidenziazione::Nessuna
        } else {
            match cli.evidenziazione {
                EvidenziazioneArg::Rettangolo => Evidenziazione::Rettangolo,
                EvidenziazioneArg::Sottolineatura => Evidenziazione::Sottolineatura,
                EvidenziazioneArg::SoloColore => Evidenziazione::SoloColore,
                EvidenziazioneArg::Nessuna => Evidenziazione::Nessuna,
            }
        },
        padding: cli.padding_evidenziazione.max(0.0),
        altezza: cli.altezza_evidenziazione.max(0.0),
        raggio: cli.raggio_evidenziazione.max(0.0),
        spessore_sottolineatura: cli.spessore_sottolineatura.max(0.0),
        ombra: !cli.senza_ombra,
        colore_ombra: leggi("--colore-ombra", &cli.colore_ombra)?,
        ombra_spostamento: cli.ombra_spostamento.max(0.0),
        ombra_sfocatura: cli.ombra_sfocatura.max(0.0),
    })
}

/// `LARGHEZZAxALTEZZA`, con dimensioni pari (ProRes lavora a blocchi 16x16).
fn analizza_risoluzione(s: &str) -> Result<(u32, u32)> {
    let (l, a) = s
        .split_once(['x', 'X', '*'])
        .with_context(|| format!("risoluzione «{s}»: formato atteso LARGHEZZAxALTEZZA"))?;
    let larghezza: u32 = l.trim().parse().with_context(|| format!("larghezza «{l}»"))?;
    let altezza: u32 = a.trim().parse().with_context(|| format!("altezza «{a}»"))?;
    if larghezza == 0 || altezza == 0 {
        bail!("risoluzione «{s}»: le dimensioni devono essere positive");
    }
    if larghezza % 2 != 0 || altezza % 2 != 0 {
        bail!("risoluzione «{s}»: larghezza e altezza devono essere pari");
    }
    Ok((larghezza, altezza))
}

/// Frame rate come frazione esatta.
///
/// I valori NTSC (23,976 / 29,97 / 59,94 …) sono scritture arrotondate di
/// frazioni con denominatore 1001: passarli come decimali produrrebbe una
/// deriva di alcuni fotogrammi all'ora, per cui vengono riconosciuti a parte.
fn analizza_fps(s: &str) -> Result<(u32, u32)> {
    let s = s.trim();
    if let Some((n, d)) = s.split_once('/') {
        let num: u32 = n.trim().parse().with_context(|| format!("numeratore «{n}»"))?;
        let den: u32 = d.trim().parse().with_context(|| format!("denominatore «{d}»"))?;
        if num == 0 || den == 0 {
            bail!("frame rate «{s}»: numeratore e denominatore devono essere positivi");
        }
        return Ok((num, den));
    }
    let valore: f64 = s.parse().with_context(|| format!("frame rate «{s}»"))?;
    if valore <= 0.0 {
        bail!("frame rate «{s}»: deve essere positivo");
    }
    for (decimale, num) in [(23.976, 24000), (29.97, 30000), (47.952, 48000), (59.94, 60000), (119.88, 120000)] {
        if (valore - decimale).abs() < 0.005 {
            return Ok((num, 1001));
        }
    }
    if (valore - valore.round()).abs() < 1e-9 {
        return Ok((valore.round() as u32, 1));
    }
    Ok(((valore * 1000.0).round() as u32, 1000))
}

fn output_path(cli: &Cli) -> PathBuf {
    cli.output.clone().unwrap_or_else(|| {
        let first = cli.input.first().map(String::as_str).unwrap_or("output");
        if first == "-" {
            PathBuf::from("output.mov")
        } else {
            PathBuf::from(first).with_extension("mov")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn il_frame_rate_ntsc_resta_una_frazione_esatta() {
        assert_eq!(analizza_fps("29.97").unwrap(), (30000, 1001));
        assert_eq!(analizza_fps("23.976").unwrap(), (24000, 1001));
        assert_eq!(analizza_fps("30").unwrap(), (30, 1));
        assert_eq!(analizza_fps("30000/1001").unwrap(), (30000, 1001));
        assert_eq!(analizza_fps("12.5").unwrap(), (12500, 1000));
        assert!(analizza_fps("0").is_err());
        assert!(analizza_fps("boh").is_err());
    }

    #[test]
    fn la_risoluzione_richiede_dimensioni_pari() {
        assert_eq!(analizza_risoluzione("1080x1920").unwrap(), (1080, 1920));
        assert_eq!(analizza_risoluzione(" 1920 X 1080 ").unwrap(), (1920, 1080));
        assert!(analizza_risoluzione("1081x1920").is_err());
        assert!(analizza_risoluzione("1080").is_err());
        assert!(analizza_risoluzione("0x0").is_err());
    }
}
