//! Verba — sottotitoli automatici in locale, dalla riga di comando.
//!
//! Tre comandi, uno per ciascuna cosa che si puo' volere:
//!
//! ```text
//! verba trascrivi discorso.mp3 --out sottotitoli.srt --lingua it --termini glossario.csv
//! verba rendi     filmato.mp4  --out filmato_sub.mp4 --preset orizzontale.json
//! verba overlay   filmato.mp4  --out overlay.mov     --preset verticale.json
//! ```
//!
//! Tutti e tre percorrono la stessa pipeline; cambia solo cosa ne esce.
//!
//! ```text
//!   file (audio o video) ──► audio.rs  ─► PCM mono 16 kHz normalizzato (RAM)
//!                                │
//!                                ├─► pyannote ONNX     ─► segmenti di parlato
//!                                │      (rilasciato)
//!                                ├─► Whisper large-v3  ─► testo per segmento
//!                                │      (SCARICATO dalla GPU)
//!                                ├─► wav2vec2 ONNX CTC ─► tempi per parola
//!                                ├─► pulizia.rs        ─► sequenza normalizzata
//!                                ├─► layout.rs         ─► blocchi e righe
//!                                │                        (misura cosmic-text)
//!                                ├─► srt.rs            ─► trascrivi: .srt .vtt .json .txt
//!                                └─► render.rs + encoder.cpp
//!                                                      ─► rendi:   video impresso
//!                                                         overlay: sfondo trasparente
//! ```
//!
//! Niente file intermedi: la trascrizione resta in RAM e alimenta
//! direttamente l'impaginazione e il disegno.

mod aspetto;
mod avanzamento;
mod elenchi;
mod modelli;
mod opzioni;
mod uscite;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use verba_core::audio::{AudioInput, NormalizeMode, PreprocessConfig, Pcm};
use verba_core::eventi::{Fase, Progresso};
use verba_core::layout::{Blocco, LayoutConfig};
use verba_core::media::{Informazioni, Media};
use verba_core::pipeline::{self, ConfigTrascrizione};
use verba_core::prompt::{self, PromptConfig};
use verba_core::render::Rasterizzatore;
use verba_core::scena::Scena;
use verba_core::segmentation::SegmentationConfig;
use verba_core::trascrizione::Parola;
use verba_core::transcribe::WhisperConfig;
use verba_core::video::{self, Sfondo, VideoConfig};
use verba_core::{align, audio, gpu, layout};

use aspetto::{DateAMano, Vincolo};
use opzioni::{Aspetto, Codifica, Comuni, NormalizzaArg, UsciteTestuali};

#[derive(Parser, Debug)]
#[command(
    name = "verba",
    about = "Sottotitoli automatici in locale: trascrizione parola per parola, \
             sottotitoli di testo, video sottotitolato e overlay trasparente",
    long_about = None,
    version,
    subcommand_required = true,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    comando: Comando,

    #[command(flatten)]
    globali: Globali,
}

/// Le opzioni che valgono per ogni comando e si possono scrivere ovunque.
#[derive(Args, Debug, Clone)]
struct Globali {
    /// Scrive l'avanzamento in JSON su stderr, un oggetto per riga: e' la
    /// forma da usare quando Verba sta dentro un altro script.
    #[arg(long, global = true, conflicts_with = "progresso")]
    json: bool,

    /// Come mostrare l'avanzamento delle fasi su stderr.
    #[arg(long, global = true, value_enum, default_value_t = avanzamento::Formato::Testo,
          value_name = "MODO")]
    progresso: avanzamento::Formato,

    /// Verbosita' del log.
    #[arg(short, long, global = true)]
    verbose: bool,
}

impl Globali {
    fn formato_avanzamento(&self) -> avanzamento::Formato {
        if self.json {
            avanzamento::Formato::Json
        } else {
            self.progresso
        }
    }
}

#[derive(Subcommand, Debug)]
enum Comando {
    /// Trascrive e scrive i sottotitoli come file di testo.
    ///
    /// Il formato lo dice l'estensione di --out: .srt, .vtt, .json (parola per
    /// parola) o .txt. Si puo' ripetere --out per averne piu' d'uno in una
    /// passata sola.
    Trascrivi(ArgomentiTesto),

