//! Pre-elaborazione audio, **interamente in RAM**: nessun file temporaneo viene
//! mai creato su disco.
//!
//! Responsabilita':
//!
//! 1. input misto: file su disco (qualsiasi container/codec supportato da
//!    Symphonia), stdin, o buffer di byte gia' in memoria; piu' sorgenti
//!    eterogenee vengono concatenate;
//! 2. decodifica in PCM f32 interleaved;
//! 3. downmix a mono (media aritmetica dei canali);
//! 4. ricampionamento a 16 kHz (sinc band-limited, rubato);
//! 5. normalizzazione: rimozione DC, guadagno RMS verso un target dBFS,
//!    tetto sul picco, clamp finale.
//!
//! L'unico fallback esterno (opzionale) e' ffmpeg, invocato in *pipe mode*
//! (`pipe:0` -> `pipe:1`): anche in quel caso i byte non toccano il disco.

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tracing::{debug, info, warn};

/// Frequenza di campionamento richiesta da Whisper, pyannote e wav2vec2.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Buffer PCM mono in virgola mobile, dominio [-1.0, 1.0].
#[derive(Debug, Clone)]
pub struct Pcm {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl Pcm {
    pub fn duration_secs(&self) -> f64 {
        self.samples.len() as f64 / self.sample_rate as f64
    }

    /// Estrae una finestra temporale (in secondi) clampata ai limiti del buffer.
    /// Restituisce una *slice*: nessuna copia, nessuna allocazione.
    pub fn slice_secs(&self, start: f64, end: f64) -> &[f32] {
        let sr = self.sample_rate as f64;
        let a = ((start.max(0.0) * sr).round() as usize).min(self.samples.len());
        let b = ((end.max(0.0) * sr).round() as usize).min(self.samples.len());
        if b <= a {
            return &[];
        }
        &self.samples[a..b]
    }
}

/// Sorgente di input. Volutamente eterogenea: la CLI puo' mescolare percorsi,
/// stdin e buffer gia' residenti in memoria nella stessa invocazione.
#[derive(Debug, Clone)]
pub enum AudioInput {
    Path(PathBuf),
    /// Contenuto letto da stdin (`-` sulla riga di comando).
    Stdin,
    /// Byte gia' in RAM; `hint` e' un suggerimento di estensione ("mp3", "wav"...).
    Bytes { data: Vec<u8>, hint: Option<String> },
}

impl AudioInput {
    /// Interpreta un argomento della CLI: `-` significa stdin.
    pub fn from_cli_arg(arg: &str) -> Self {
        if arg == "-" {
            AudioInput::Stdin
        } else {
            AudioInput::Path(PathBuf::from(arg))
        }
    }

    fn label(&self) -> String {
        match self {
            AudioInput::Path(p) => p.display().to_string(),
            AudioInput::Stdin => "<stdin>".to_string(),
            AudioInput::Bytes { hint, data } => {
                format!("<memoria:{} byte,{}>", data.len(), hint.as_deref().unwrap_or("?"))
            }
        }
    }
}

/// Strategia di normalizzazione dell'ampiezza.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizeMode {
    /// Nessun guadagno applicato (solo clamp di sicurezza).
    None,
    /// Porta il picco assoluto al `peak_ceiling_dbfs`.
    Peak,
    /// Porta l'RMS al `target_dbfs` e poi limita il picco (default).
    Rms,
}

#[derive(Debug, Clone)]
pub struct PreprocessConfig {
    pub target_sample_rate: u32,
    pub normalize: NormalizeMode,
    /// Target RMS in dBFS (tipico per ASR: -20 dBFS).
    pub target_dbfs: f32,
    /// Tetto per il picco dopo il guadagno, in dBFS (headroom anti-clipping).
    pub peak_ceiling_dbfs: f32,
    /// Rimuove la componente continua (offset DC) prima del guadagno.
    pub remove_dc: bool,
    /// Guadagno massimo applicabile, in dB: evita di amplificare il rumore di
    /// fondo di registrazioni quasi silenziose.
    pub max_gain_db: f32,
    /// Se Symphonia fallisce, riprova con ffmpeg in pipe (nessun file temporaneo).
    pub ffmpeg_fallback: bool,
}

