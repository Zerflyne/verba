//! Utility condivise per ONNX Runtime (pyannote e wav2vec2).

use std::path::Path;

use anyhow::{Context, Result};
use ort::execution_providers::{CPUExecutionProvider, CUDAExecutionProvider};
use ort::session::{builder::GraphOptimizationLevel, Session};
use tracing::info;

use crate::gpu::Device;

/// Crea una sessione ONNX con provider CUDA quando disponibile.
///
/// Il provider CUDA e' registrato in modo *non fatale*: se il runtime non
/// trova le librerie CUDA/cuDNN, ORT ricade automaticamente sulla CPU invece
/// di far fallire l'intera trascrizione.
pub fn build_session(model: &Path, device: &Device, intra_threads: usize) -> Result<Session> {
    let mut providers: Vec<ort::execution_providers::ExecutionProviderDispatch> = Vec::new();

    if let Some(idx) = device.cuda_index() {
        providers.push(
            CUDAExecutionProvider::default()
                .with_device_id(idx as i32)
                .build(),
        );
    }
    providers.push(CPUExecutionProvider::default().build());

    let session = Session::builder()
        .context("creazione del builder ONNX")?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(intra_threads.max(1))?
        .with_execution_providers(providers)?
        .commit_from_file(model)
        .with_context(|| format!("caricamento del modello ONNX {}", model.display()))?;

    info!(
        modello = %model.display(),
        device = %device.describe(),
        "sessione ONNX pronta"
    );
    Ok(session)
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
