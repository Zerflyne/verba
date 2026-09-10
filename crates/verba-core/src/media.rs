//! Il file di partenza: cosa contiene, e il fotogramma a un dato istante.
//!
//! Verba si comporta in modo diverso a seconda di cosa viene caricato, e la
//! distinzione nasce qui. Un file audio non ha nulla da disegnare sotto i
//! sottotitoli; un file video ha proporzioni, durata e frame rate propri, e
//! serve da anteprima e da base per l'export.
//!
//! L'audio continua a passare da [`crate::audio`], che decodifica in RAM tutto
//! cio' che Symphonia sa leggere — contenitori video compresi. Qui si legge
//! solo cio' che serve *in piu'* per un video: le proporzioni e i fotogrammi.

use std::ffi::{c_char, c_double, c_int, CStr, CString};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use tracing::debug;

const ERRORE_MAX: usize = 512;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VerbaInfoC {
    ha_video: c_int,
    ha_audio: c_int,
    larghezza: c_int,
    altezza: c_int,
    fps_num: c_int,
    fps_den: c_int,
    durata: c_double,
    codec_video: [c_char; 32],
    codec_audio: [c_char; 32],
}

#[repr(C)]
struct VerbaMediaC {
    _opaco: [u8; 0],
}

extern "C" {
    fn verba_media_apri(
        percorso: *const c_char,
        info: *mut VerbaInfoC,
        errore: *mut c_char,
        errore_len: c_int,
    ) -> *mut VerbaMediaC;

    fn verba_media_fotogramma(
        m: *mut VerbaMediaC,
        t: c_double,
        rgba: *mut u8,
        passo: c_int,
        errore: *mut c_char,
        errore_len: c_int,
    ) -> c_int;

    fn verba_media_libera(m: *mut VerbaMediaC);
}

/// Come Verba si comporta con questo file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Modalita {
    /// Solo audio: non c'e' niente da disegnare sotto i sottotitoli, e le
    /// impostazioni grafiche di stile e posizione non hanno un riferimento.
    Audio,
    /// Video: l'audio viene estratto e trascritto, il video fa da anteprima e
    /// da base per l'export.
    Video,
}

/// Cio' che si sa del file senza decodificarlo.
#[derive(Debug, Clone, Serialize)]
pub struct Informazioni {
    pub modalita: Modalita,
    pub larghezza: u32,
    pub altezza: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    /// Durata in secondi; 0 se il contenitore non la dichiara.
    pub durata: f64,
    pub codec_video: Option<String>,
    pub codec_audio: Option<String>,
}

impl Informazioni {
    pub fn e_video(&self) -> bool {
        self.modalita == Modalita::Video
    }

    pub fn fps(&self) -> f64 {
        if self.fps_den == 0 {
            return 0.0;
        }
        self.fps_num as f64 / self.fps_den as f64
    }

    /// La risoluzione, se c'e' un video.
    pub fn risoluzione(&self) -> Option<(u32, u32)> {
        (self.e_video() && self.larghezza > 0 && self.altezza > 0)
            .then_some((self.larghezza, self.altezza))
    }

    /// Una riga da mostrare accanto al nome del file.
    pub fn descrizione(&self) -> String {
        let durata = if self.durata > 0.0 {
            format!("{:02}:{:02}", (self.durata as u64) / 60, (self.durata as u64) % 60)
        } else {
            "durata sconosciuta".to_string()
        };
        match self.modalita {
            Modalita::Video => format!(
                "{}x{} · {:.2} fps · {durata}",
                self.larghezza,
                self.altezza,
                self.fps()
            ),
            Modalita::Audio => format!(
                "audio {} · {durata}",
                self.codec_audio.as_deref().unwrap_or("sconosciuto")
            ),
        }
    }
}

/// Un file aperto, da cui si possono chiedere fotogrammi.
pub struct Media {
    ptr: *mut VerbaMediaC,
    info: Informazioni,
}

impl std::fmt::Debug for Media {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Media").field("info", &self.info).finish()
    }
}

// Il contesto di libavformat non e' condiviso fra thread, ma spostarlo da un
// thread all'altro e' lecito: il puntatore appartiene a questa struttura e
// nessun altro lo tocca.
unsafe impl Send for Media {}

