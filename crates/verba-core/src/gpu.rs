//! Selezione del dispositivo di calcolo e monitoraggio della VRAM.
//!
//! Regola richiesta: la GPU va preferita se possiede **almeno N MiB di VRAM
//! totale**, indipendentemente da quanta ne sia libera al momento della
//! scelta. Fra le GPU idonee vince quella con piu' memoria totale.


use nvml_wrapper::Nvml;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Soglia di default: 8000 MiB. Nota: schede da "8 GB" espongono spesso
/// 8188 MiB (7.99 GiB), quindi una soglia di 8192 le escluderebbe per 4 MiB.
pub const DEFAULT_MIN_VRAM_MIB: u64 = 8000;

/// Una GPU come la vede chi deve sceglierla dall'elenco.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scheda {
    pub indice: u32,
    pub nome: String,
    pub totale_mib: u64,
    pub libera_mib: u64,
}

impl Scheda {
    /// «CUDA:1 — Tesla P40, 23040 MiB». E' la riga del menu a tendina.
    pub fn etichetta(&self) -> String {
        format!("CUDA:{} — {}, {} MiB", self.indice, self.nome, self.totale_mib)
    }
}

/// Tutte le GPU NVIDIA visibili, nell'ordine in cui NVML le espone.
///
/// Un elenco vuoto significa «nessuna GPU utilizzabile», non «errore»: NVML
/// puo' mancare del tutto, ed e' una macchina che lavora su CPU.
pub fn elenco() -> Vec<Scheda> {
    let Ok(nvml) = Nvml::init() else { return Vec::new() };
    let Ok(count) = nvml.device_count() else { return Vec::new() };
    let mut out = Vec::with_capacity(count as usize);
    for indice in 0..count {
        let Ok(dev) = nvml.device_by_index(indice) else { continue };
        let nome = dev.name().unwrap_or_else(|_| format!("GPU {indice}"));
        let Ok(mem) = dev.memory_info() else { continue };
        out.push(Scheda {
            indice,
            nome,
            totale_mib: mem.total / 1024 / 1024,
            libera_mib: mem.free / 1024 / 1024,
        });
    }
    out
}

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

    let schede = elenco();
    if schede.is_empty() {
        warn!("nessuna GPU NVIDIA visibile: si procede su CPU");
        return Device::Cpu;
    }
    for s in &schede {
        info!(
            gpu = s.indice, nome = %s.nome, totale_mib = s.totale_mib, libera_mib = s.libera_mib,
            "GPU rilevata"
        );
    }

    if let Some(want) = preferred {
        if let Some(s) = schede.iter().find(|s| s.indice == want) {
            let dev = Device::Cuda { index: s.indice, name: s.nome.clone(), total_mib: s.totale_mib };
            info!(device = %dev.describe(), "GPU scelta a mano");
            return dev;
        }
        warn!(indice = want, "la GPU richiesta non esiste: ricado sulla selezione automatica");
    }

    // Il criterio e' la VRAM *totale*: la memoria libera non incide, perche'
    // le fasi della pipeline vengono caricate e scaricate una alla volta.
    let best = schede
        .into_iter()
        .filter(|s| s.totale_mib >= min_total_mib)
        .max_by_key(|s| s.totale_mib);

    match best {
        Some(s) => {
            let dev = Device::Cuda { index: s.indice, name: s.nome, total_mib: s.totale_mib };
            info!(device = %dev.describe(), soglia_mib = min_total_mib, "GPU selezionata");
            dev
        }
        None => {
            warn!(soglia_mib = min_total_mib, "nessuna GPU raggiunge la soglia di VRAM: si procede su CPU");
            Device::Cpu
        }
    }
}

/// La VRAM libera, in MiB, del dispositivo scelto.
pub fn vram_libera_mib(device: &Device) -> Option<u64> {
    let index = device.cuda_index()?;
    let nvml = Nvml::init().ok()?;
    let dev = nvml.device_by_index(index).ok()?;
    let mem = dev.memory_info().ok()?;
    Some(mem.free / 1024 / 1024)
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