impl Default for PreprocessConfig {
    fn default() -> Self {
        Self {
            target_sample_rate: TARGET_SAMPLE_RATE,
            normalize: NormalizeMode::Rms,
            target_dbfs: -20.0,
            peak_ceiling_dbfs: -1.0,
            remove_dc: true,
            max_gain_db: 30.0,
            ffmpeg_fallback: true,
        }
    }
}

/// Punto di ingresso del modulo: carica, decodifica, unifica e normalizza.
///
/// Piu' sorgenti vengono concatenate *dopo* il ricampionamento individuale,
/// cosi' file con sample rate diversi si fondono correttamente; la
/// normalizzazione e' invece globale, per non creare salti di volume tra parti.
pub fn load_and_preprocess(inputs: &[AudioInput], cfg: &PreprocessConfig) -> Result<Pcm> {
    if inputs.is_empty() {
        bail!("nessuna sorgente audio fornita");
    }

    let mut merged: Vec<f32> = Vec::new();

    for input in inputs {
        let label = input.label();
        let (bytes, hint) = read_source(input)?;
        debug!(source = %label, bytes = bytes.len(), "sorgente caricata in RAM");

        let decoded = match decode_to_mono(&bytes, hint.as_deref()) {
            Ok(d) => d,
            Err(e) if cfg.ffmpeg_fallback => {
                warn!(source = %label, error = %e, "Symphonia ha fallito, provo con ffmpeg in pipe");
                decode_with_ffmpeg(&bytes, cfg.target_sample_rate)?
            }
            Err(e) => return Err(e).with_context(|| format!("decodifica di {label} fallita")),
        };

        info!(
            source = %label,
            sample_rate = decoded.sample_rate,
            secondi = format!("{:.2}", decoded.duration_secs()),
            "decodificato"
        );

        let resampled = resample(decoded, cfg.target_sample_rate)?;
        merged.extend_from_slice(&resampled.samples);
    }

    if merged.is_empty() {
        bail!("l'audio decodificato e' vuoto");
    }

    let mut pcm = Pcm { samples: merged, sample_rate: cfg.target_sample_rate };
    normalize_in_place(&mut pcm.samples, cfg);

    info!(
        secondi = format!("{:.2}", pcm.duration_secs()),
        campioni = pcm.samples.len(),
        "pre-elaborazione completata (0 file temporanei)"
    );
    Ok(pcm)
}

/// Comodita': una sola sorgente con la configurazione di default.
pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Pcm> {
    load_and_preprocess(
        &[AudioInput::Path(path.as_ref().to_path_buf())],
        &PreprocessConfig::default(),
    )
}

// ---------------------------------------------------------------------------
// 1. lettura della sorgente (tutto in memoria)
// ---------------------------------------------------------------------------

fn read_source(input: &AudioInput) -> Result<(Vec<u8>, Option<String>)> {
    match input {
        AudioInput::Path(p) => {
            let data = std::fs::read(p).with_context(|| format!("lettura di {}", p.display()))?;
            let hint = p.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase());
            Ok((data, hint))
        }
        AudioInput::Stdin => {
            let mut data = Vec::new();
            std::io::stdin().lock().read_to_end(&mut data).context("lettura da stdin")?;
            Ok((data, None))
        }
        AudioInput::Bytes { data, hint } => Ok((data.clone(), hint.clone())),
    }
}

// ---------------------------------------------------------------------------
// 2 + 3. decodifica e downmix a mono
// ---------------------------------------------------------------------------

