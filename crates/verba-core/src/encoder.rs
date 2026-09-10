//! Interfaccia Rust all'encoder video scritto in C++ (`cpp/encoder.cpp`).
//!
//! La codifica vera e propria avviene in libavcodec/libavformat: qui c'e' solo
//! il ponte FFI e la gestione delle risorse. I formati sono quattro, due con
//! canale alfa e due senza: vedi [`FormatoVideo`].

use std::ffi::{c_char, c_int, CStr, CString};
use std::path::Path;

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};

const ERRORE_LEN: usize = 512;

#[allow(non_camel_case_types)]
enum SubEncoderC {}

extern "C" {
    fn sub_formato_ha_alfa(formato: c_int) -> c_int;

    fn sub_formato_estensione(formato: c_int) -> *const c_char;

    fn sub_encoder_apri(
        percorso: *const c_char,
        formato: c_int,
        audio_da: *const c_char,
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

/// I formati di uscita, negli stessi valori dell'enumerazione C.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(i32)]
pub enum FormatoVideo {
    /// MOV, ProRes 4444 in `yuva444p10le`. Sottotitoli su sfondo trasparente,
    /// da sovrapporre in montaggio.
    #[default]
    Prores4444 = 0,
    /// MOV, ProRes 422 HQ. Video sottotitolato senza perdita, per chi rimonta.
    Prores422 = 1,
    /// MP4, H.264 in `yuv420p`. Il video che si puo' dare a chiunque.
    H264 = 2,
    /// WebM, VP9 con alfa. Un overlay di due ordini di grandezza piu' leggero
    /// del ProRes, al prezzo di una codifica piu' lenta.
    Vp9Alpha = 3,
}

impl FormatoVideo {
    /// Vero se il formato trasporta il canale alfa, cioe' se produce un
    /// overlay invece di un video finito.
    pub fn ha_alfa(self) -> bool {
        unsafe { sub_formato_ha_alfa(self as c_int) != 0 }
    }

    /// L'estensione del file, senza il punto.
    pub fn estensione(self) -> &'static str {
        let p = unsafe { sub_formato_estensione(self as c_int) };
        if p.is_null() {
            return "";
        }
        unsafe { CStr::from_ptr(p) }.to_str().unwrap_or("")
    }

    /// Il suffisso da aggiungere al nome del file di partenza.
    pub fn suffisso(self) -> &'static str {
        if self.ha_alfa() {
            "_overlay"
        } else {
            "_sub"
        }
    }

    /// Come si chiama nell'elenco dei formati.
    pub fn etichetta(self) -> &'static str {
        match self {
            FormatoVideo::Prores4444 => "Overlay trasparente",
            FormatoVideo::Prores422 => "Video sottotitolato senza perdita",
            FormatoVideo::H264 => "Video sottotitolato",
            FormatoVideo::Vp9Alpha => "Overlay trasparente compatto",
        }
    }

    /// Una riga che dice a chi serve.
    pub fn descrizione(self) -> &'static str {
        match self {
            FormatoVideo::Prores4444 => {
                "ProRes 4444 con canale alfa: solo i sottotitoli, da sovrapporre in montaggio"
            }
            FormatoVideo::Prores422 => {
                "ProRes 422 HQ: i sottotitoli impressi, senza perdita, per rimontare"
            }
            FormatoVideo::H264 => "H.264 CRF 18: i sottotitoli impressi, si riproduce ovunque",
            FormatoVideo::Vp9Alpha => {
                "VP9 con alfa: come l'overlay ProRes ma centinaia di volte piu' leggero, e piu' lento da produrre"
            }
        }
    }

    pub const TUTTI: [FormatoVideo; 4] = [
        FormatoVideo::H264,
        FormatoVideo::Prores422,
        FormatoVideo::Prores4444,
        FormatoVideo::Vp9Alpha,
    ];
}

/// Parametri del file video in uscita.
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    pub formato: FormatoVideo,
    pub larghezza: u32,
    pub altezza: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    /// Quantizzatore per i ProRes, CRF per H.264 e VP9: in entrambi i casi
    /// piu' basso = piu' qualita' e piu' bit. Zero = il valore consigliato.
    pub qualita: u32,
    pub thread: usize,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            formato: FormatoVideo::Prores4444,
            larghezza: 1080,
            altezza: 1920,
            fps_num: 30,
            fps_den: 1,
            qualita: 0,
            thread: 0,
        }
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
    /// Apre l'encoder senza copiare alcuna traccia audio.
    pub fn apri(percorso: &Path, cfg: &EncoderConfig) -> Result<Self> {
        Self::apri_con_audio(percorso, cfg, None)
    }

    /// Apre l'encoder copiando la traccia audio da `audio_da`.
    ///
    /// La copia e' un rimultiplexaggio: nessuna ricodifica, nessuna perdita.
    /// Vale solo per i formati senza alfa — un overlay trasparente non porta
    /// audio, altrimenti in montaggio ci si ritroverebbe la stessa traccia due
    /// volte.
    pub fn apri_con_audio(
        percorso: &Path,
        cfg: &EncoderConfig,
        audio_da: Option<&Path>,
    ) -> Result<Self> {
        if cfg.larghezza % 2 != 0 || cfg.altezza % 2 != 0 {
            bail!(
                "risoluzione {}x{}: gli encoder richiedono larghezza e altezza pari",
                cfg.larghezza,
                cfg.altezza
            );
        }
        let c_path = CString::new(percorso.as_os_str().as_encoded_bytes())
            .map_err(|_| anyhow!("il percorso {} contiene un byte nullo", percorso.display()))?;
        let c_audio = match audio_da {
            Some(p) => Some(
                CString::new(p.as_os_str().as_encoded_bytes())
                    .map_err(|_| anyhow!("il percorso {} contiene un byte nullo", p.display()))?,
            ),
            None => None,
        };
        let mut errore = [0u8; ERRORE_LEN];

        // SAFETY: i puntatori vivono per tutta la chiamata; la stringa e' NUL-terminata.
        let ptr = unsafe {
            sub_encoder_apri(
                c_path.as_ptr(),
                cfg.formato as c_int,
                c_audio.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
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
