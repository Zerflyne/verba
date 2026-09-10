//! Quanta memoria serve, e se ce n'e' abbastanza per non fare la staffetta.
//!
//! La pipeline nasce prudente: Whisper viene scaricato dalla memoria prima che
//! l'allineatore venga caricato, perche' su una scheda da 8 GB i due insieme
//! non ci stanno. Ma su una scheda da 24 GB quella prudenza e' solo tempo
//! perso — scarico, ricarico, e nel mezzo il driver deve pure liberare.
//!
//! Qui sta la regola che decide quale delle due strade prendere: **se la
//! memoria libera e' almeno il 20% in piu' della somma stimata dei due
//! modelli, si tengono caricati entrambi.** Il margine non e' decorativo: le
//! stime sono stime, ONNX Runtime alloca i propri buffer di lavoro oltre ai
//! pesi, e una previsione sbagliata per difetto costa un errore di memoria
//! esaurita a meta' di una trascrizione lunga.
//!
//! Quando la memoria libera non si riesce a misurare — nessuna GPU, oppure un
//! sistema operativo di cui non sappiamo leggere i contatori — la risposta e'
//! no. Una stima ottimistica costerebbe un errore; una prudente costa
//! qualche secondo.

use tracing::info;

use crate::gpu::{self, Device};
use crate::modelli::Dimensione;

/// Il margine richiesto sopra la stima: 1.2 significa «il 20% in piu'».
pub const MARGINE: f64 = 1.2;

/// Memoria di lavoro stimata per Whisper, in MiB.
///
/// Non e' la dimensione del file: whisper.cpp alloca i pesi piu' i buffer di
/// stato e il grafo di calcolo. I valori sono quelli dichiarati da whisper.cpp
/// per l'inferenza a beam search, arrotondati per eccesso.
pub fn whisper_mib(d: Dimensione) -> u64 {
    match d {
        Dimensione::LargeV3 => 4400,
        Dimensione::Medium => 2200,
        Dimensione::Small => 1000,
    }
}

/// Memoria di lavoro stimata per l'allineatore wav2vec2, in MiB.
///
/// Il file ONNX pesa 1,26 GB; ONNX Runtime ci aggiunge le attivazioni, che su
/// finestre da trenta secondi restano modeste.
pub const ALLINEATORE_MIB: u64 = 1600;

/// Memoria di lavoro stimata per la segmentazione pyannote, in MiB.
///
/// Il modello e' minuscolo (5,9 MB) ma la sessione ONNX ha comunque un costo
/// fisso. Non entra nella decisione: quella fase e' finita da un pezzo quando
/// si arriva a scegliere.
pub const SEGMENTAZIONE_MIB: u64 = 300;

/// La somma che deve starci: Whisper piu' l'allineatore.
pub fn richiesta_mib(d: Dimensione) -> u64 {
    whisper_mib(d) + ALLINEATORE_MIB
}

/// Quanto serve avere libero perche' valga la pena tenerli insieme.
pub fn soglia_mib(d: Dimensione) -> u64 {
    (richiesta_mib(d) as f64 * MARGINE).ceil() as u64
}

/// La memoria libera che conta per questo dispositivo: la VRAM su GPU, la RAM
/// disponibile su CPU. `None` quando non si riesce a misurarla.
pub fn libera_mib(device: &Device) -> Option<u64> {
    match device {
        Device::Cuda { .. } => gpu::vram_libera_mib(device),
        Device::Cpu => ram_disponibile_mib(),
    }
}

