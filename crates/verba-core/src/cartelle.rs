//! Dove Verba tiene le sue cose sulla macchina di chi la usa.
//!
//! I modelli non si impacchettano: Whisper large-v3 da solo supera il
//! gigabyte, e un `.deb` di quella dimensione non e' distribuibile. Si
//! scaricano al primo avvio e vivono nella cartella dati dell'utente, che
//! cambia da sistema a sistema:
//!
//! | Sistema | Cartella |
//! |---|---|
//! | Linux | `$XDG_DATA_HOME/verba`, altrimenti `~/.local/share/verba` |
//! | Windows | `%LOCALAPPDATA%\verba` |
//! | macOS | `~/Library/Application Support/verba` |
//!
//! `VERBA_DATA_DIR` ha comunque la precedenza su tutto: serve a chi tiene i
//! modelli su un disco esterno, e serve ai test.

use std::path::PathBuf;

/// Il nome della cartella dell'applicazione dentro la cartella dati.
const NOME: &str = "verba";

/// La cartella dati di Verba. Non viene creata: lo fa chi ci scrive.
pub fn dati() -> PathBuf {
    if let Some(esplicita) = std::env::var_os("VERBA_DATA_DIR") {
        return PathBuf::from(esplicita);
    }
    base().join(NOME)
}

/// I modelli scaricati.
pub fn modelli() -> PathBuf {
    dati().join("models")
}

/// Le librerie native che accompagnano l'applicazione (ONNX Runtime).
pub fn librerie() -> PathBuf {
    dati().join("lib")
}

/// La cartella dell'eseguibile in esecuzione.
///
/// E' il primo posto dove cercare una libreria impacchettata insieme
/// all'applicazione: in un `.AppImage` o in uno zip di Windows le librerie
/// stanno li', non nel sistema.
pub fn accanto_all_eseguibile() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // Se l'eseguibile e' un collegamento simbolico, quello che conta e' dove
    // punta: e' li' che sta il resto del pacchetto.
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    exe.parent().map(PathBuf::from)
}

/// La cartella dati del sistema, senza il nome dell'applicazione.
fn base() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local);
        }
        if let Some(profilo) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(profilo).join("AppData").join("Local");
        }
        return PathBuf::from(".");
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library").join("Application Support");
        }
        return PathBuf::from(".");
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
            let p = PathBuf::from(xdg);
            // Lo standard XDG dice che un percorso relativo va ignorato.
            if p.is_absolute() {
                return p;
            }
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".local").join("share");
        }
        PathBuf::from(".")
    }
}

/// Crea la cartella se non c'e', e la restituisce.
pub fn assicura(cartella: PathBuf) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(&cartella)?;
    Ok(cartella)
}

/// Una dimensione in byte come la scriverebbe una persona.
pub fn dimensione_leggibile(byte: u64) -> String {
    const K: f64 = 1024.0;
    let b = byte as f64;
    if b < K {
        return format!("{byte} B");
    }
    for (soglia, unita) in [(K * K, "kB"), (K * K * K, "MB"), (K * K * K * K, "GB")] {
        if b < soglia {
            let valore = b / (soglia / K);
            let cifre = if valore < 10.0 { 1 } else { 0 };
            return format!("{valore:.cifre$} {unita}");
        }
    }
    format!("{:.1} TB", b / (K * K * K * K))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_dimensioni_si_leggono_come_su_un_sito_di_download() {
        assert_eq!(dimensione_leggibile(512), "512 B");
        assert_eq!(dimensione_leggibile(1024), "1.0 kB");
        assert_eq!(dimensione_leggibile(1536), "1.5 kB");
        assert_eq!(dimensione_leggibile(20 * 1024 * 1024), "20 MB");
        assert_eq!(dimensione_leggibile(3_100_000_000), "2.9 GB");
    }

    #[test]
    fn i_modelli_e_le_librerie_stanno_sotto_la_cartella_dati() {
        let d = dati();
        assert!(modelli().starts_with(&d));
        assert!(librerie().starts_with(&d));
        assert!(modelli() != librerie());
    }
}
