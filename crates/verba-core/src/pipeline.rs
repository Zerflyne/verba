//! La pipeline di trascrizione, dall'audio alla sequenza di parole.
//!
//! Questo modulo esiste perche' l'anteprima e l'export devono percorrere la
//! stessa strada. Se la riga di comando e l'applicazione orchestrassero i
//! modelli ognuna per conto suo, divergerebbero su qualche dettaglio — un
//! parametro passato diversamente, una fase saltata — e la differenza si
//! manifesterebbe come un risultato che cambia a seconda di come lo hai
//! chiesto.
//!
//! L'ordine dei modelli non e' negoziabile: **Whisper viene scaricato dalla
//! memoria prima che l'allineatore venga caricato**. I due insieme non stanno
//! in 8 GB di VRAM, e se la sequenza non e' esplicita l'errore che ne esce
//! sembra casuale.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::warn;

use crate::align::{AlignConfig, Aligner};
use crate::audio::Pcm;
use crate::eventi::{Fase, Progresso};
use crate::gpu::{self, Device};
use crate::onnx;
use crate::segmentation::{self, SegmentationConfig, Segmenter};
use crate::transcribe::{Transcriber, WhisperConfig};
use crate::trascrizione::Trascrizione;

/// Dove stanno i modelli.
#[derive(Debug, Clone)]
pub struct PercorsiModelli {
    /// Whisper large-v3 in formato GGML/GGUF (whisper.cpp).
    pub whisper: PathBuf,
    /// pyannote segmentation-3.0 esportato in ONNX.
    pub segmentazione: PathBuf,
    /// wav2vec2 italiano (testa CTC) esportato in ONNX.
    pub allineamento: PathBuf,
    /// Vocabolario del tokenizer wav2vec2 (`vocab.json`).
    pub vocabolario: PathBuf,
}

impl PercorsiModelli {
    /// I quattro file dentro una cartella, con i nomi che usa lo script di
    /// esportazione.
    pub fn nella_cartella(cartella: impl AsRef<Path>) -> Self {
        let c = cartella.as_ref();
        Self {
            whisper: c.join("ggml-large-v3.bin"),
            segmentazione: c.join("pyannote-segmentation-3.0.onnx"),
            allineamento: c.join("wav2vec2-italian.onnx"),
            vocabolario: c.join("wav2vec2-italian.vocab.json"),
        }
    }

    /// Quali dei file richiesti mancano. Vuoto significa che si puo' partire.
    pub fn mancanti(&self) -> Vec<&Path> {
        [&self.whisper, &self.segmentazione, &self.allineamento, &self.vocabolario]
            .into_iter()
            .map(|p| p.as_path())
            .filter(|p| !p.exists())
            .collect()
    }
}

/// Tutto cio' che serve per trascrivere.
#[derive(Debug, Clone)]
pub struct ConfigTrascrizione {
    pub modelli: PercorsiModelli,
    pub segmentazione: SegmentationConfig,
    pub whisper: WhisperConfig,
    pub allineamento: AlignConfig,
    /// Thread CPU per ONNX Runtime e whisper.cpp.
    pub thread: usize,
    /// Salta pyannote e usa finestre uniformi di questa durata, in secondi.
    /// Serve quando il modello di segmentazione non c'e' o da' problemi.
    pub finestre_uniformi: Option<f64>,
}

impl Default for ConfigTrascrizione {
    fn default() -> Self {
        Self {
            modelli: PercorsiModelli::nella_cartella("models"),
            segmentazione: SegmentationConfig::default(),
            whisper: WhisperConfig::default(),
            allineamento: AlignConfig::default(),
            thread: 4,
            finestre_uniformi: None,
        }
    }
}