/// La RAM realmente disponibile, in MiB.
///
/// Su Linux e' `MemAvailable` di `/proc/meminfo`, che e' la stima del kernel di
/// quanto si puo' allocare senza far entrare in gioco lo swap — molto piu'
/// utile di `MemFree`, che su una macchina in uso e' quasi sempre piccolo
/// perche' la cache del disco occupa il resto. Altrove non lo sappiamo leggere
/// senza tirarsi dietro una dipendenza, e restituiamo `None`.
pub fn ram_disponibile_mib() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let testo = std::fs::read_to_string("/proc/meminfo").ok()?;
        for riga in testo.lines() {
            if let Some(resto) = riga.strip_prefix("MemAvailable:") {
                let kib: u64 = resto.split_whitespace().next()?.parse().ok()?;
                return Some(kib / 1024);
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// L'esito della decisione, con i numeri che l'hanno prodotta.
///
/// Porta con se' il perche' perche' finisce nel log e, quando serve, in un
/// avviso a schermo: «tengo caricati entrambi» senza dire con quanta memoria
/// non e' verificabile da chi legge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decisione {
    /// Vero se i due modelli restano caricati insieme.
    pub insieme: bool,
    pub richiesta_mib: u64,
    pub soglia_mib: u64,
    /// La memoria libera misurata, se si e' riusciti a misurarla.
    pub libera_mib: Option<u64>,
}

impl Decisione {
    /// La riga da mettere nel log o sotto gli occhi di chi guarda.
    pub fn spiegazione(&self) -> String {
        match (self.insieme, self.libera_mib) {
            (true, Some(libera)) => format!(
                "Whisper e allineatore restano caricati insieme: {libera} MiB liberi, \
                 ne servivano {} ({} stimati piu' il 20%)",
                self.soglia_mib, self.richiesta_mib
            ),
            (false, Some(libera)) => format!(
                "Whisper viene scaricato prima dell'allineatore: {libera} MiB liberi, \
                 per tenerli insieme ne servirebbero {}",
                self.soglia_mib
            ),
            (_, None) => {
                "Whisper viene scaricato prima dell'allineatore: la memoria libera \
                 non e' misurabile su questo sistema"
                    .to_string()
            }
        }
    }
}

/// Decide se caricare tutto insieme.
pub fn decidi(device: &Device, d: Dimensione) -> Decisione {
    let libera = libera_mib(device);
    let soglia = soglia_mib(d);
    let decisione = Decisione {
        insieme: libera.is_some_and(|l| l >= soglia),
        richiesta_mib: richiesta_mib(d),
        soglia_mib: soglia,
        libera_mib: libera,
    };
    info!(
        insieme = decisione.insieme,
        libera_mib = libera.unwrap_or(0),
        soglia_mib = soglia,
        dispositivo = %device.describe(),
        "{}",
        decisione.spiegazione()
    );
    decisione
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_soglia_e_il_venti_per_cento_sopra_la_stima() {
        // large-v3: 4400 + 1600 = 6000, e il 20% sopra fa 7200.
        assert_eq!(richiesta_mib(Dimensione::LargeV3), 6000);
        assert_eq!(soglia_mib(Dimensione::LargeV3), 7200);
    }

    #[test]
    fn un_modello_piu_piccolo_abbassa_la_soglia() {
        assert!(soglia_mib(Dimensione::Small) < soglia_mib(Dimensione::Medium));
        assert!(soglia_mib(Dimensione::Medium) < soglia_mib(Dimensione::LargeV3));
    }

    #[test]
    fn senza_misura_non_si_azzarda() {
        // E' il caso di una macchina di cui non sappiamo leggere la memoria:
        // la risposta deve essere la strada prudente, non quella comoda.
        let d = Decisione {
            insieme: false,
            richiesta_mib: 6000,
            soglia_mib: 7200,
            libera_mib: None,
        };
        assert!(!d.insieme);
        assert!(d.spiegazione().contains("non e' misurabile"));
    }

    #[test]
    fn la_spiegazione_dice_i_numeri_su_cui_si_e_deciso() {
        let d = Decisione {
            insieme: true,
            richiesta_mib: 6000,
            soglia_mib: 7200,
            libera_mib: Some(22000),
        };
        let s = d.spiegazione();
        assert!(s.contains("22000"), "{s}");
        assert!(s.contains("7200"), "{s}");
    }

    #[test]
    fn su_cpu_la_memoria_che_conta_e_la_ram() {
        // Su Linux il valore c'e' sempre; altrove non lo sappiamo leggere e la
        // pipeline ricade sulla staffetta, che e' l'esito prudente.
        let misura = libera_mib(&Device::Cpu);
        #[cfg(target_os = "linux")]
        assert!(misura.is_some_and(|m| m > 0), "MemAvailable dovrebbe essere leggibile");
        #[cfg(not(target_os = "linux"))]
        assert!(misura.is_none());
    }
}