    /// Imprime i sottotitoli sul filmato di partenza.
    ///
    /// L'estensione di --out sceglie il codec: .mp4 per H.264 (si riproduce
    /// ovunque), .mov per ProRes 422 HQ (senza perdita, per chi rimonta). La
    /// traccia audio del sorgente viene ricopiata senza ricodifica.
    Rendi(ArgomentiVideo),

    /// Disegna i soli sottotitoli su sfondo trasparente, da sovrapporre in
    /// montaggio.
    ///
    /// L'estensione di --out sceglie il codec: .mov per ProRes 4444, .webm
    /// per VP9 con alfa (centinaia di volte piu' leggero, piu' lento da
    /// produrre).
    Overlay(ArgomentiVideo),

    /// Elenca i caratteri disponibili con i loro pesi.
    Caratteri(ArgomentiCaratteri),

    /// Elenca i preset di serie.
    Preset,

    /// Elenca i formati di uscita.
    Formati,

    /// Elenca, scarica e verifica i modelli.
    ///
    /// Senza opzioni dice cosa c'e' e cosa manca. I modelli non sono dentro
    /// l'eseguibile: si scaricano una volta e restano nella cartella dati.
    Modelli(ArgomentiModelli),
}

#[derive(Args, Debug)]
struct ArgomentiModelli {
    /// Scarica quello che manca, riprendendo uno scaricamento interrotto.
    #[arg(long)]
    scarica: bool,

    /// Ricalcola l'impronta SHA-256 di quello che c'e'.
    #[arg(long)]
    verifica: bool,

    /// Cancella un modello. Ripetibile.
    #[arg(long, value_name = "ID")]
    rimuovi: Vec<String>,

    /// Per quale dimensione del modello di trascrizione.
    #[arg(long, value_enum, default_value_t = opzioni::DimensioneArg::LargeV3,
          value_name = "DIMENSIONE")]
    modello: opzioni::DimensioneArg,

    /// Cartella dei modelli (default: la cartella dati di Verba).
    #[arg(long, value_name = "CARTELLA")]
    cartella_modelli: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct ArgomentiTesto {
    /// Sorgenti: qualsiasi formato audio o video, `-` per stdin. Piu'
    /// sorgenti vengono concatenate.
    #[arg(num_args = 1.., required = true, value_name = "FILE")]
    input: Vec<String>,

    /// File da scrivere; l'estensione decide il formato. Ripetibile.
    /// Senza --out si scrive un .srt accanto al sorgente.
    #[arg(short, long, value_name = "FILE")]
    out: Vec<PathBuf>,

    /// Struttura dei sottotitoli. `blocchi` ricalca esattamente le righe che
    /// comparirebbero nel video; le altre derivano dalle sole parole.
    #[arg(long, alias = "srt-mode", value_enum,
          default_value_t = opzioni::SrtStrutturaArg::Blocchi)]
    srt_struttura: opzioni::SrtStrutturaArg,

    /// Caratteri massimi per battuta nelle strutture `riga` e `karaoke`.
    #[arg(long, alias = "srt-max-chars", default_value_t = 84, value_name = "N")]
    srt_caratteri_max: usize,

    #[command(flatten)]
    comuni: Comuni,

    #[command(flatten)]
    aspetto: Aspetto,
}

#[derive(Args, Debug)]
struct ArgomentiVideo {
    /// Sorgenti: qualsiasi formato audio o video, `-` per stdin.
    #[arg(num_args = 1.., required = true, value_name = "FILE")]
    input: Vec<String>,

    /// File video da scrivere; l'estensione decide il codec.
    #[arg(short, long, value_name = "FILE")]
    out: Option<PathBuf>,

    #[command(flatten)]
    codifica: Codifica,

    #[command(flatten)]
    testuali: UsciteTestuali,

    #[command(flatten)]
    comuni: Comuni,

    #[command(flatten)]
    aspetto: Aspetto,
}

#[derive(Args, Debug)]
struct ArgomentiCaratteri {
    /// Cartella con altri caratteri da aggiungere all'elenco. Ripetibile.
    #[arg(long, value_name = "CARTELLA")]
    cartella_caratteri: Vec<PathBuf>,

    /// Elenca anche i caratteri installati sul sistema.
    #[arg(long)]
    caratteri_di_sistema: bool,
}

