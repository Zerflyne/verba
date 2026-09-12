//! Utility condivise per ONNX Runtime (pyannote e wav2vec2).
//!
//! Due responsabilita' oltre alla matematica: trovare la libreria nativa di
//! ONNX Runtime, e dire quale provider di calcolo sta effettivamente girando.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

use anyhow::{bail, Context, Result};
use ort::execution_providers::{CPUExecutionProvider, CUDAExecutionProvider};
use ort::session::{builder::GraphOptimizationLevel, Session};
use tracing::{info, warn};

use crate::cartelle;
use crate::gpu::Device;

/// Il provider su cui ONNX Runtime sta effettivamente calcolando.
///
/// Non e' la stessa cosa del dispositivo *chiesto*: CUDA puo' essere
/// disponibile come scheda e mancare come libreria, e in quel caso Verba
/// ricade su CPU senza fermarsi. La barra di stato deve dire quello che
/// succede davvero, non quello che si sperava.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Provider {
    /// Nessuna sessione ONNX e' stata ancora creata.
    Ignoto,
    Cpu,
    Cuda,
}

impl Provider {
    pub fn etichetta(self) -> &'static str {
        match self {
            Provider::Ignoto => "non ancora determinato",
            Provider::Cpu => "CPU",
            Provider::Cuda => "GPU NVIDIA (CUDA)",
        }
    }
}

static PROVIDER: AtomicU8 = AtomicU8::new(0);

fn segna(p: Provider) {
    PROVIDER.store(
        match p {
            Provider::Ignoto => 0,
            Provider::Cpu => 1,
            Provider::Cuda => 2,
        },
        Ordering::Relaxed,
    );
}

/// Il provider attivo, per la barra di stato e per il log.
pub fn provider_attivo() -> Provider {
    match PROVIDER.load(Ordering::Relaxed) {
        1 => Provider::Cpu,
        2 => Provider::Cuda,
        _ => Provider::Ignoto,
    }
}

/// Il nome del file della libreria ONNX Runtime su questo sistema.
pub const fn nome_libreria() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "onnxruntime.dll"
    }
    #[cfg(target_os = "macos")]
    {
        "libonnxruntime.dylib"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "libonnxruntime.so"
    }
}

/// Trova la libreria di ONNX Runtime e la comunica a `ort`.
///
/// Va chiamata una volta all'avvio, prima di qualunque sessione. L'ordine di
/// ricerca ha una logica sola: prima quello che ha deciso chi lancia il
/// programma, poi quello che viaggia col programma, poi il sistema.
///
/// 1. `ORT_DYLIB_PATH`, se indica un file che esiste;
/// 2. accanto all'eseguibile, e in `lib/` e `../lib/` — e' cosi' che sono
///    fatti un `.AppImage`, uno zip di Windows e un `.deb`;
/// 3. nella cartella dati dell'utente (`verba/lib`), dove finisce se e' stata
///    scaricata;
/// 4. le cartelle di sistema.
///
/// Se non la trova, l'errore dice cosa manca e come rimediare: senza questo
/// controllo `ort` andrebbe in panico al primo modello, con un messaggio che
/// non aiuta nessuno.
pub fn assicura_libreria() -> Result<PathBuf> {
    let nome = nome_libreria();

    if let Some(indicata) = std::env::var_os("ORT_DYLIB_PATH") {
        let p = PathBuf::from(indicata);
        if p.is_file() {
            info!(file = %p.display(), "ONNX Runtime da ORT_DYLIB_PATH");
            return Ok(p);
        }
        warn!(
            file = %p.display(),
            "ORT_DYLIB_PATH indica un file che non esiste: la cerco altrove"
        );
    }

    for cartella in cartelle_di_ricerca() {
        let candidato = cartella.join(nome);
        if candidato.is_file() {
            std::env::set_var("ORT_DYLIB_PATH", &candidato);
            info!(file = %candidato.display(), "ONNX Runtime trovata");
            return Ok(candidato);
        }
    }

    bail!(
        "{nome} non trovata.\n\
         ONNX Runtime serve per la segmentazione del parlato e per l'allineamento\n\
         delle parole, e non e' impacchettata dentro l'eseguibile.\n\
         Mettila accanto all'eseguibile, oppure in {},\n\
         oppure indicala con ORT_DYLIB_PATH=/percorso/{nome}.",
        cartelle::librerie().display()
    );
}

