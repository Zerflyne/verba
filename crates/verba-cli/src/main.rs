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

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use verba_core::audio::{AudioInput, NormalizeMode, PreprocessConfig};
use verba_core::layout::{Attivazione, Formato, LayoutConfig, Posizione, Tipografo};
use verba_core::prompt::{self, PromptConfig};
use verba_core::render::{Colore, Rasterizzatore, Stile};
use verba_core::segmentation::{SegmentationConfig, Segmenter};
use verba_core::srt::{SrtConfig, SrtMode};
use verba_core::transcribe::{Transcriber, WhisperConfig};
use verba_core::video::{self, VideoConfig};
use verba_core::{align, audio, gpu, layout, segmentation, srt, FONT_INTER_BOLD};

#[derive(Parser, Debug)]
#[command(
    name = "verba",
    about = "Sottotitoli grafici a sfondo trasparente (ProRes 4444) da un file audio: Whisper large-v3 + pyannote (ONNX) + wav2vec2-italian (ONNX)",
    version
)]
struct Cli {
    /// Sorgenti audio: percorsi di qualsiasi formato, oppure `-` per stdin.
    /// Piu' sorgenti vengono concatenate.
    #[arg(required = true, num_args = 1..)]
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
    /// File `.ttf` da usare al posto di Inter 700, che e' incorporato.
    #[arg(long, value_name = "FILE")]
    font: Option<PathBuf>,

    /// Corpo del font in pixel (default: 6,5 % del lato minore del fotogramma).
    #[arg(long, value_name = "PIXEL")]
    dimensione_font: Option<f32>,

    /// Margine laterale fra testo e bordi, in frazione della larghezza.
    #[arg(long, default_value_t = 0.08)]
    margine: f32,

    /// Distanza dal bordo superiore o inferiore, in frazione dell'altezza.
    #[arg(long, default_value_t = 0.14)]
    margine_verticale: f32,

    /// Posizione verticale del blocco di sottotitoli.
    #[arg(long, value_enum, default_value_t = PosizioneArg::Basso)]
    posizione: PosizioneArg,

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

    /// Colore del rettangolo dietro la parola in corso.
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

    /// Colore del contorno del testo.
    #[arg(long, default_value = "#000000")]
    colore_bordo: String,

    /// Spessore del contorno del testo in pixel (0 = nessun contorno).
    #[arg(long, default_value_t = 0.0)]
    bordo: f32,

    /// Non disegnare il rettangolo di evidenziazione.
    #[arg(long)]
    senza_evidenziazione: bool,

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
    Word,
    Line,
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

