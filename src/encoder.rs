//! Interfaccia Rust all'encoder video scritto in C++ (`cpp/encoder.cpp`).
//!
//! La codifica vera e propria avviene in libavcodec/libavformat: qui c'e' solo
//! il ponte FFI e la gestione delle risorse. Il file prodotto e' un MOV con una
//! traccia **ProRes 4444** (`prores_ks`, profilo 4) in `yuva444p10le`, ossia con
//! canale alfa: i sottotitoli si sovrappongono a qualunque video nel montaggio.

use std::ffi::{c_char, c_int, CString};
use std::path::Path;

use anyhow::{anyhow, bail, Result};

const ERRORE_LEN: usize = 512;

#[allow(non_camel_case_types)]
enum SubEncoderC {}

extern "C" {
    fn sub_encoder_apri(
        percorso: *const c_char,
        larghezza: c_int,
        altezza: c_int,
        fps_num: c_int,
        fps_den: c_int,
        qualita: c_int,
        thread: c_int,
        errore: *mut c_char,
        errore_len: c_int,
    ) -> *mut SubEncoderC;

    fn sub_encoder_scrivi(
        enc: *mut SubEncoderC,
        rgba: *const u8,
        passo: c_int,
        ripetizioni: c_int,
        errore: *mut c_char,
        errore_len: c_int,
    ) -> c_int;

    fn sub_encoder_chiudi(enc: *mut SubEncoderC, errore: *mut c_char, errore_len: c_int) -> c_int;

    fn sub_encoder_libera(enc: *mut SubEncoderC);

    fn sub_encoder_frame_scritti(enc: *const SubEncoderC) -> i64;
}

/// Parametri del file video in uscita.
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    pub larghezza: u32,
    pub altezza: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    /// Quantizzatore ProRes (`-qscale`): piu' basso = piu' qualita' e piu' bit.
    pub qualita: u32,
    pub thread: usize,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self { larghezza: 1080, altezza: 1920, fps_num: 30, fps_den: 1, qualita: 4, thread: 0 }
    }
}

/// Encoder aperto su un file. La chiusura ordinata avviene con [`Encoder::chiudi`];
/// il `Drop` libera comunque le risorse anche in caso di errore o panico.
pub struct Encoder {
    ptr: *mut SubEncoderC,
    larghezza: u32,
    altezza: u32,
    chiuso: bool,
}

impl Encoder {
    pub fn apri(percorso: &Path, cfg: &EncoderConfig) -> Result<Self> {
        if cfg.larghezza % 2 != 0 || cfg.altezza % 2 != 0 {
            bail!(
                "risoluzione {}x{}: ProRes richiede larghezza e altezza pari",
                cfg.larghezza,
                cfg.altezza
            );
        }
        let c_path = CString::new(percorso.as_os_str().as_encoded_bytes())
            .map_err(|_| anyhow!("il percorso {} contiene un byte nullo", percorso.display()))?;
        let mut errore = [0u8; ERRORE_LEN];

        // SAFETY: i puntatori vivono per tutta la chiamata; la stringa e' NUL-terminata.
        let ptr = unsafe {
            sub_encoder_apri(
                c_path.as_ptr(),
                cfg.larghezza as c_int,
                cfg.altezza as c_int,
                cfg.fps_num as c_int,
                cfg.fps_den.max(1) as c_int,
                cfg.qualita as c_int,
                cfg.thread as c_int,
                errore.as_mut_ptr() as *mut c_char,
                ERRORE_LEN as c_int,
            )
        };
        if ptr.is_null() {
            bail!("apertura dell'encoder video: {}", messaggio(&errore));
        }
        Ok(Self { ptr, larghezza: cfg.larghezza, altezza: cfg.altezza, chiuso: false })
    }

    /// Scrive un frame RGBA8 (alfa dritta, non premoltiplicata) `ripetizioni` volte.
    ///
    /// I sottotitoli restano identici per decine di fotogrammi consecutivi: la
    /// conversione colore avviene una sola volta e il frame gia' convertito
    /// viene ricodificato.
    pub fn scrivi(&mut self, rgba: &[u8], ripetizioni: u32) -> Result<()> {
        if ripetizioni == 0 {
            return Ok(());
        }
        let attesi = self.larghezza as usize * self.altezza as usize * 4;
        if rgba.len() < attesi {
            bail!("frame RGBA di {} byte, attesi {attesi}", rgba.len());
        }
        let passo = (self.larghezza * 4) as c_int;
        let mut errore = [0u8; ERRORE_LEN];
        // SAFETY: `rgba` e' lungo almeno larghezza*altezza*4 (verificato sopra).
        let ret = unsafe {
            sub_encoder_scrivi(
                self.ptr,
                rgba.as_ptr(),
                passo,
                ripetizioni as c_int,
                errore.as_mut_ptr() as *mut c_char,
                ERRORE_LEN as c_int,
            )
        };
        if ret < 0 {
            bail!("codifica del frame: {}", messaggio(&errore));
        }
        Ok(())
    }

    /// Numero di fotogrammi gia' scritti.
    pub fn frame_scritti(&self) -> i64 {
        // SAFETY: `self.ptr` e' valido finche' l'oggetto esiste.
        unsafe { sub_encoder_frame_scritti(self.ptr) }
    }

    /// Svuota l'encoder e chiude il contenitore. Va chiamata esplicitamente:
    /// il `Drop` non puo' segnalare gli errori di scrittura del trailer.
    pub fn chiudi(mut self) -> Result<()> {
        let mut errore = [0u8; ERRORE_LEN];
        // SAFETY: `self.ptr` e' valido e non ancora chiuso.
        let ret =
            unsafe { sub_encoder_chiudi(self.ptr, errore.as_mut_ptr() as *mut c_char, ERRORE_LEN as c_int) };
        self.chiuso = true;
        if ret < 0 {
            bail!("chiusura del file video: {}", messaggio(&errore));
        }
        Ok(())
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            if !self.chiuso {
                // SAFETY: chiusura di emergenza; l'esito non e' segnalabile qui.
                unsafe { sub_encoder_chiudi(self.ptr, std::ptr::null_mut(), 0) };
            }
            // SAFETY: il puntatore viene liberato una sola volta.
            unsafe { sub_encoder_libera(self.ptr) };
            self.ptr = std::ptr::null_mut();
        }
    }
}

fn messaggio(buf: &[u8]) -> String {
    let fine = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..fine]).into_owned()
}