fn decode_to_mono(bytes: &[u8], ext_hint: Option<&str>) -> Result<Pcm> {
    // `Cursor<Vec<u8>>` implementa MediaSource: il flusso resta in RAM.
    let cursor = Cursor::new(bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), MediaSourceStreamOptions::default());

    let mut hint = Hint::new();
    if let Some(ext) = ext_hint {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions { enable_gapless: true, ..Default::default() },
            &MetadataOptions::default(),
        )
        .context("formato non riconosciuto da Symphonia")?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("nessuna traccia audio decodificabile nel container"))?;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("codec audio non supportato")?;

    let mut mono: Vec<f32> = Vec::new();
    let mut sample_rate: u32 = track.codec_params.sample_rate.unwrap_or(0);
    let mut interleaved: Option<SampleBuffer<f32>> = None;
    let mut decode_errors = 0usize;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(SymphoniaError::ResetRequired) => {
                // Cambio di layout dello stream: chiudiamo qui, il grosso e' gia' letto.
                warn!("reset dello stream richiesto: interrompo la decodifica");
                break;
            }
            Err(e) => return Err(e).context("lettura del pacchetto"),
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let spec = *audio_buf.spec();
                sample_rate = spec.rate;
                let channels = spec.channels.count().max(1);

                // (ri)alloca il buffer di appoggio solo quando cambia capienza
                let need = audio_buf.capacity() as u64;
                let buf = match interleaved.as_mut() {
                    Some(b) if b.capacity() as u64 >= need * channels as u64 => b,
                    _ => {
                        interleaved = Some(SampleBuffer::<f32>::new(need, spec));
                        interleaved.as_mut().unwrap()
                    }
                };
                buf.copy_interleaved_ref(audio_buf);

                // downmix: media aritmetica dei canali (preserva la fase, evita
                // il raddoppio di ampiezza della semplice somma)
                let data = buf.samples();
                let inv = 1.0 / channels as f32;
                mono.reserve(data.len() / channels);
                for frame in data.chunks_exact(channels) {
                    mono.push(frame.iter().sum::<f32>() * inv);
                }
            }
            // Pacchetti corrotti: saltali, non interrompere l'intera trascrizione.
            Err(SymphoniaError::DecodeError(e)) => {
                decode_errors += 1;
                if decode_errors <= 3 {
                    warn!(error = %e, "pacchetto corrotto ignorato");
                }
            }
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(e) => return Err(e).context("decodifica"),
        }
    }

    if decode_errors > 3 {
        warn!(totale = decode_errors, "pacchetti corrotti ignorati");
    }
    if mono.is_empty() {
        bail!("decodifica riuscita ma nessun campione prodotto");
    }
    if sample_rate == 0 {
        bail!("sample rate della sorgente sconosciuto");
    }

    Ok(Pcm { samples: mono, sample_rate })
}

/// Fallback: ffmpeg legge da `pipe:0` e scrive PCM f32 little-endian su
/// `pipe:1`. Nessun file temporaneo: solo due pipe anonime.
fn decode_with_ffmpeg(bytes: &[u8], target_sr: u32) -> Result<Pcm> {
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner", "-loglevel", "error",
            "-i", "pipe:0",
            "-vn",
            "-map", "a:0",
            "-ac", "1",
            "-ar", &target_sr.to_string(),
            "-f", "f32le",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("ffmpeg non disponibile nel PATH (fallback di decodifica)")?;

    // Scrittura su un thread: evita il deadlock quando ffmpeg riempie stdout
    // mentre noi stiamo ancora riempiendo stdin.
    let mut stdin = child.stdin.take().expect("stdin della pipe");
    let payload = bytes.to_vec();
    let writer = std::thread::spawn(move || {
        // BrokenPipe e' normale se ffmpeg smette di leggere in anticipo.
        let _ = stdin.write_all(&payload);
        let _ = stdin.flush();
    });

    let out = child.wait_with_output().context("attesa di ffmpeg")?;
    let _ = writer.join();

    if !out.status.success() {
        bail!("ffmpeg ha fallito: {}", String::from_utf8_lossy(&out.stderr).trim());
    }

    let samples: Vec<f32> = out
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();

    if samples.is_empty() {
        bail!("ffmpeg non ha prodotto campioni");
    }
    Ok(Pcm { samples, sample_rate: target_sr })
}