    if cli.solo_prompt {
        match &initial_prompt {
            Some(p) => println!("{p}"),
            None => println!("(nessun initial prompt configurato)"),
        }
        return Ok(());
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
    let pcm = audio::load_and_preprocess(&inputs, &pre_cfg)?;

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
    gpu::log_vram(&device, "iniziale");

    // ---------------------------------------------------------------- fase 1
    // Segmentazione con pyannote. La sessione ONNX viene chiusa subito dopo,
    // per lasciare la VRAM libera a Whisper.
    let segments = if cli.no_segmentation {
        warn!("segmentazione disattivata: uso finestre uniformi da 25 s");
        segmentation::uniform_segments(pcm.duration_secs(), 25.0)
    } else {
        let seg_cfg = SegmentationConfig { onset: cli.onset, offset: cli.offset, ..Default::default() };
        let mut segmenter = Segmenter::new(&cli.segmentation_model, &device, seg_cfg, threads)
            .context("inizializzazione del segmentatore pyannote")?;
        let s = segmenter.run(&pcm)?;
        drop(segmenter);
        s
    };

    // ---------------------------------------------------------------- fase 2
    // Trascrizione con Whisper large-v3.
    let transcripts = if segments.is_empty() {
        warn!("nessun parlato rilevato");
        Vec::new()
    } else {
        let whisper_cfg = WhisperConfig {
            language: cli.language.clone(),
            beam_size: cli.beam_size,
            threads: threads as i32,
            initial_prompt: initial_prompt.clone(),
            ..Default::default()
        };
        let mut transcriber = Transcriber::new(&cli.whisper_model, &device, whisper_cfg)
            .context("inizializzazione di Whisper")?;
        let t = transcriber.run(&pcm, &segments)?;

        // *** Whisper viene scaricato dalla GPU PRIMA di caricare l'allineatore ***
        transcriber.release();
        drop(transcriber);
        t
    };

    // ---------------------------------------------------------------- fase 3
    // Allineamento forzato CTC con wav2vec2-italian: ora la GPU e' libera.
    // Il risultato resta in RAM: non passa da alcun file intermedio.
    let words = if transcripts.is_empty() {
        warn!("Whisper non ha prodotto testo: il video sara' interamente trasparente");
        Vec::new()
    } else {
        let mut aligner = align::Aligner::new(
            &cli.align_model,
            &cli.align_vocab,
            &device,
            align::AlignConfig::default(),
            threads,
        )
        .context("inizializzazione dell'allineatore wav2vec2")?;
        let w = aligner.run(&pcm, &transcripts)?;
        drop(aligner);
        w
    };

    // ---------------------------------------------------------------- fase 4
    // Impaginazione: le parole diventano righe — una alla volta a schermo —
    // misurate sul font che verra' effettivamente disegnato.
    let font = match &cli.font {
        Some(p) => std::fs::read(p).with_context(|| format!("lettura del font {}", p.display()))?,
        None => FONT_INTER_BOLD.to_vec(),
    };
    let mut tipografo = Tipografo::nuovo(&font, layout_cfg.corpo(), layout_cfg.interlinea)
        .context("caricamento del font")?;
    let blocchi = layout::impagina(&words, &mut tipografo, &layout_cfg)?;
    info!(
        righe = blocchi.len(),
        parole = words.len(),
        corpo = layout_cfg.corpo(),
        larghezza_utile = layout_cfg.larghezza_utile(),
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
    let mut rasterizzatore = Rasterizzatore::nuovo(tipografo, layout_cfg.clone(), stile);
    let stat = video::esporta(&blocchi, &mut rasterizzatore, &layout_cfg, &vcfg, &out_path)?;
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
            SrtModeArg::Blocchi => srt::cues_da_blocchi(&blocchi),
            altro => {
                let srt_cfg = SrtConfig {
                    mode: match altro {
                        SrtModeArg::Word => SrtMode::Word,
                        SrtModeArg::Karaoke => SrtMode::Karaoke,
                        _ => SrtMode::Line,
                    },
                    max_chars: cli.srt_max_chars,
                    ..Default::default()
                };
                srt::build_cues(&words, &srt_cfg)
            }
        };
        std::fs::write(srt_path, srt::render(&cues))
            .with_context(|| format!("scrittura di {}", srt_path.display()))?;
        info!(file = %srt_path.display(), battute = cues.len(), "SRT scritto");
    }
    if let Some(json_path) = &cli.json {
        std::fs::write(json_path, srt::render_json(&words)?)
            .with_context(|| format!("scrittura di {}", json_path.display()))?;
        info!(file = %json_path.display(), parole = words.len(), "mappatura JSON scritta");
    }

    gpu::log_vram(&device, "finale");
    Ok(())
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
    if !(0.0..0.45).contains(&cli.margine_verticale) {
        bail!("--margine-verticale deve stare fra 0 e 0,45 (ricevuto {})", cli.margine_verticale);
    }
    Ok(LayoutConfig {
        larghezza,
        altezza,
        margine_orizzontale: cli.margine,
        margine_verticale: cli.margine_verticale,
        posizione: match cli.posizione {
            PosizioneArg::Alto => Posizione::Alto,
            PosizioneArg::Centro => Posizione::Centro,
            PosizioneArg::Basso => Posizione::Basso,
        },
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
        colore_evidenziazione: leggi("--colore-evidenziazione", &cli.colore_evidenziazione)?,
        colore_bordo: leggi("--colore-bordo", &cli.colore_bordo)?,
        bordo: cli.bordo.max(0.0),
        evidenzia: !cli.senza_evidenziazione,
        padding: cli.padding_evidenziazione.max(0.0),
        altezza: cli.altezza_evidenziazione.max(0.0),
        raggio: cli.raggio_evidenziazione.max(0.0),
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
