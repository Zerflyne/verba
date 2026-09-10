//! Le impostazioni che l'applicazione ricorda fra un avvio e l'altro.
//!
//! Non sono un preset: un preset descrive **come vengono disegnati** i
//! sottotitoli e si scambia con altre persone, queste descrivono **come e'
//! configurata questa macchina** — quale modello, quale dispositivo, dove
//! salvare, quali caratteri sono stati aggiunti a mano. Le due cose stanno in
//! file separati per la stessa ragione per cui un preset non contiene
//! riferimenti a file: perche' altrimenti non si potrebbe mandarlo a nessuno.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::cartelle;
use crate::modelli::Dimensione;

/// Su cosa calcolare.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dispositivo {
    /// GPU se c'e' e se regge, altrimenti CPU. Senza errori bloccanti.
    #[default]
    Automatico,
    Gpu,
    Cpu,
}

/// Il nome del file dentro la cartella dati.
const FILE: &str = "impostazioni.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Impostazioni {
    pub versione: u32,
    /// Dimensione del modello di trascrizione.
    pub modello: Dimensione,
    /// Lingua ISO-639-1, oppure `auto`.
    pub lingua: String,
    pub dispositivo: Dispositivo,
    /// Il CSV dei termini noti, se ne e' stato caricato uno.
    pub termini: Option<PathBuf>,
    /// Sotto questa confidenza una parola viene segnalata.
    pub soglia: f32,
    /// Dove proporre gli export. Vuoto = accanto al file di partenza.
    pub cartella_export: Option<PathBuf>,
    /// Dove tenere i modelli, se non nella cartella predefinita.
    pub cartella_modelli: Option<PathBuf>,
    /// I file di carattere che l'utente ha aggiunto: si e' scaricato un
    /// `.ttf` e ce lo ha messo. Vengono ricaricati a ogni avvio.
    pub caratteri_aggiunti: Vec<PathBuf>,
    /// Cercare anche fra i caratteri installati sul sistema.
    pub caratteri_di_sistema: bool,
    /// L'ultimo formato scelto nella sezione Esporta.
    pub ultimo_formato: Option<String>,
}

impl Default for Impostazioni {
    fn default() -> Self {
        Self {
            versione: 1,
            modello: Dimensione::default(),
            lingua: "it".to_string(),
            dispositivo: Dispositivo::default(),
            termini: None,
            soglia: 0.5,
            cartella_export: None,
            cartella_modelli: None,
            caratteri_aggiunti: Vec::new(),
            caratteri_di_sistema: false,
            ultimo_formato: None,
        }
    }
}

impl Impostazioni {
    /// Il file in cui vivono.
    pub fn percorso() -> PathBuf {
        cartelle::dati().join(FILE)
    }

    /// Le carica, o restituisce i valori predefiniti.
    ///
    /// Un file illeggibile non e' un motivo per non far partire
    /// l'applicazione: si riparte dai valori predefiniti e lo si dice.
    pub fn carica() -> Self {
        let percorso = Self::percorso();
        match std::fs::read_to_string(&percorso) {
            Ok(testo) => match serde_json::from_str::<Self>(&testo) {
                Ok(i) => i,
                Err(e) => {
                    tracing::warn!(
                        file = %percorso.display(),
                        errore = %e,
                        "impostazioni illeggibili: riparto da quelle predefinite"
                    );
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    /// Le scrive, creando la cartella se serve.
    pub fn salva(&self) -> Result<()> {
        let percorso = Self::percorso();
        if let Some(dir) = percorso.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("creazione di {}", dir.display()))?;
        }
        let testo = serde_json::to_string_pretty(self)?;
        std::fs::write(&percorso, testo)
            .with_context(|| format!("scrittura di {}", percorso.display()))?;
        info!(file = %percorso.display(), "impostazioni salvate");
        Ok(())
    }

    /// Aggiunge un file di carattere, se non c'e' gia'.
    ///
    /// Ritorna vero se l'elenco e' cambiato: e' il segnale per ricostruire il
    /// catalogo dei caratteri.
    pub fn aggiungi_carattere(&mut self, file: &Path) -> bool {
        let file = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        if self.caratteri_aggiunti.contains(&file) {
            return false;
        }
        self.caratteri_aggiunti.push(file);
        true
    }

    /// Toglie un carattere aggiunto a mano. Il file su disco resta dov'e': non
    /// e' Verba che ce lo ha messo, e non tocca a Verba cancellarlo.
    pub fn togli_carattere(&mut self, file: &Path) -> bool {
        let prima = self.caratteri_aggiunti.len();
        self.caratteri_aggiunti.retain(|c| c != file);
        self.caratteri_aggiunti.len() != prima
    }

    /// La cartella dei modelli secondo queste impostazioni.
    pub fn modelli(&self) -> PathBuf {
        self.cartella_modelli.clone().unwrap_or_else(cartelle::modelli)
    }

    /// Forza la CPU?
    pub fn solo_cpu(&self) -> bool {
        self.dispositivo == Dispositivo::Cpu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i_valori_predefiniti_sono_quelli_della_spec() {
        let i = Impostazioni::default();
        assert_eq!(i.modello, Dimensione::LargeV3);
        assert_eq!(i.lingua, "it");
        assert_eq!(i.dispositivo, Dispositivo::Automatico);
        assert_eq!(i.soglia, 0.5);
        assert!(i.termini.is_none());
    }

    #[test]
    fn un_json_incompleto_si_legge_lo_stesso() {
        // Un file scritto da una versione precedente non deve impedire
        // l'avvio: i campi che mancano prendono il valore predefinito.
        let i: Impostazioni = serde_json::from_str(r#"{"lingua":"en"}"#).unwrap();
        assert_eq!(i.lingua, "en");
        assert_eq!(i.modello, Dimensione::LargeV3);
        assert_eq!(i.soglia, 0.5);
    }

    #[test]
    fn un_carattere_non_si_aggiunge_due_volte() {
        let mut i = Impostazioni::default();
        let f = std::env::temp_dir().join("verba-carattere-di-prova.ttf");
        std::fs::write(&f, b"non e' un vero font").unwrap();
        assert!(i.aggiungi_carattere(&f));
        assert!(!i.aggiungi_carattere(&f));
        assert_eq!(i.caratteri_aggiunti.len(), 1);
        assert!(i.togli_carattere(&i.caratteri_aggiunti[0].clone()));
        assert!(i.caratteri_aggiunti.is_empty());
        std::fs::remove_file(&f).ok();
    }

    #[test]
    fn il_giro_json_non_perde_niente() {
        let i = Impostazioni {
            lingua: "auto".into(),
            dispositivo: Dispositivo::Cpu,
            ultimo_formato: Some("h264".into()),
            ..Default::default()
        };
        let testo = serde_json::to_string(&i).unwrap();
        assert_eq!(serde_json::from_str::<Impostazioni>(&testo).unwrap(), i);
    }
}
