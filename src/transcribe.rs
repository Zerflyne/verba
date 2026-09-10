//! Trascrizione con **Whisper large-v3** tramite whisper.cpp (whisper-rs).
//!
//! Whisper fornisce il *testo*; i tempi parola-per-parola arrivano dal
//! successivo allineamento forzato con wav2vec2. Il contesto viene rilasciato
//! esplicitamente (`release`) per liberare la VRAM prima di caricare
//! l'allineatore.

use std::path::Path;

use anyhow::{Context, Result};
use tracing::{debug, info, warn};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::audio::Pcm;
use crate::gpu::{self, Device};
use crate::segmentation::Segment;

/// Un segmento con il testo riconosciuto.
#[derive(Debug, Clone)]
pub struct Transcript {
    pub segment: Segment,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct WhisperConfig {
    pub language: String,
    pub translate: bool,
    pub beam_size: i32,
    pub threads: i32,
    /// Prompt iniziale: aiuta su nomi propri e punteggiatura.
    pub initial_prompt: Option<String>,
    /// Lunghezza minima di un segmento da inviare a Whisper.
    pub min_segment_secs: f64,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            language: "it".to_string(),
            translate: false,
            beam_size: 5,
            threads: num_threads(),
            initial_prompt: None,
            min_segment_secs: 0.20,
        }
    }
}

fn num_threads() -> i32 {
    std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(4).min(16)
}

pub struct Transcriber {
    ctx: Option<WhisperContext>,
    cfg: WhisperConfig,
    device: Device,
}

impl Transcriber {
    pub fn new(model: &Path, device: &Device, cfg: WhisperConfig) -> Result<Self> {
        let mut params = WhisperContextParameters::default();
        params.use_gpu(device.is_cuda());
        if let Some(idx) = device.cuda_index() {
            params.gpu_device(idx as i32);
        }

        let path = model
            .to_str()
            .context("il percorso del modello Whisper non e' UTF-8 valido")?;
        let ctx = WhisperContext::new_with_params(path, params)
            .with_context(|| format!("caricamento del modello Whisper {}", model.display()))?;

        info!(
            modello = %model.display(),
            device = %device.describe(),
            lingua = %cfg.language,
            "Whisper large-v3 caricato"
        );
        gpu::log_vram(device, "dopo il caricamento di Whisper");

        Ok(Self { ctx: Some(ctx), cfg, device: device.clone() })
    }

    /// Trascrive ogni segmento individuato da pyannote.
    pub fn run(&mut self, pcm: &Pcm, segments: &[Segment]) -> Result<Vec<Transcript>> {
        let ctx = self.ctx.as_ref().context("contesto Whisper gia' rilasciato")?;
        let mut state = ctx.create_state().context("creazione dello stato Whisper")?;
        let mut out = Vec::with_capacity(segments.len());

        for (i, seg) in segments.iter().enumerate() {
            if seg.duration() < self.cfg.min_segment_secs {
                continue;
            }
            let samples = pcm.slice_secs(seg.start, seg.end);
            // whisper.cpp richiede almeno ~1 s di segnale: completa con silenzio.
            let padded: Vec<f32> = if samples.len() < 16_000 {
                let mut v = samples.to_vec();
                v.resize(16_000, 0.0);
                v
            } else {
                samples.to_vec()
            };

            let mut params = FullParams::new(SamplingStrategy::BeamSearch {
                beam_size: self.cfg.beam_size,
                patience: 0.0,
            });
            params.set_n_threads(self.cfg.threads);
            params.set_language(Some(self.cfg.language.as_str()));
            params.set_translate(self.cfg.translate);
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            params.set_suppress_blank(true);
            params.set_no_context(true);
            if let Some(p) = self.cfg.initial_prompt.as_deref() {
                params.set_initial_prompt(p);
            }

            state
                .full(params, &padded)
                .with_context(|| format!("trascrizione del segmento {i}"))?;

            let n = state.full_n_segments().context("conteggio dei segmenti Whisper")?;
            let mut text = String::new();
            for s in 0..n {
                let piece = state
                    .full_get_segment_text(s)
                    .with_context(|| format!("testo del sotto-segmento {s}"))?;
                text.push_str(&piece);
            }
            let text = clean(&text);

            if text.is_empty() {
                debug!(segmento = i, "nessun testo riconosciuto: segmento scartato");
                continue;
            }
            debug!(
                segmento = i,
                inizio = format!("{:.2}", seg.start),
                fine = format!("{:.2}", seg.end),
                testo = %text,
                "trascritto"
            );
            out.push(Transcript { segment: *seg, text });
        }

        let parole: usize = out.iter().map(|t| t.text.split_whitespace().count()).sum();
        info!(segmenti = out.len(), parole, "trascrizione completata");
        Ok(out)
    }

    /// **Scarica Whisper dalla GPU.** Va chiamata prima di istanziare
    /// l'allineatore: il `Drop` di `WhisperContext` libera i pesi dalla VRAM.
    pub fn release(&mut self) {
        if self.ctx.take().is_some() {
            info!("Whisper scaricato dalla memoria del dispositivo");
        }
        // La deallocazione asincrona del driver puo' impiegare qualche
        // millisecondo a riflettersi nei contatori NVML.
        std::thread::sleep(std::time::Duration::from_millis(300));
        gpu::log_vram(&self.device, "dopo lo scarico di Whisper");
    }
}

impl Drop for Transcriber {
    fn drop(&mut self) {
        if self.ctx.is_some() {
            warn!("Transcriber deallocato senza release() esplicita");
        }
    }
}

/// Ripulisce il testo di Whisper: token speciali, spazi doppi, bordi.
fn clean(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '[' | '(' if !in_tag => in_tag = true,
            ']' | ')' if in_tag => in_tag = false,
            _ if in_tag => {}
            _ => s.push(ch),
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn rimuove_tag_e_spazi() {
        assert_eq!(clean("  [MUSICA]  ciao   mondo "), "ciao mondo");
        assert_eq!(clean("(applausi) buongiorno"), "buongiorno");
    }
}