fn main() -> Result<()> {
    // Si passa dagli ArgMatches invece che da `Cli::parse()` per poter
    // distinguere un'opzione scritta a mano da una lasciata al valore
    // predefinito: senza quella distinzione un preset verrebbe sempre
    // sovrascritto dai valori di clap.
    let matches = Cli::command().get_matches();
    let cli = Cli::from_arg_matches(&matches).map_err(|e| e.exit()).unwrap();
    let sub = matches.subcommand().map(|(_, m)| m.clone()).unwrap_or_default();
    let date = DateAMano(sub);

    let livello = if cli.globali.verbose { "debug" } else { "info" };
    // Il log va su stderr insieme all'avanzamento: su stdout resta solo cio'
    // che il comando ha il compito di stampare, cosi' `verba ... > file` e le
    // pipe funzionano come ci si aspetta.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new(format!("verba={livello},verba_core={livello},warn"))
        }))
        .with_target(false)
        .init();

    match cli.comando {
        Comando::Caratteri(a) => {
            elenchi::caratteri(&a.cartella_caratteri, a.caratteri_di_sistema);
            Ok(())
        }
        Comando::Preset => {
            elenchi::preset();
            Ok(())
        }
        Comando::Formati => {
            elenchi::formati();
            Ok(())
        }
        Comando::Modelli(a) => comando_modelli(a, &cli.globali),
        Comando::Trascrivi(a) => trascrivi(a, &date, &cli.globali),
        Comando::Rendi(a) => video(a, &date, &cli.globali, Uso::Rendi),
        Comando::Overlay(a) => video(a, &date, &cli.globali, Uso::Overlay),
    }
}

// ----------------------------------------------------------------- modelli

fn comando_modelli(a: ArgomentiModelli, globali: &Globali) -> Result<()> {
    let cartella = a.cartella_modelli.clone().unwrap_or_else(verba_core::cartelle::modelli);
    let dimensione: verba_core::modelli::Dimensione = a.modello.into();

    if !a.rimuovi.is_empty() {
        return modelli::rimuovi(&cartella, &a.rimuovi);
    }
    if a.verifica {
        return modelli::verifica(&cartella);
    }
    if a.scarica {
        let progresso = canale(globali);
        let a_mano = verba_core::modelli::scarica_mancanti(&cartella, dimensione, &progresso)?;
        for m in a_mano {
            println!(
                "\n{} non si scarica.\n{}",
                m.nome,
                verba_core::modelli::istruzioni_a_mano(m)
            );
        }
        println!();
    }
    modelli::elenca(&cartella, dimensione);
    Ok(())
}

/// Cosa si sta producendo. E' l'unica differenza fra `rendi` e `overlay`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Uso {
    Rendi,
    Overlay,
}

impl Uso {
    fn alfa(self) -> bool {
        self == Uso::Overlay
    }

    fn suffisso(self) -> &'static str {
        match self {
            Uso::Rendi => "_sub",
            Uso::Overlay => "_overlay",
        }
    }

    /// L'estensione proposta quando --out non c'e'.
    fn estensione(self) -> &'static str {
        match self {
            Uso::Rendi => "mp4",
            Uso::Overlay => "mov",
        }
    }

    fn estensioni_ammesse(self) -> &'static str {
        match self {
            Uso::Rendi => ".mp4 (H.264) o .mov (ProRes 422 HQ)",
            Uso::Overlay => ".mov (ProRes 4444) o .webm (VP9 con alfa)",
        }
    }
}

// --------------------------------------------------------------- trascrivi

fn trascrivi(a: ArgomentiTesto, date: &DateAMano, globali: &Globali) -> Result<()> {
    let richieste = uscite::richieste(&a.out, &a.input[0])?;
    let sorgente = apri_sorgente(&a.input)?;
    let progresso = canale(globali);
    let Some(elab) =
        elabora(&a.input, sorgente.as_ref(), &a.comuni, &a.aspetto, date, None, &progresso)?
    else {
        return Ok(());
    };

    let struttura = uscite::Struttura {
        struttura: a.srt_struttura,
        caratteri_max: a.srt_caratteri_max,
    };
    for (percorso, formato) in &richieste {
        uscite::scrivi(percorso, *formato, &elab.blocchi, &elab.parole, &struttura)?;
    }
    Ok(())
}

// ------------------------------------------------------------ rendi/overlay

