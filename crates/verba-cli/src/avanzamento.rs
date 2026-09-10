//! Come l'avanzamento del motore diventa qualcosa da leggere.
//!
//! Il motore emette eventi e non sa nulla di terminali: qui si decide se
//! diventano righe per una persona o JSON per uno script.

use std::io::Write;
use std::sync::Mutex;

use clap::ValueEnum;
use verba_core::eventi::{Evento, Progresso};

/// Dove va a finire l'avanzamento.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum, Default)]
pub enum Formato {
    /// Una riga per fase, leggibile, su stderr.
    #[default]
    Testo,
    /// Un oggetto JSON per evento, una riga ciascuno, su stderr: e' la forma
    /// che serve a chi integra Verba in un altro script.
    Json,
    /// Nessuna uscita di avanzamento.
    Muto,
}

/// Costruisce il canale di avanzamento per il formato scelto.
pub fn progresso(formato: Formato) -> Progresso {
    match formato {
        Formato::Muto => Progresso::silenzioso(),
        Formato::Json => Progresso::con(|e| {
            if let Ok(riga) = serde_json::to_string(&e) {
                let mut err = std::io::stderr().lock();
                let _ = writeln!(err, "{riga}");
            }
        }),
        Formato::Testo => {
            // L'ultima frazione mostrata, per non riscrivere la stessa
            // percentuale decine di volte.
            let ultima = Mutex::new(-1i32);
            Progresso::con(move |e| {
                let mut err = std::io::stderr().lock();
                match e {
                    Evento::Iniziata { fase } => {
                        *ultima.lock().unwrap() = -1;
                        let _ = writeln!(err, "  ▸ {}", fase.etichetta());
                    }
                    Evento::Avanzamento { fase, frazione } => {
                        let percento = (frazione * 100.0).round() as i32;
                        let mut u = ultima.lock().unwrap();
                        if percento / 10 > *u / 10 {
                            *u = percento;
                            let _ = writeln!(err, "    {} {percento:>3} %", fase.etichetta());
                        }
                    }
                    Evento::Conclusa { fase, secondi } => {
                        let _ = writeln!(
                            err,
                            "  ✓ {}  {}",
                            fase.etichetta(),
                            durata_leggibile(secondi)
                        );
                    }
                    Evento::Avviso { messaggio } => {
                        let _ = writeln!(err, "  ! {messaggio}");
                    }
                    Evento::Annullata => {
                        let _ = writeln!(err, "  ✕ annullata");
                    }
                }
            })
        }
    }
}

/// `2s`, `1m 04s`, `1h 02m`: la stessa forma che usa l'elenco delle fasi
/// nell'applicazione.
pub fn durata_leggibile(secondi: f64) -> String {
    let s = secondi.max(0.0).round() as u64;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, (s % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_durate_si_leggono_come_nell_interfaccia() {
        assert_eq!(durata_leggibile(2.0), "2s");
        assert_eq!(durata_leggibile(64.0), "1m 04s");
        assert_eq!(durata_leggibile(3720.0), "1h 02m");
        assert_eq!(durata_leggibile(-1.0), "0s");
    }

    #[test]
    fn il_formato_muto_non_ha_ascoltatori() {
        // Non deve andare in panico ne' scrivere nulla.
        let p = progresso(Formato::Muto);
        p.avviso("niente");
    }
}