/// Le cartelle in cui cercare la libreria, nell'ordine.
fn cartelle_di_ricerca() -> Vec<PathBuf> {
    let mut cartelle = Vec::new();
    if let Some(exe) = cartelle::accanto_all_eseguibile() {
        cartelle.push(exe.clone());
        cartelle.push(exe.join("lib"));
        if let Some(su) = exe.parent() {
            // Un `.deb` mette il binario in `/usr/bin` e le risorse in
            // `/usr/lib/<nome del prodotto>`; un `.AppImage` monta la stessa
            // struttura sotto `$APPDIR/usr`.
            // `resources: ["lib/*"]` conserva il prefisso, quindi il file
            // finisce in `<prodotto>/lib/`: va guardato anche quello, non
            // solo la radice della cartella delle risorse.
            cartelle.push(su.join("lib"));
            for prodotto in ["verba", "Verba", "verba-app"] {
                let base = su.join("lib").join(prodotto);
                cartelle.push(base.join("lib"));
                cartelle.push(base);
            }
        }
    }
    cartelle.push(cartelle::librerie());
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        cartelle.push(PathBuf::from("/usr/local/lib"));
        cartelle.push(PathBuf::from("/usr/lib"));
        cartelle.push(PathBuf::from("/usr/lib/x86_64-linux-gnu"));
    }
    cartelle
}

/// Crea una sessione ONNX, con CUDA se e' davvero utilizzabile.
///
/// La GPU e' opzionale e la CPU e' il default: se il provider CUDA non si
/// registra — driver assenti, cuDNN di versione sbagliata, VRAM insufficiente
/// — la sessione viene ricostruita su CPU **senza errori bloccanti**, e viene
/// detto a voce alta su cosa si sta calcolando. Un'applicazione che si rifiuta
/// di partire senza CUDA e' un'applicazione che meta' delle persone non riesce
/// ad aprire.
pub fn build_session(model: &Path, device: &Device, intra_threads: usize) -> Result<Session> {
    let costruisci = |providers: Vec<ort::execution_providers::ExecutionProviderDispatch>| {
        Session::builder()
            .context("creazione del builder ONNX")?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(intra_threads.max(1))?
            .with_execution_providers(providers)?
            .commit_from_file(model)
            .with_context(|| format!("caricamento del modello ONNX {}", model.display()))
    };

    if let Some(idx) = device.cuda_index() {
        // `error_on_failure` e' cio' che rende la ricaduta *osservabile*:
        // senza, ort passerebbe alla CPU in silenzio e la barra di stato
        // continuerebbe a dire «GPU».
        let cuda = CUDAExecutionProvider::default().with_device_id(idx as i32).build().error_on_failure();
        match costruisci(vec![cuda, CPUExecutionProvider::default().build()]) {
            Ok(sessione) => {
                segna(Provider::Cuda);
                info!(modello = %model.display(), device = %device.describe(), "sessione ONNX pronta");
                return Ok(sessione);
            }
            Err(e) => {
                warn!(
                    errore = %e,
                    "CUDA non utilizzabile per ONNX Runtime: si continua su CPU (piu' lento, stesso risultato)"
                );
            }
        }
    }

    let sessione = costruisci(vec![CPUExecutionProvider::default().build()])?;
    segna(Provider::Cpu);
    info!(modello = %model.display(), device = "CPU", "sessione ONNX pronta");
    Ok(sessione)
}

/// Softmax numericamente stabile, in-place.
pub fn softmax(row: &mut [f32]) {
    let max = row.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    if !max.is_finite() {
        return;
    }
    let mut sum = 0.0f32;
    for v in row.iter_mut() {
        *v = (*v - max).exp();
        sum += *v;
    }
    if sum > 0.0 {
        for v in row.iter_mut() {
            *v /= sum;
        }
    }
}

/// log-softmax numericamente stabile, in-place.
pub fn log_softmax(row: &mut [f32]) {
    let max = row.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    if !max.is_finite() {
        return;
    }
    let sum: f32 = row.iter().map(|v| (*v - max).exp()).sum();
    let log_sum = sum.max(f32::MIN_POSITIVE).ln();
    for v in row.iter_mut() {
        *v = *v - max - log_sum;
    }
}

pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Riconosce se una riga contiene gia' probabilita' normalizzate (somma ~1 e
/// valori in [0,1]): serve a supportare export ONNX con o senza softmax finale.
pub fn looks_like_probabilities(row: &[f32]) -> bool {
    let sum: f32 = row.iter().sum();
    row.iter().all(|v| *v >= -1e-4 && *v <= 1.0 + 1e-4) && (sum - 1.0).abs() < 1e-2
}