fn video(a: ArgomentiVideo, date: &DateAMano, globali: &Globali, uso: Uso) -> Result<()> {
    // Il file di uscita si decide prima di trascrivere: un'estensione
    // sbagliata non deve emergere dopo mezz'ora di lavoro.
    let out = match &a.out {
        Some(p) => p.clone(),
        None => uscite::accanto(&a.input[0], uso.suffisso(), uso.estensione()),
    };
    let formato = opzioni::formato_da_estensione(&out, uso.alfa()).ok_or_else(|| {
        anyhow::anyhow!(
            "«{}»: estensione non adatta a `verba {}`. Estensioni ammesse: {}.",
            out.display(),
            if uso.alfa() { "overlay" } else { "rendi" },
            uso.estensioni_ammesse()
        )
    })?;

    // Anche il filmato di partenza si controlla adesso: chiedere di imprimere
    // i sottotitoli su un file audio e' un errore che deve costare un
    // secondo, non una trascrizione intera.
    let sorgente = apri_sorgente(&a.input)?;
    if uso == Uso::Rendi && !sorgente.as_ref().is_some_and(|i| i.e_video()) {
        bail!(
            "`verba rendi` imprime i sottotitoli sul filmato, e «{}» non e' un file video. \
             Da un file audio si puo' produrre solo un overlay: usa `verba overlay`.",
            a.input[0]
        );
    }

    let progresso = canale(globali);
    let Some(elab) =
        elabora(&a.input, sorgente.as_ref(), &a.comuni, &a.aspetto, date, Some(uso), &progresso)?
    else {
        return Ok(());
    };

    // Il frame rate viene dal file, se non e' stato chiesto a mano: un
    // overlay con un frame rate diverso da quello del filmato si sfalsa a
    // poco a poco.
    let (fps_num, fps_den) = if date.ha("fps") {
        aspetto::analizza_fps(&a.codifica.fps)?
    } else {
        match sorgente.as_ref().filter(|i| i.e_video() && i.fps_num > 0) {
            Some(i) => (i.fps_num, i.fps_den.max(1)),
            None => aspetto::analizza_fps(&a.codifica.fps)?,
        }
    };

    let thread = elab.thread;
    let vcfg = VideoConfig {
        formato,
        fps_num,
        fps_den,
        qualita: a.codifica.qualita,
        thread,
        durata: a.codifica.durata.unwrap_or(elab.durata_audio),
    };

    // I formati che imprimono i sottotitoli hanno bisogno del filmato sotto.
    let mut filmato = if formato.ha_alfa() {
        None
    } else {
        let percorso = PathBuf::from(&a.input[0]);
        Some((Media::apri(&percorso)?, percorso))
    };

    info!(
        file = %out.display(),
        formato = %formato.etichetta(),
        risoluzione = format!("{}x{}", elab.layout.larghezza, elab.layout.altezza),
        fps = format!("{fps_num}/{fps_den}"),
        durata = format!("{:.2} s", vcfg.durata),
        "codifica"
    );

    let mut scena = Scena::nuova(elab.blocchi, elab.rasterizzatore);
    let stat = {
        let _c = progresso.inizia(Fase::Codifica);
        let sfondo = match &mut filmato {
            Some((media, percorso)) => Sfondo::Filmato { media, percorso },
            None => Sfondo::Trasparente,
        };
        video::esporta(&mut scena, sfondo, &vcfg, &out, &progresso)?
    };
    info!(
        file = %out.display(),
        fotogrammi = stat.fotogrammi,
        disegnati = stat.fotogrammi_disegnati,
        secondi = format!("{:.2}", stat.secondi),
        "video scritto"
    );

    // Uscite accessorie, solo se richieste esplicitamente.
    let struttura = uscite::Struttura {
        struttura: a.testuali.srt_struttura,
        caratteri_max: a.testuali.srt_caratteri_max,
    };
    for (percorso, formato) in [
        (&a.testuali.srt, uscite::FormatoTesto::Srt),
        (&a.testuali.vtt, uscite::FormatoTesto::Vtt),
        (&a.testuali.txt, uscite::FormatoTesto::Txt),
        (&a.testuali.mappa, uscite::FormatoTesto::Json),
    ] {
        if let Some(p) = percorso {
            uscite::scrivi(p, formato, scena.blocchi(), &elab.parole, &struttura)?;
        }
    }

    gpu::log_vram(&elab.device, "finale");
    Ok(())
}

