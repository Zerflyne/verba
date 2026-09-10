//! Segmentazione vocale con **pyannote esportato in ONNX**
//! (`pyannote/segmentation-3.0` o `segmentation@2.1`).
//!
//! Il modello lavora su finestre scorrevoli di 10 s a 16 kHz e restituisce una
//! griglia di frame. Due formati di uscita sono supportati:
//!
//!   * **powerset** (seg-3.0): 7 classi = {silenzio, s1, s2, s3, s1+s2, s1+s3,
//!     s2+s3}. La probabilita' di parlato e' `1 - p(silenzio)`.
//!   * **multi-label** (seg-2.1): 3 uscite sigmoidee, una per speaker; la
//!     probabilita' di parlato e' il massimo fra gli speaker.
//!
//! Le finestre si sovrappongono e vengono fuse su una griglia globale a 10 ms
//! prendendo il massimo, poi binarizzate con isteresi (onset/offset) e
//! ripulite con durate minime di parlato/silenzio.

use std::path::Path;

use anyhow::{bail, Context, Result};
use ndarray::Array3;
use ort::session::Session;
use ort::value::TensorRef;
use tracing::{debug, info};

use crate::audio::Pcm;
use crate::gpu::Device;
use crate::onnx::{build_session, looks_like_probabilities, sigmoid, softmax};

/// Risoluzione della griglia globale di probabilita'.
const GRID_MS: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
}

impl Segment {
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }
}

#[derive(Debug, Clone)]
pub struct SegmentationConfig {
    /// Durata della finestra scorrevole, in secondi (pyannote: 10 s).
    pub window_secs: f64,
    /// Passo fra finestre consecutive, in secondi.
    pub hop_secs: f64,
    /// Soglia di attivazione (inizio parlato).
    pub onset: f32,
    /// Soglia di disattivazione (fine parlato); < onset = isteresi.
    pub offset: f32,
    /// Durata minima di un segmento di parlato.
    pub min_duration_on: f64,
    /// Pause piu' corte di questo valore non spezzano il segmento.
    pub min_duration_off: f64,
    /// Margine aggiunto a inizio e fine di ogni segmento.
    pub pad_secs: f64,
    /// Lunghezza massima di un segmento: oltre, viene spezzato nel punto meno
    /// "parlato" (Whisper lavora su finestre di 30 s).
    pub max_segment_secs: f64,
}

impl Default for SegmentationConfig {
    fn default() -> Self {
        Self {
            window_secs: 10.0,
            hop_secs: 5.0,
            onset: 0.50,
            offset: 0.35,
            min_duration_on: 0.20,
            min_duration_off: 0.30,
            pad_secs: 0.15,
            max_segment_secs: 28.0,
        }
    }
}

pub struct Segmenter {
    session: Session,
    cfg: SegmentationConfig,
    input_name: String,
    /// Rango atteso dal tensore di ingresso: 3 = [B,1,N], 2 = [B,N].
    input_rank: usize,
}

impl Segmenter {
    pub fn new(model: &Path, device: &Device, cfg: SegmentationConfig, threads: usize) -> Result<Self> {
        let session = build_session(model, device, threads)?;

        let input = session
            .inputs
            .first()
            .context("il modello di segmentazione non espone input")?;
        let input_name = input.name.clone();
        let input_rank = input
            .input_type
            .tensor_shape()
            .map(|d| d.len())
            .unwrap_or(3);

        info!(input = %input_name, rank = input_rank, "modello di segmentazione pyannote caricato");
        Ok(Self { session, cfg, input_name, input_rank })
    }

    /// Restituisce i segmenti di parlato dell'intero file.
    pub fn run(&mut self, pcm: &Pcm) -> Result<Vec<Segment>> {
        let sr = pcm.sample_rate as f64;
        let win_len = (self.cfg.window_secs * sr).round() as usize;
        let hop_len = (self.cfg.hop_secs * sr).round() as usize;
        if win_len == 0 || hop_len == 0 {
            bail!("finestra o passo di segmentazione non validi");
        }

        let total = pcm.samples.len();
        let duration = pcm.duration_secs();
        let grid_len = ((duration * 1000.0 / GRID_MS).ceil() as usize).max(1);
        let mut grid = vec![0.0f32; grid_len];

        let mut start = 0usize;
        let mut windows = 0usize;
        loop {
            let end = (start + win_len).min(total);
            // Ultima finestra: zero-padding fino alla lunghezza attesa (il
            // modello ha un ingresso a lunghezza fissa nella maggior parte
            // degli export).
            let mut chunk = vec![0.0f32; win_len];
            chunk[..end - start].copy_from_slice(&pcm.samples[start..end]);

            let probs = self.infer_window(&chunk)?;
            let win_start_secs = start as f64 / sr;
            let frame_secs = self.cfg.window_secs / probs.len().max(1) as f64;

            for (i, &p) in probs.iter().enumerate() {
                let t0 = win_start_secs + i as f64 * frame_secs;
                let t1 = t0 + frame_secs;
                let g0 = ((t0 * 1000.0 / GRID_MS).floor() as usize).min(grid_len - 1);
                let g1 = ((t1 * 1000.0 / GRID_MS).ceil() as usize).min(grid_len);
                for g in grid.iter_mut().take(g1).skip(g0) {
                    // fusione delle finestre sovrapposte: massimo, piu'
                    // conservativo della media sui bordi di parola
                    if p > *g {
                        *g = p;
                    }
                }
            }

            windows += 1;
            if end >= total {
                break;
            }
            start += hop_len;
        }

        debug!(finestre = windows, frame_griglia = grid_len, "inferenza di segmentazione completata");

        let segments = self.binarize(&grid, duration);
        let segments = self.split_long(segments, &grid);

        let parlato: f64 = segments.iter().map(|s| s.duration()).sum();
        info!(
            segmenti = segments.len(),
            parlato_secs = format!("{:.1}", parlato),
            totale_secs = format!("{:.1}", duration),
            "segmentazione completata"
        );
        Ok(segments)
    }