// ---------------------------------------------------------------------------
// 4. ricampionamento a 16 kHz
// ---------------------------------------------------------------------------

fn resample(pcm: Pcm, target_sr: u32) -> Result<Pcm> {
    if pcm.sample_rate == target_sr {
        return Ok(pcm);
    }

    use rubato::{
        Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
    };

    let ratio = target_sr as f64 / pcm.sample_rate as f64;
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    const CHUNK: usize = 8192;
    let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, CHUNK, 1)
        .context("inizializzazione del resampler")?;

    let mut out = Vec::with_capacity((pcm.samples.len() as f64 * ratio) as usize + CHUNK);
    let mut pos = 0usize;

    while pos + CHUNK <= pcm.samples.len() {
        let block = [&pcm.samples[pos..pos + CHUNK]];
        let done = resampler.process(&block, None).context("ricampionamento")?;
        out.extend_from_slice(&done[0]);
        pos += CHUNK;
    }

    // Coda: process_partial accetta un blocco piu' corto della chunk size.
    if pos < pcm.samples.len() {
        let tail = [&pcm.samples[pos..]];
        let done = resampler
            .process_partial(Some(&tail), None)
            .context("ricampionamento della coda")?;
        out.extend_from_slice(&done[0]);
    }
    // Svuota la latenza interna del filtro, altrimenti gli ultimi campioni
    // reali resterebbero nella pipeline.
    if let Ok(done) = resampler.process_partial::<&[f32]>(None, None) {
        out.extend_from_slice(&done[0]);
    }

    // Lo svuotamento aggiunge una coda di zeri lunga quanto un chunk: si
    // taglia alla lunghezza teorica. Il flusso NON va invece traslato: la
    // latenza del filtro e' gia' compensata internamente da rubato, e
    // scartare `output_delay()` campioni introdurrebbe una deriva negativa
    // di ~2,7 ms su tutti i timestamp (verificato dal test sull'impulso).
    let expected = (pcm.samples.len() as f64 * ratio).round() as usize;
    out.truncate(expected.min(out.len()));

    debug!(da = pcm.sample_rate, a = target_sr, campioni = out.len(), "ricampionato");
    Ok(Pcm { samples: out, sample_rate: target_sr })
}

// ---------------------------------------------------------------------------
// 5. normalizzazione
// ---------------------------------------------------------------------------