// ------------------------------------------------------------- la pipeline

/// Tutto cio' che serve dopo la trascrizione, per qualunque comando.
struct Elaborato {
    blocchi: Vec<Blocco>,
    rasterizzatore: Rasterizzatore,
    parole: Vec<Parola>,
    durata_audio: f64,
    layout: LayoutConfig,
    device: gpu::Device,
    thread: usize,
}

/// Il canale di avanzamento, con Ctrl-C gia' collegato.
///
/// Ctrl-C non uccide il processo: chiede alla pipeline di fermarsi al primo
/// punto utile, cosi' il file video parziale viene cancellato invece di
/// restare li' a sembrare un export riuscito.
fn canale(globali: &Globali) -> Progresso {
    let progresso = avanzamento::progresso(globali.formato_avanzamento());
    let interruttore = progresso.interruttore();
    if let Err(e) = ctrlc::set_handler(move || interruttore.annulla()) {
        warn!(errore = %e, "Ctrl-C non intercettato: l'interruzione sara' brusca");
    }
    progresso
}

/// Dal file alle righe impaginate.
///
/// Ritorna `None` quando un'opzione diagnostica ha gia' detto tutto quello che
/// c'era da dire (`--solo-prompt`, `--solo-audio`) e non c'e' altro da fare.
fn elabora(
    input: &[String],
    sorgente: Option<&Informazioni>,
    comuni: &Comuni,
    asp: &Aspetto,
    date: &DateAMano,
    uso: Option<Uso>,
    progresso: &Progresso,
) -> Result<Option<Elaborato>> {
    let thread = comuni
        .thread
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4))
        .clamp(1, 32);

    // Initial prompt: costruito per primo, cosi' un CSV malformato viene
    // segnalato prima di spendere tempo su decodifica e modelli.
    let initial_prompt = prompt::build(&PromptConfig {
        csv: comuni.termini.clone(),
        column: comuni.termini_colonna.clone(),
        delimiter: comuni.termini_delimitatore,
        preamble: comuni.termini_preambolo.clone(),
        free_text: comuni.prompt.clone(),
        max_chars: comuni.prompt_max_caratteri,
    })?;
    if comuni.solo_prompt {
        match &initial_prompt {
            Some(p) => println!("{p}"),
            None => println!("(nessun initial prompt configurato)"),
        }
        return Ok(None);
    }

    // I prerequisiti che non stanno dentro l'eseguibile — la libreria di ONNX
    // Runtime e i quattro modelli — si controllano adesso: dopo verrebbero
    // fuori a decodifica finita.
    verba_core::onnx::assicura_libreria()?;
    let percorsi = modelli::percorsi(comuni, progresso)?;

    // Le impostazioni grafiche vengono validate subito: un colore scritto male
    // non deve emergere dopo mezz'ora di trascrizione.
    let base = aspetto::preset_di_partenza(asp)?;
    if let Some(b) = &base {
        info!(preset = %b.nome, "aspetto caricato da preset");
    }
    // Chi imprime i sottotitoli sul filmato non sceglie le proporzioni: le
    // riceve. Chi ci fa un overlay sopra puo' sceglierle, ma se non
    // coincidono e' meglio saperlo.
    let vincolo = match (uso, sorgente.and_then(|i| i.risoluzione())) {
        (Some(Uso::Rendi), Some((l, h))) => Vincolo::Impresso(l, h),
        (Some(Uso::Overlay), Some((l, h))) => Vincolo::Sovrapposto(l, h),
        _ => Vincolo::Libero,
    };
    let layout_cfg = aspetto::configura_layout(
        asp,
        date,
        base.as_ref(),
        sorgente.and_then(|i| i.risoluzione()),
        &vincolo,
    )?;
    let stile = aspetto::configura_stile(asp, date, base.as_ref())?;

    // ---------------------------------------------------------------- fase 0
    // Pre-elaborazione audio: tutto in RAM, nessun file temporaneo.
    let pcm = {
        let _c = progresso.inizia(Fase::Preparazione);
        let inputs: Vec<AudioInput> = input.iter().map(|a| AudioInput::from_cli_arg(a)).collect();
        audio::load_and_preprocess(
            &inputs,
            &PreprocessConfig {
                normalize: match comuni.normalizza {
                    NormalizzaArg::Niente => NormalizeMode::None,
                    NormalizzaArg::Picco => NormalizeMode::Peak,
                    NormalizzaArg::Rms => NormalizeMode::Rms,
                },
                target_dbfs: comuni.dbfs_obiettivo,
                ffmpeg_fallback: !comuni.senza_ffmpeg,
                ..Default::default()
            },
        )?
    };
    if comuni.solo_audio {
        stampa_statistiche_audio(&pcm);
        return Ok(None);
    }

    // ---------------------------------------------------------------- fase 0b
    // Scelta del dispositivo: GPU se la VRAM *totale* raggiunge la soglia.
    let device = gpu::select(comuni.vram_minima_mib, comuni.cpu, comuni.gpu);
    info!(device = %device.describe(), "dispositivo di calcolo");

    // ------------------------------------------------------- fasi 1, 2 e 3
    // Rilevamento del parlato, trascrizione e allineamento: l'ordine e la
    // sequenza di caricamento e rilascio dei modelli stanno in verba-core,
    // gli stessi per la riga di comando e per l'applicazione.
    let cfg = ConfigTrascrizione {
        modelli: percorsi,
        segmentazione: SegmentationConfig {
            onset: comuni.soglia_attacco,
            offset: comuni.soglia_rilascio,
            ..Default::default()
        },
        whisper: WhisperConfig {
            language: comuni.lingua.clone(),
            beam_size: comuni.beam,
            threads: thread as i32,
            initial_prompt,
            ..Default::default()
        },
        allineamento: align::AlignConfig {
            durata_minima_parola: aspetto::durata_minima_parola(asp, date, base.as_ref()),
            ..Default::default()
        },
        thread,
        finestre_uniformi: comuni.senza_segmentazione.then_some(25.0),
    };
    let trascrizione = pipeline::trascrivi(&pcm, &device, &cfg, progresso)?;

    // ---------------------------------------------------------------- fase 4
    // Impaginazione: le parole diventano righe — una alla volta a schermo —
    // misurate sul font che verra' effettivamente disegnato.
    let (mut tipografo, esito) = aspetto::costruisci_tipografo(asp, date, base.as_ref(), &layout_cfg)?;
    if let Some(avviso) = esito.avviso() {
        warn!("{avviso}");
        progresso.avviso(avviso);
    }
    aspetto::salva_preset(asp, &layout_cfg, &stile, &esito)?;

    let blocchi = {
        let _c = progresso.inizia(Fase::Impaginazione);
        layout::impagina(trascrizione.parole(), &mut tipografo, &layout_cfg)?
    };
    info!(
        righe = blocchi.len(),
        parole = trascrizione.len(),
        righe_max = layout_cfg.righe_consentite(),
        corpo = format!(
            "{:.0} px ({:.1} % dell'altezza)",
            layout_cfg.corpo(),
            layout_cfg.corpo_percentuale()
        ),
        larghezza_utile = format!("{:.0} px", layout_cfg.larghezza_utile()),
        "impaginazione completata"
    );

    let rasterizzatore = Rasterizzatore::nuovo(tipografo, layout_cfg.clone(), stile);
    Ok(Some(Elaborato {
        blocchi,
        rasterizzatore,
        parole: trascrizione.parole().to_vec(),
        durata_audio: pcm.duration_secs(),
        layout: layout_cfg,
        device,
        thread,
    }))
}

