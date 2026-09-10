//! Selezione del dispositivo di calcolo e monitoraggio della VRAM.
//!
//! Regola richiesta: la GPU va preferita se possiede **almeno N MiB di VRAM
//! totale**, indipendentemente da quanta ne sia libera al momento della
//! scelta. Fra le GPU idonee vince quella con piu' memoria totale.


use nvml_wrapper::Nvml;
use tracing::{info, warn};

/// Soglia di default: 8000 MiB. Nota: schede da "8 GB" espongono spesso
/// 8188 MiB (7.99 GiB), quindi una soglia di 8192 le escluderebbe per 4 MiB.
pub const DEFAULT_MIN_VRAM_MIB: u64 = 8000;

#[derive(Debug, Clone)]
pub enum Device {
    Cuda { index: u32, name: String, total_mib: u64 },
    Cpu,
}

impl Device {
    pub fn is_cuda(&self) -> bool {
        matches!(self, Device::Cuda { .. })
    }

    /// Indice CUDA da passare a ONNX Runtime / whisper.cpp.
    pub fn cuda_index(&self) -> Option<u32> {
        match self {
            Device::Cuda { index, .. } => Some(*index),
            Device::Cpu => None,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Device::Cuda { index, name, total_mib } => {
                format!("CUDA:{index} ({name}, {total_mib} MiB totali)")
            }
            Device::Cpu => "CPU".to_string(),
        }
    }
}

/// Sceglie il dispositivo. `force_cpu` bypassa tutto; `preferred` forza un
/// indice CUDA specifico (saltando comunque il controllo di soglia).
pub fn select(min_total_mib: u64, force_cpu: bool, preferred: Option<u32>) -> Device {
    if force_cpu {
        info!("esecuzione su CPU forzata da riga di comando");
        return Device::Cpu;
    }

    let nvml = match Nvml::init() {
        Ok(n) => n,
        Err(e) => {
            warn!(error = %e, "NVML non disponibile: si procede su CPU");
            return Device::Cpu;
        }
    };

    let count = match nvml.device_count() {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "impossibile enumerare le GPU: si procede su CPU");
            return Device::Cpu;
        }
    };

    let mut candidates: Vec<(u32, String, u64, u64)> = Vec::new(); // idx, nome, totale, libera
    for idx in 0..count {
        let Ok(dev) = nvml.device_by_index(idx) else { continue };
        let name = dev.name().unwrap_or_else(|_| format!("GPU {idx}"));
        let Ok(mem) = dev.memory_info() else { continue };
        let total_mib = mem.total / 1024 / 1024;
        let free_mib = mem.free / 1024 / 1024;
        info!(
            gpu = idx, %name, totale_mib = total_mib, libera_mib = free_mib,
            "GPU rilevata"
        );
        candidates.push((idx, name, total_mib, free_mib));
    }

    if let Some(want) = preferred {
        if let Some((idx, name, total, _)) = candidates.iter().find(|c| c.0 == want) {
            let dev = Device::Cuda { index: *idx, name: name.clone(), total_mib: *total };
            info!(device = %dev.describe(), "GPU imposta da riga di comando");
            return dev;
        }
        warn!(indice = want, "la GPU richiesta non esiste: ricado sulla selezione automatica");
    }

    // Il criterio e' la VRAM *totale*: la memoria libera non incide, perche'
    // le fasi della pipeline vengono caricate e scaricate una alla volta.
    let best = candidates
        .into_iter()
        .filter(|(_, _, total, _)| *total >= min_total_mib)
        .max_by_key(|(_, _, total, _)| *total);

    match best {
        Some((index, name, total_mib, _)) => {
            let dev = Device::Cuda { index, name, total_mib };
            info!(device = %dev.describe(), soglia_mib = min_total_mib, "GPU selezionata");
            dev
        }
        None => {
            warn!(soglia_mib = min_total_mib, "nessuna GPU raggiunge la soglia di VRAM: si procede su CPU");
            Device::Cpu
        }
    }
}

/// Stato della VRAM (MiB usati / totali) del dispositivo selezionato.
pub fn memory_used_mib(device: &Device) -> Option<(u64, u64)> {
    let index = device.cuda_index()?;
    let nvml = Nvml::init().ok()?;
    let dev = nvml.device_by_index(index).ok()?;
    let mem = dev.memory_info().ok()?;
    Some((mem.used / 1024 / 1024, mem.total / 1024 / 1024))
}

/// Traccia l'occupazione di VRAM in un punto della pipeline: serve a rendere
/// verificabile lo scarico di Whisper prima del caricamento dell'allineatore.
pub fn log_vram(device: &Device, fase: &str) {
    if let Some((used, total)) = memory_used_mib(device) {
        info!(fase, vram_usata_mib = used, vram_totale_mib = total, "stato VRAM");
    }
}