    /// Inferenza su una finestra: ritorna la probabilita' di parlato per frame.
    fn infer_window(&mut self, chunk: &[f32]) -> Result<Vec<f32>> {
        let n = chunk.len();
        let array = if self.input_rank == 2 {
            Array3::from_shape_vec((1, 1, n), chunk.to_vec())?
                .into_shape_with_order((1, n))?
                .into_dyn()
        } else {
            Array3::from_shape_vec((1, 1, n), chunk.to_vec())?.into_dyn()
        };

        let tensor = TensorRef::from_array_view(&array)?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => tensor])
            .context("inferenza di segmentazione")?;

        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .context("estrazione dell'uscita di segmentazione")?;

        // Atteso [batch, frames, classi]; tolleriamo [frames, classi].
        let dims: Vec<usize> = shape.iter().map(|d| *d as usize).collect();
        let (frames, classes) = match dims.as_slice() {
            [_b, f, c] => (*f, *c),
            [f, c] => (*f, *c),
            other => bail!("forma inattesa dell'uscita di segmentazione: {other:?}"),
        };

        let mut speech = Vec::with_capacity(frames);
        let mut row = vec![0.0f32; classes];
        for f in 0..frames {
            row.copy_from_slice(&data[f * classes..(f + 1) * classes]);

            let p = if classes >= 5 {
                // powerset: la classe 0 e' "nessuno parla"
                if !looks_like_probabilities(&row) {
                    softmax(&mut row);
                }
                1.0 - row[0]
            } else {
                // multi-label: una sigmoide per speaker
                row.iter()
                    .map(|&v| if (0.0..=1.0).contains(&v) { v } else { sigmoid(v) })
                    .fold(0.0f32, f32::max)
            };
            speech.push(p.clamp(0.0, 1.0));
        }
        Ok(speech)
    }

    /// Binarizzazione con isteresi + pulizia delle durate.
    fn binarize(&self, grid: &[f32], duration: f64) -> Vec<Segment> {
        let step = GRID_MS / 1000.0;
        let mut raw: Vec<Segment> = Vec::new();
        let mut active = false;
        let mut start_idx = 0usize;

        for (i, &p) in grid.iter().enumerate() {
            if !active && p >= self.cfg.onset {
                active = true;
                start_idx = i;
            } else if active && p < self.cfg.offset {
                active = false;
                raw.push(Segment { start: start_idx as f64 * step, end: i as f64 * step });
            }
        }
        if active {
            raw.push(Segment { start: start_idx as f64 * step, end: grid.len() as f64 * step });
        }

        // Fonde i segmenti separati da pause troppo brevi.
        let mut merged: Vec<Segment> = Vec::with_capacity(raw.len());
        for seg in raw {
            match merged.last_mut() {
                Some(prev) if seg.start - prev.end < self.cfg.min_duration_off => {
                    prev.end = seg.end;
                }
                _ => merged.push(seg),
            }
        }

        // Scarta i frammenti troppo corti, applica il padding e richiude le
        // sovrapposizioni create dal padding stesso.
        let mut out: Vec<Segment> = Vec::with_capacity(merged.len());
        for seg in merged {
            if seg.duration() < self.cfg.min_duration_on {
                continue;
            }
            let mut s = Segment {
                start: (seg.start - self.cfg.pad_secs).max(0.0),
                end: (seg.end + self.cfg.pad_secs).min(duration),
            };
            if let Some(prev) = out.last_mut() {
                if s.start < prev.end {
                    let mid = (prev.end + s.start) / 2.0;
                    prev.end = mid;
                    s.start = mid;
                }
            }
            out.push(s);
        }
        out
    }

    /// Spezza i segmenti oltre `max_segment_secs` nel punto di minima
    /// probabilita' di parlato, cosi' il taglio cade fra due parole.
    fn split_long(&self, segments: Vec<Segment>, grid: &[f32]) -> Vec<Segment> {
        let step = GRID_MS / 1000.0;
        let max_len = self.cfg.max_segment_secs;
        let mut out = Vec::with_capacity(segments.len());
        let mut queue: Vec<Segment> = segments;

        while let Some(seg) = queue.pop() {
            if seg.duration() <= max_len {
                out.push(seg);
                continue;
            }
            // Cerca il minimo nella meta' centrale, per non produrre spezzoni
            // squilibrati.
            let a = ((seg.start + seg.duration() * 0.35) / step) as usize;
            let b = (((seg.start + seg.duration() * 0.65) / step) as usize).min(grid.len());
            let cut_idx = (a..b)
                .min_by(|&i, &j| grid[i].partial_cmp(&grid[j]).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or((a + b) / 2);
            let cut = cut_idx as f64 * step;

            queue.push(Segment { start: seg.start, end: cut });
            queue.push(Segment { start: cut, end: seg.end });
        }

        out.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
        out
    }
}

/// Fallback usato quando il modello di segmentazione non e' disponibile:
/// finestre a lunghezza fissa con sovrapposizione minima.
pub fn uniform_segments(duration: f64, window: f64) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut t = 0.0;
    while t < duration {
        let end = (t + window).min(duration);
        out.push(Segment { start: t, end });
        t = end;
    }
    out
}