/// Legge le caratteristiche del file di partenza.
///
/// Ritorna `None` per stdin e per gli ingressi multipli: li' non c'e' un file
/// solo di cui parlare, e la pre-elaborazione audio se la cava lo stesso.
fn apri_sorgente(input: &[String]) -> Result<Option<Informazioni>> {
    if input.len() != 1 || input[0] == "-" {
        return Ok(None);
    }
    let percorso = std::path::Path::new(&input[0]);
    if !percorso.is_file() {
        bail!("«{}» non esiste, o non e' un file", percorso.display());
    }
    let media =
        Media::apri(percorso).with_context(|| format!("apertura di {}", percorso.display()))?;
    let info = media.informazioni().clone();
    info!(
        file = %percorso.display(),
        modalita = ?info.modalita,
        descrizione = %info.descrizione(),
        "file di partenza"
    );
    Ok(Some(info))
}

fn stampa_statistiche_audio(pcm: &Pcm) {
    let peak = pcm.samples.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    let rms = (pcm.samples.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>()
        / pcm.samples.len().max(1) as f64)
        .sqrt();
    println!("durata      : {:.3} s", pcm.duration_secs());
    println!("campioni    : {}", pcm.samples.len());
    println!("sample rate : {} Hz (mono)", pcm.sample_rate);
    println!("picco       : {:.4} ({:.1} dBFS)", peak, 20.0 * peak.max(1e-9).log10());
    println!("RMS         : {:.4} ({:.1} dBFS)", rms, 20.0 * rms.max(1e-9).log10());
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use verba_core::encoder::FormatoVideo;

    #[test]
    fn la_riga_di_comando_e_coerente() {
        Cli::command().debug_assert();
    }

    #[test]
    fn i_tre_comandi_della_spec_si_analizzano() {
        // Sono le tre righe scritte nella spec, alla lettera.
        let a = Cli::try_parse_from([
            "verba", "trascrivi", "input.mp3", "--out", "sottotitoli.srt",
            "--lingua", "it", "--termini", "glossario.csv",
        ])
        .unwrap();
        match a.comando {
            Comando::Trascrivi(t) => {
                assert_eq!(t.input, ["input.mp3"]);
                assert_eq!(t.out, [PathBuf::from("sottotitoli.srt")]);
                assert_eq!(t.comuni.lingua, "it");
                assert_eq!(t.comuni.termini, Some(PathBuf::from("glossario.csv")));
            }
            _ => panic!("comando sbagliato"),
        }

        let b = Cli::try_parse_from([
            "verba", "rendi", "input.mp4", "--out", "video_sub.mp4",
            "--preset", "orizzontale.json",
        ])
        .unwrap();
        assert!(matches!(b.comando, Comando::Rendi(_)));

        let c = Cli::try_parse_from([
            "verba", "overlay", "input.mp4", "--out", "overlay.mov",
            "--preset", "verticale.json",
        ])
        .unwrap();
        assert!(matches!(c.comando, Comando::Overlay(_)));
    }

    #[test]
    fn json_vale_su_qualsiasi_comando_e_dopo_gli_argomenti() {
        for riga in [
            vec!["verba", "trascrivi", "a.mp3", "--json"],
            vec!["verba", "--json", "trascrivi", "a.mp3"],
            vec!["verba", "rendi", "a.mp4", "--json"],
            vec!["verba", "overlay", "a.mp4", "--json"],
        ] {
            let cli = Cli::try_parse_from(riga.clone()).unwrap_or_else(|e| panic!("{riga:?}: {e}"));
            assert_eq!(cli.globali.formato_avanzamento(), avanzamento::Formato::Json);
        }
    }

    #[test]
    fn l_estensione_dell_uscita_sceglie_il_codec() {
        let a = |p: &str, alfa| opzioni::formato_da_estensione(std::path::Path::new(p), alfa);
        assert_eq!(a("v.mp4", false), Some(FormatoVideo::H264));
        assert_eq!(a("v.mov", false), Some(FormatoVideo::Prores422));
        assert_eq!(a("v.mov", true), Some(FormatoVideo::Prores4444));
        assert_eq!(a("v.webm", true), Some(FormatoVideo::Vp9Alpha));
        // Un .webm senza alfa e un .mp4 con alfa non esistono in Verba.
        assert_eq!(a("v.webm", false), None);
        assert_eq!(a("v.mp4", true), None);
    }

    #[test]
    fn i_nomi_inglesi_di_prima_continuano_a_funzionare() {
        let cli = Cli::try_parse_from([
            "verba", "trascrivi", "a.mp3", "--language", "en", "--prompt-csv", "t.csv",
            "--threads", "8", "--srt-mode", "karaoke",
        ])
        .unwrap();
        match cli.comando {
            Comando::Trascrivi(t) => {
                assert_eq!(t.comuni.lingua, "en");
                assert_eq!(t.comuni.thread, Some(8));
                assert_eq!(t.srt_struttura, opzioni::SrtStrutturaArg::Karaoke);
            }
            _ => panic!("comando sbagliato"),
        }
    }

    #[test]
    fn senza_comando_non_si_parte() {
        assert!(Cli::try_parse_from(["verba"]).is_err());
        assert!(Cli::try_parse_from(["verba", "trascrivi"]).is_err());
    }
}