fn normalize_in_place(samples: &mut [f32], cfg: &PreprocessConfig) {
    if samples.is_empty() {
        return;
    }

    // Sostituisce eventuali NaN/Inf prodotti da decoder difettosi.
    for s in samples.iter_mut() {
        if !s.is_finite() {
            *s = 0.0;
        }
    }

    if cfg.remove_dc {
        let mean = samples.iter().map(|&s| s as f64).sum::<f64>() / samples.len() as f64;
        if mean.abs() > 1e-6 {
            let mean = mean as f32;
            for s in samples.iter_mut() {
                *s -= mean;
            }
            debug!(offset = mean, "offset DC rimosso");
        }
    }

    let peak = samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    if peak <= 1e-9 {
        warn!("audio silenzioso: normalizzazione saltata");
        return;
    }
    let rms = (samples.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>()
        / samples.len() as f64)
        .sqrt() as f32;

    let ceiling = db_to_lin(cfg.peak_ceiling_dbfs);
    let max_gain = db_to_lin(cfg.max_gain_db);

    let gain = match cfg.normalize {
        NormalizeMode::None => 1.0,
        NormalizeMode::Peak => ceiling / peak,
        NormalizeMode::Rms => {
            if rms <= 1e-9 {
                1.0
            } else {
                // Guadagno verso il target RMS, poi vincolato dal tetto sul
                // picco: preserva la dinamica senza clippare i transienti.
                (db_to_lin(cfg.target_dbfs) / rms).min(ceiling / peak)
            }
        }
    }
    .min(max_gain)
    .max(1e-6);

    if (gain - 1.0).abs() > 1e-3 {
        for s in samples.iter_mut() {
            *s *= gain;
        }
    }

    // Clamp finale: garanzia dura del dominio [-1, 1] per i modelli a valle.
    for s in samples.iter_mut() {
        *s = s.clamp(-1.0, 1.0);
    }

    info!(
        picco_db = format!("{:.1}", lin_to_db(peak)),
        rms_db = format!("{:.1}", lin_to_db(rms)),
        guadagno_db = format!("{:.1}", lin_to_db(gain)),
        "normalizzazione applicata"
    );
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn lin_to_db(lin: f32) -> f32 {
    if lin <= 1e-9 {
        -120.0
    } else {
        20.0 * lin.log10()
    }
}

/// Normalizzazione a media nulla e varianza unitaria richiesta dal feature
/// extractor di wav2vec2 (`do_normalize = true`). Lavora su una copia perche'
/// il buffer globale non va alterato.
pub fn zero_mean_unit_var(x: &[f32]) -> Vec<f32> {
    if x.is_empty() {
        return Vec::new();
    }
    let n = x.len() as f64;
    let mean = x.iter().map(|&v| v as f64).sum::<f64>() / n;
    let var = x.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / n;
    let inv_std = 1.0 / (var.sqrt() + 1e-7);
    x.iter().map(|&v| ((v as f64 - mean) * inv_std) as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_cambia_lunghezza_in_proporzione() {
        let pcm = Pcm { samples: vec![0.0; 48_000], sample_rate: 48_000 };
        let out = resample(pcm, 16_000).unwrap();
        assert_eq!(out.sample_rate, 16_000);
        assert_eq!(out.samples.len(), 16_000, "lunghezza attesa esatta dopo la compensazione");
    }

    #[test]
    fn resample_preserva_la_posizione_temporale() {
        // impulso a 0,5 s: dopo il ricampionamento deve restare a 0,5 s
        let mut samples = vec![0.0f32; 48_000];
        samples[24_000] = 1.0;
        let out = resample(Pcm { samples, sample_rate: 48_000 }, 16_000).unwrap();
        let (peak_idx, _) = out
            .samples
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
            .unwrap();
        let drift_ms = (peak_idx as f64 - 8_000.0) / 16.0;
        // tolleranza molto sotto i 20 ms di risoluzione dei frame CTC
        assert!(drift_ms.abs() < 1.0, "deriva di {drift_ms:.2} ms (indice {peak_idx})");
    }

    #[test]
    fn normalizzazione_rms_rispetta_il_tetto_di_picco() {
        let mut s: Vec<f32> = (0..16_000)
            .map(|i| 0.01 * (i as f32 * 0.05).sin())
            .collect();
        normalize_in_place(&mut s, &PreprocessConfig::default());
        let peak = s.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        assert!(peak <= db_to_lin(-1.0) + 1e-3, "peak={peak}");
        assert!(peak > 0.05, "il guadagno non e' stato applicato: {peak}");
    }

    #[test]
    fn dc_offset_rimosso() {
        let mut s = vec![0.5f32; 1000];
        let cfg = PreprocessConfig { normalize: NormalizeMode::None, ..Default::default() };
        normalize_in_place(&mut s, &cfg);
        let mean = s.iter().sum::<f32>() / s.len() as f32;
        assert!(mean.abs() < 1e-4, "mean={mean}");
    }

    #[test]
    fn zero_mean_unit_var_ok() {
        let out = zero_mean_unit_var(&[1.0, 2.0, 3.0, 4.0]);
        let mean = out.iter().sum::<f32>() / out.len() as f32;
        assert!(mean.abs() < 1e-5);
    }
}