impl Media {
    /// Apre un file e ne legge le caratteristiche.
    pub fn apri(percorso: &Path) -> Result<Self> {
        let c_percorso = CString::new(percorso.as_os_str().as_encoded_bytes())
            .with_context(|| format!("percorso non valido: {}", percorso.display()))?;
        let mut info = VerbaInfoC {
            ha_video: 0,
            ha_audio: 0,
            larghezza: 0,
            altezza: 0,
            fps_num: 0,
            fps_den: 0,
            durata: 0.0,
            codec_video: [0; 32],
            codec_audio: [0; 32],
        };
        let mut errore = [ZERO; ERRORE_MAX];

        let ptr = unsafe {
            verba_media_apri(
                c_percorso.as_ptr(),
                &mut info,
                errore.as_mut_ptr(),
                ERRORE_MAX as c_int,
            )
        };
        if ptr.is_null() {
            bail!("{}: {}", percorso.display(), messaggio(&errore));
        }

        let informazioni = Informazioni {
            modalita: if info.ha_video != 0 { Modalita::Video } else { Modalita::Audio },
            larghezza: info.larghezza.max(0) as u32,
            altezza: info.altezza.max(0) as u32,
            fps_num: info.fps_num.max(0) as u32,
            fps_den: info.fps_den.max(0) as u32,
            durata: info.durata.max(0.0),
            codec_video: (info.ha_video != 0).then(|| stringa(&info.codec_video)),
            codec_audio: (info.ha_audio != 0).then(|| stringa(&info.codec_audio)),
        };

        if info.ha_audio == 0 {
            unsafe { verba_media_libera(ptr) };
            bail!(
                "{}: il file non contiene audio. Serve un file con una traccia audio.",
                percorso.display()
            );
        }

        debug!(
            file = %percorso.display(),
            modalita = ?informazioni.modalita,
            descrizione = %informazioni.descrizione(),
            "file aperto"
        );
        Ok(Self { ptr, info: informazioni })
    }

    pub fn informazioni(&self) -> &Informazioni {
        &self.info
    }

    /// Il fotogramma visibile al tempo `t`, in RGBA opaco.
    ///
    /// `pixel` deve essere lungo `larghezza * altezza * 4`. Ritorna `false` se
    /// `t` cade oltre la fine del video, nel qual caso `pixel` non viene
    /// toccato.
    pub fn fotogramma(&mut self, t: f64, pixel: &mut [u8]) -> Result<bool> {
        let attesi = self.info.larghezza as usize * self.info.altezza as usize * 4;
        if !self.info.e_video() {
            bail!("il file non ha una traccia video");
        }
        if pixel.len() < attesi {
            bail!("servono {attesi} byte per il fotogramma, ne sono stati dati {}", pixel.len());
        }

        let mut errore = [ZERO; ERRORE_MAX];
        let esito = unsafe {
            verba_media_fotogramma(
                self.ptr,
                t,
                pixel.as_mut_ptr(),
                (self.info.larghezza as c_int) * 4,
                errore.as_mut_ptr(),
                ERRORE_MAX as c_int,
            )
        };
        match esito {
            0 => Ok(true),
            1 => Ok(false),
            _ => bail!("{}", messaggio(&errore)),
        }
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { verba_media_libera(self.ptr) };
            self.ptr = std::ptr::null_mut();
        }
    }
}

/// Zero come `c_char`, che su alcune architetture e' `u8` e su altre `i8`.
const ZERO: c_char = 0;

fn messaggio(buf: &[c_char]) -> String {
    let testo = unsafe { CStr::from_ptr(buf.as_ptr()) };
    let s = testo.to_string_lossy().into_owned();
    if s.is_empty() {
        "errore sconosciuto".to_string()
    } else {
        s
    }
}

fn stringa(buf: &[c_char; 32]) -> String {
    messaggio(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_file_inesistente_da_errore_col_percorso() {
        let errore = Media::apri(Path::new("/non/esiste.mp4")).unwrap_err().to_string();
        assert!(errore.contains("/non/esiste.mp4"), "{errore}");
    }

    #[test]
    fn un_file_che_non_e_un_media_viene_rifiutato() {
        let errore = Media::apri(Path::new("Cargo.toml")).unwrap_err().to_string();
        assert!(errore.contains("Cargo.toml"), "{errore}");
    }

    #[test]
    fn la_descrizione_di_un_video_dice_proporzioni_e_durata() {
        let i = Informazioni {
            modalita: Modalita::Video,
            larghezza: 1920,
            altezza: 1080,
            fps_num: 30000,
            fps_den: 1001,
            durata: 157.0,
            codec_video: Some("h264".into()),
            codec_audio: Some("aac".into()),
        };
        let d = i.descrizione();
        assert!(d.contains("1920x1080"), "{d}");
        assert!(d.contains("29.97"), "{d}");
        assert!(d.contains("02:37"), "{d}");
        assert_eq!(i.risoluzione(), Some((1920, 1080)));
    }

    #[test]
    fn un_file_audio_non_ha_risoluzione() {
        let i = Informazioni {
            modalita: Modalita::Audio,
            larghezza: 0,
            altezza: 0,
            fps_num: 0,
            fps_den: 0,
            durata: 9.6,
            codec_video: None,
            codec_audio: Some("mp3".into()),
        };
        assert!(i.risoluzione().is_none());
        assert!(!i.e_video());
        assert!(i.descrizione().contains("mp3"));
    }

    #[test]
    fn un_mp3_e_in_modalita_audio() {
        let m = Media::apri(Path::new("../../prova.mp3"));
        // Il campione non e' versionato: se non c'e' il test non ha nulla da
        // dire, ma quando c'e' deve dire la cosa giusta.
        if let Ok(m) = m {
            assert_eq!(m.informazioni().modalita, Modalita::Audio);
            assert!(m.informazioni().durata > 0.0);
            assert!(m.informazioni().risoluzione().is_none());
        }
    }
}
