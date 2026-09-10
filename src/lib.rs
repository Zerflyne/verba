//! AutoSubtitler — trascrizione audio parola-per-parola e sottotitoli grafici
//! con sfondo trasparente.
//!
//! I moduli sono utilizzabili anche separatamente:
//!
//! * [`audio`] — pre-elaborazione (multi-formato, mono, 16 kHz, normalizzazione)
//!   interamente in RAM;
//! * [`segmentation`] — pyannote in ONNX;
//! * [`transcribe`] — Whisper large-v3 via whisper.cpp;
//! * [`align`] — allineamento forzato CTC con wav2vec2-italian in ONNX;
//! * [`layout`] — impaginazione dei sottotitoli misurata con cosmic-text;
//! * [`render`] — disegno dei fotogrammi RGBA a sfondo trasparente;
//! * [`encoder`] — encoder ProRes 4444 (libavcodec/libavformat, C++);
//! * [`video`] — esportazione del filmato dei sottotitoli;
//! * [`srt`] — generazione dei sottotitoli testuali (uscita accessoria);
//! * [`gpu`] — selezione del dispositivo e monitoraggio VRAM.

pub mod align;
pub mod audio;
pub mod encoder;
pub mod gpu;
pub mod layout;
pub mod onnx;
pub mod prompt;
pub mod render;
pub mod segmentation;
pub mod srt;
pub mod transcribe;
pub mod video;

/// Inter peso 700 (statico, `.ttf`), incorporato nel binario.
///
/// Incorporarlo evita che il risultato dipenda dai font installati sulla
/// macchina: lo stesso ingresso produce lo stesso fotogramma ovunque. Con
/// l'opzione `--font` si puo' comunque usare un altro file.
pub const FONT_INTER_BOLD: &[u8] = include_bytes!("../assets/Inter-Bold.ttf");