/// Dall'audio gia' preparato alla sequenza di parole.
///
/// Ogni fase carica il proprio modello e lo rilascia prima della successiva.
/// `progresso` riceve inizio, avanzamento e fine di ciascuna; fra una fase e
/// l'altra viene verificata la richiesta di annullamento.
///
/// Un audio senza parlato, o in cui Whisper non riconosce nulla, non e' un
/// errore: produce una trascrizione vuota, e a valle un video interamente
/// trasparente.
pub fn trascrivi(
    pcm: &Pcm,
    device: &Device,
    cfg: &ConfigTrascrizione,
    progresso: &Progresso,
) -> Result<Trascrizione> {
    // Prima di tutto: la libreria di ONNX Runtime. Se manca, e' meglio dirlo
    // adesso che dopo aver caricato un modello da un gigabyte.
    onnx::assicura_libreria()?;
    gpu::log_vram(device, "iniziale");

    // ------------------------------------------------------------- fase 1
    // Rilevamento del parlato. La sessione ONNX viene chiusa subito dopo, per
    // lasciare la memoria a Whisper.
    let segmenti = {
        let _c = progresso.inizia(Fase::Segmentazione);
        match cfg.finestre_uniformi {
            Some(finestra) => {
                let messaggio = format!(
                    "rilevamento del parlato disattivato: finestre uniformi da {finestra:.0} s"
                );
                warn!("{messaggio}");
                progresso.avviso(messaggio);
                segmentation::uniform_segments(pcm.duration_secs(), finestra)
            }
            None => {
                let mut segmenter = Segmenter::new(
                    &cfg.modelli.segmentazione,
                    device,
                    cfg.segmentazione.clone(),
                    cfg.thread,
                )
                .context("inizializzazione del segmentatore pyannote")?;
                let s = segmenter.run(pcm)?;
                drop(segmenter);
                s
            }
        }
    };
    progresso.verifica()?;

    if segmenti.is_empty() {
        let messaggio = "nessun parlato riconosciuto nell'audio".to_string();
        warn!("{messaggio}");
        progresso.avviso(messaggio);
        return Ok(Trascrizione::vuota(pcm.duration_secs()));
    }

    // ------------------------------------------------------------- fase 2
    // Trascrizione.
    let testi = {
        let _c = progresso.inizia(Fase::Trascrizione);
        let mut transcriber = Transcriber::new(&cfg.modelli.whisper, device, cfg.whisper.clone())
            .context("inizializzazione di Whisper")?;
        let t = transcriber.run(pcm, &segmenti)?;

        // *** Whisper esce dalla memoria PRIMA che entri l'allineatore. ***
        transcriber.release();
        drop(transcriber);
        t
    };
    progresso.verifica()?;

    if testi.iter().all(|t| t.text.trim().is_empty()) {
        let messaggio = "nessun parlato riconosciuto nell'audio".to_string();
        warn!("{messaggio}");
        progresso.avviso(messaggio);
        return Ok(Trascrizione::vuota(pcm.duration_secs()));
    }

    // ------------------------------------------------------------- fase 3
    // Allineamento forzato: ora la memoria e' libera.
    let trascrizione = {
        let _c = progresso.inizia(Fase::Allineamento);
        let mut aligner = Aligner::new(
            &cfg.modelli.allineamento,
            &cfg.modelli.vocabolario,
            device,
            cfg.allineamento.clone(),
            cfg.thread,
        )
        .context("inizializzazione dell'allineatore wav2vec2")?;
        let t = aligner.run(pcm, &testi)?;
        drop(aligner);
        t
    };
    progresso.verifica()?;

    gpu::log_vram(device, "finale");
    Ok(trascrizione)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i_modelli_mancanti_vengono_elencati() {
        let m = PercorsiModelli::nella_cartella("/una/cartella/che/non/esiste");
        assert_eq!(m.mancanti().len(), 4);
    }

    #[test]
    fn i_nomi_dei_modelli_seguono_lo_script_di_esportazione() {
        let m = PercorsiModelli::nella_cartella("models");
        assert!(m.whisper.ends_with("ggml-large-v3.bin"));
        assert!(m.segmentazione.ends_with("pyannote-segmentation-3.0.onnx"));
        assert!(m.allineamento.ends_with("wav2vec2-italian.onnx"));
        assert!(m.vocabolario.ends_with("wav2vec2-italian.vocab.json"));
    }
}
