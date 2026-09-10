//! Verba — sottotitoli automatici in locale.
//!
//! Il motore: dal file di partenza alla trascrizione parola per parola, e da
//! questa ai fotogrammi dei sottotitoli. Non sa che esistono ne' la riga di
//! comando ne' l'applicazione: espone la pipeline come funzioni piu' un canale
//! di eventi di avanzamento.
//!
//! I moduli sono utilizzabili anche separatamente:
//!
//! * [`audio`] — pre-elaborazione (multi-formato, mono, 16 kHz, normalizzazione)
//!   interamente in RAM;
//! * [`segmentation`] — pyannote in ONNX;
//! * [`transcribe`] — Whisper large-v3 via whisper.cpp;
//! * [`align`] — allineamento forzato CTC con wav2vec2-italian in ONNX;
//! * [`trascrizione`] — la sequenza di parole, mutabile e con identificativi
//!   stabili, separata dal risultato grezzo del modello;
//! * [`pulizia`] — normalizzazione della sequenza: e' su questa funzione che si
//!   regge tutto cio' che viene dopo;
//! * [`pipeline`] — l'ordine delle fasi, uno solo per tutti i chiamanti;
//! * [`layout`] — impaginazione dei sottotitoli misurata con cosmic-text;
//! * [`render`] — disegno dei fotogrammi RGBA a sfondo trasparente;
//! * [`encoder`] — encoder ProRes 4444 (libavcodec/libavformat, C++);
//! * [`video`] — esportazione del filmato dei sottotitoli;
//! * [`srt`] — generazione dei sottotitoli testuali (uscita accessoria);
//! * [`eventi`] — avanzamento per fasi e annullamento;
//! * [`gpu`] — selezione del dispositivo e monitoraggio VRAM.

pub mod align;
pub mod audio;
pub mod caratteri;
pub mod encoder;
pub mod eventi;
pub mod gpu;
pub mod layout;
pub mod onnx;
pub mod pipeline;
pub mod prompt;
pub mod pulizia;
pub mod render;
pub mod scena;
pub mod segmentation;
pub mod srt;
pub mod transcribe;
pub mod trascrizione;
pub mod video;

/// Inter peso 700 (statico, `.ttf`), incorporato nel binario.
///
/// Incorporarlo evita che il risultato dipenda dai font installati sulla
/// macchina: lo stesso ingresso produce lo stesso fotogramma ovunque. Con
/// l'opzione `--font` si puo' comunque usare un altro file.
pub const FONT_INTER_BOLD: &[u8] = include_bytes!("../assets/Inter-Bold.ttf");
