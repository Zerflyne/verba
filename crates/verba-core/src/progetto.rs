//! Preset: l'aspetto dei sottotitoli, salvato e ricaricato.
//!
//! Un preset contiene **solo** cio' che riguarda l'aspetto — testo, colori,
//! evidenziazione, posizione, tempi. Non contiene le impostazioni del modello
//! ne' riferimenti a file: le prime perche' cambiarle vorrebbe dire
//! ritrascrivere, e non e' cio' che si chiede a un preset; i secondi perche'
//! un preset deve poter passare da una macchina all'altra.
//!
//! Per la stessa ragione qui non c'e' la risoluzione ma il **formato**: un
//! preset verticale deve funzionare tanto su un 1080x1920 quanto su un
//! 720x1280.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::caratteri::{self, Richiesta};
use crate::layout::{Allineamento, Attivazione, Formato, LayoutConfig, RIGHE_MAX_CONSENTITE};
use crate::render::{Colore, Evidenziazione, Stile};

/// La versione del formato del file. Serve a poter cambiare idea in futuro
/// senza che un preset vecchio venga letto come se fosse nuovo.
pub const VERSIONE: u32 = 1;

/// Il formato del fotogramma richiesto dal preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatoPreset {
    /// 9:16 verticale.
    Verticale,
    /// 16:9 orizzontale.
    Orizzontale,
    /// Le proporzioni del file di partenza.
    #[default]
    DalSorgente,
}

impl FormatoPreset {
    /// La risoluzione da usare, date le proporzioni del sorgente se note.
    pub fn risoluzione(self, sorgente: Option<(u32, u32)>) -> (u32, u32) {
        match self {
            FormatoPreset::Verticale => Formato::Verticale.risoluzione(),
            FormatoPreset::Orizzontale => Formato::Orizzontale.risoluzione(),
            FormatoPreset::DalSorgente => sorgente.unwrap_or_else(|| {
                // Senza sorgente (un file audio) il verticale e' la scelta
                // piu' probabile: i sottotitoli generati finiscono sui social.
                Formato::Verticale.risoluzione()
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Testo {
    pub carattere: String,
    pub peso: u16,
    /// Corpo in pixel riferiti all'altezza del fotogramma. `None` = automatico.
    #[serde(default)]
    pub corpo: Option<f32>,
    #[serde(default)]
    pub maiuscole: bool,
    pub interlinea: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Colori {
    pub testo: String,
    pub testo_attivo: String,
    pub evidenziazione: String,
    pub bordo: String,
    /// Spessore del contorno in pixel.
    pub bordo_px: f32,
    pub ombra: bool,
    pub colore_ombra: String,
    pub ombra_spostamento: f32,
    pub ombra_sfocatura: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidenzia {
    pub forma: Evidenziazione,
    pub padding: f32,
    pub altezza: f32,
    pub raggio: f32,
    pub spessore_sottolineatura: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Posizione {
    pub formato: FormatoPreset,
    pub verticale: f32,
    pub orizzontale: f32,
    pub larghezza_max: f32,
    pub margine: f32,
    pub righe_max: usize,
    pub allineamento: Allineamento,
}

/// I quattro parametri temporali della spec, piu' quelli del blocco.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tempi {
    /// Quanto l'evidenziazione precede l'inizio della parola.
    pub anticipo_ms: u32,
    /// Tetto alla permanenza nella pausa che segue la parola.
    pub pausa_massima_ms: u32,
    /// Permanenza dopo l'ultima parola del blocco.
    pub coda_ms: u32,
    /// Durata minima attribuita a una parola.
    pub durata_minima_parola_ms: u32,
    /// Durata massima di un blocco, in secondi.
    pub durata_blocco: f64,
    /// Una pausa piu' lunga di questo chiude il blocco, in secondi.
    pub pausa_blocco: f64,
    /// Permanenza del blocco dopo l'ultima parola, in secondi.
    pub tenuta: f64,
}

/// L'aspetto dei sottotitoli, salvabile e ricaricabile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    #[serde(default = "versione_predefinita")]
    pub versione: u32,
    pub nome: String,
    pub testo: Testo,
    pub colori: Colori,
    pub evidenziazione: Evidenzia,
    pub posizione: Posizione,
    pub tempi: Tempi,
}

fn versione_predefinita() -> u32 {
    VERSIONE
}

impl Default for Preset {
    fn default() -> Self {
        Self::da(
            "Personalizzato",
            &LayoutConfig::default(),
            &Stile::default(),
            &Richiesta::default(),
            FormatoPreset::DalSorgente,
        )
    }
}

impl Preset {
    /// Costruisce un preset dalle impostazioni correnti.
    ///
    /// Della richiesta del carattere si tiene famiglia e peso, non il file: un
    /// percorso non sopravvive al passaggio a un'altra macchina.
    pub fn da(
        nome: &str,
        layout: &LayoutConfig,
        stile: &Stile,
        carattere: &Richiesta,
        formato: FormatoPreset,
    ) -> Self {
        Self {
            versione: VERSIONE,
            nome: nome.to_string(),
            testo: Testo {
                carattere: carattere.famiglia.clone(),
                peso: carattere.peso,
                corpo: layout.dimensione_font,
                maiuscole: layout.maiuscole,
                interlinea: layout.interlinea,
            },
            colori: Colori {
                testo: stile.colore.esadecimale(),
                testo_attivo: stile.colore_attivo.esadecimale(),
                evidenziazione: stile.colore_evidenziazione.esadecimale(),
                bordo: stile.colore_bordo.esadecimale(),
                bordo_px: stile.bordo,
                ombra: stile.ombra,
                colore_ombra: stile.colore_ombra.esadecimale(),
                ombra_spostamento: stile.ombra_spostamento,
                ombra_sfocatura: stile.ombra_sfocatura,
            },
            evidenziazione: Evidenzia {
                forma: stile.evidenziazione,
                padding: stile.padding,
                altezza: stile.altezza,
                raggio: stile.raggio,
                spessore_sottolineatura: stile.spessore_sottolineatura,
            },
            posizione: Posizione {
                formato,
                verticale: layout.posizione_verticale,
                orizzontale: layout.posizione_orizzontale,
                larghezza_max: layout.larghezza_max,
                margine: layout.margine,
                righe_max: layout.righe_max,
                allineamento: layout.allineamento,
            },
            tempi: Tempi {
                anticipo_ms: (layout.attivazione.anticipo * 1000.0).round().max(0.0) as u32,
                pausa_massima_ms: (layout.attivazione.pausa_max * 1000.0).round().max(0.0) as u32,
                coda_ms: (layout.attivazione.coda * 1000.0).round().max(0.0) as u32,
                durata_minima_parola_ms: (crate::pulizia::DURATA_MINIMA_PAROLA * 1000.0).round()
                    as u32,
                durata_blocco: layout.durata_max,
                pausa_blocco: layout.pausa_max,
                tenuta: layout.tenuta,
            },
        }
    }

    /// La configurazione di impaginazione, data la risoluzione da usare.
    pub fn layout(&self, sorgente: Option<(u32, u32)>) -> LayoutConfig {
        let (larghezza, altezza) = self.posizione.formato.risoluzione(sorgente);
        LayoutConfig {
            larghezza,
            altezza,
            margine: self.posizione.margine,
            larghezza_max: self.posizione.larghezza_max,
            posizione_verticale: self.posizione.verticale,
            posizione_orizzontale: self.posizione.orizzontale,
            righe_max: self.posizione.righe_max.clamp(1, RIGHE_MAX_CONSENTITE),
            allineamento: self.posizione.allineamento,
            maiuscole: self.testo.maiuscole,
            dimensione_font: self.testo.corpo,
            interlinea: self.testo.interlinea,
            durata_max: self.tempi.durata_blocco,
            pausa_max: self.tempi.pausa_blocco,
            tenuta: self.tempi.tenuta,
            attivazione: Attivazione {
                anticipo: self.tempi.anticipo_ms as f64 / 1000.0,
                pausa_max: self.tempi.pausa_massima_ms as f64 / 1000.0,
                coda: self.tempi.coda_ms as f64 / 1000.0,
            },
        }
    }

    /// Lo stile di disegno. Un colore scritto male e' un errore del file, non
    /// qualcosa da far emergere a meta' della codifica.
    pub fn stile(&self) -> Result<Stile> {
        let leggi = |nome: &str, valore: &str| -> Result<Colore> {
            Colore::da_esadecimale(valore)
                .map_err(|e| anyhow::anyhow!("colore «{nome}» del preset: {e}"))
        };
        Ok(Stile {
            colore: leggi("testo", &self.colori.testo)?,
            colore_attivo: leggi("testo_attivo", &self.colori.testo_attivo)?,
            colore_evidenziazione: leggi("evidenziazione", &self.colori.evidenziazione)?,
            colore_bordo: leggi("bordo", &self.colori.bordo)?,
            bordo: self.colori.bordo_px.max(0.0),
            evidenziazione: self.evidenziazione.forma,
            padding: self.evidenziazione.padding.max(0.0),
            altezza: self.evidenziazione.altezza.max(0.0),
            raggio: self.evidenziazione.raggio.max(0.0),
            spessore_sottolineatura: self.evidenziazione.spessore_sottolineatura.max(0.0),
            ombra: self.colori.ombra,
            colore_ombra: leggi("ombra", &self.colori.colore_ombra)?,
            ombra_spostamento: self.colori.ombra_spostamento.max(0.0),
            ombra_sfocatura: self.colori.ombra_sfocatura.max(0.0),
        })
    }

    /// Il carattere richiesto.
    pub fn carattere(&self) -> Richiesta {
        Richiesta {
            famiglia: self.testo.carattere.clone(),
            peso: self.testo.peso,
            file: None,
        }
    }

    /// La durata minima da passare alla normalizzazione, in secondi.
    pub fn durata_minima_parola(&self) -> f64 {
        self.tempi.durata_minima_parola_ms as f64 / 1000.0
    }

    pub fn da_json(testo: &str) -> Result<Self> {
        let preset: Preset =
            serde_json::from_str(testo).context("il file non e' un preset di Verba valido")?;
        if preset.versione > VERSIONE {
            anyhow::bail!(
                "il preset e' in versione {} e questa copia di Verba arriva alla {VERSIONE}: \
                 serve una versione piu' recente",
                preset.versione
            );
        }
        Ok(preset)
    }

    pub fn in_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("serializzazione del preset")
    }

    pub fn carica(percorso: &Path) -> Result<Self> {
        let testo = std::fs::read_to_string(percorso)
            .with_context(|| format!("lettura del preset {}", percorso.display()))?;
        Self::da_json(&testo)
            .with_context(|| format!("lettura del preset {}", percorso.display()))
    }

    pub fn salva(&self, percorso: &Path) -> Result<()> {
        std::fs::write(percorso, self.in_json()?)
            .with_context(|| format!("scrittura del preset {}", percorso.display()))
    }
}

/// I preset di serie.
///
/// Sono anche la vetrina di cosa sa fare l'applicazione: chi apre Verba per la
/// prima volta deve poter vedere tre risultati diversi senza toccare un
/// cursore.
pub fn di_serie() -> Vec<Preset> {
    vec![verticale(), orizzontale(), sobrio()]
}

/// Quello che ci si aspetta dai social in verticale: corto, grosso, centrato
/// in basso, con il rettangolo viola.
pub fn verticale() -> Preset {
    let layout = LayoutConfig { ..Default::default() };
    Preset::da(
        "Verticale",
        &layout,
        &Stile::default(),
        &Richiesta::default(),
        FormatoPreset::Verticale,
    )
}

/// Per il 16:9: due righe, corpo piu' contenuto, piu' in basso.
pub fn orizzontale() -> Preset {
    let (larghezza, altezza) = Formato::Orizzontale.risoluzione();
    let layout = LayoutConfig {
        larghezza,
        altezza,
        righe_max: 2,
        posizione_verticale: 0.86,
        larghezza_max: 0.72,
        ..Default::default()
    };
    Preset::da(
        "Orizzontale",
        &layout,
        &Stile::default(),
        &Richiesta::default(),
        FormatoPreset::Orizzontale,
    )
}

/// Solo testo bianco con contorno: nessuna evidenziazione. E' il preset per
/// chi vuole sottotitoli e basta.
pub fn sobrio() -> Preset {
    let stile = Stile {
        evidenziazione: Evidenziazione::Nessuna,
        bordo: 3.0,
        ombra: true,
        ..Default::default()
    };
    Preset::da(
        "Sobrio",
        &LayoutConfig { righe_max: 2, ..Default::default() },
        &stile,
        &Richiesta {
            famiglia: caratteri::FAMIGLIA_PREDEFINITA.to_string(),
            peso: 700,
            file: None,
        },
        FormatoPreset::DalSorgente,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_preset_sopravvive_al_giro_in_json() {
        for p in di_serie() {
            let json = p.in_json().unwrap();
            let riletto = Preset::da_json(&json).unwrap();
            assert_eq!(p, riletto, "il preset «{}» non e' tornato indietro uguale", p.nome);
        }
    }

    #[test]
    fn il_preset_ricostruisce_layout_e_stile() {
        let layout = LayoutConfig {
            righe_max: 3,
            maiuscole: true,
            posizione_verticale: 0.4,
            allineamento: Allineamento::Sinistra,
            dimensione_font: Some(48.0),
            ..Default::default()
        };
        let stile = Stile {
            evidenziazione: Evidenziazione::Sottolineatura,
            colore_attivo: Colore([1, 2, 3, 255]),
            bordo: 4.0,
            ..Default::default()
        };
        let p = Preset::da("prova", &layout, &stile, &Richiesta::default(), FormatoPreset::Verticale);

        let l = p.layout(None);
        assert_eq!(l.righe_max, 3);
        assert!(l.maiuscole);
        assert!((l.posizione_verticale - 0.4).abs() < 1e-6);
        assert_eq!(l.allineamento, Allineamento::Sinistra);
        assert_eq!(l.dimensione_font, Some(48.0));

        let s = p.stile().unwrap();
        assert_eq!(s.evidenziazione, Evidenziazione::Sottolineatura);
        assert_eq!(s.colore_attivo.0, [1, 2, 3, 255]);
        assert!((s.bordo - 4.0).abs() < 1e-6);
    }

    #[test]
    fn i_tempi_passano_per_i_millisecondi_senza_perdersi() {
        let layout = LayoutConfig {
            attivazione: Attivazione { anticipo: 0.06, pausa_max: 0.60, coda: 0.40 },
            ..Default::default()
        };
        let p = Preset::da(
            "prova",
            &layout,
            &Stile::default(),
            &Richiesta::default(),
            FormatoPreset::Verticale,
        );
        assert_eq!(p.tempi.anticipo_ms, 60);
        assert_eq!(p.tempi.pausa_massima_ms, 600);
        assert_eq!(p.tempi.coda_ms, 400);

        let l = p.layout(None);
        assert!((l.attivazione.anticipo - 0.06).abs() < 1e-9);
        assert!((l.attivazione.pausa_max - 0.60).abs() < 1e-9);
        assert!((l.attivazione.coda - 0.40).abs() < 1e-9);
    }

    #[test]
    fn il_formato_dal_sorgente_usa_le_proporzioni_del_file() {
        let p = sobrio();
        assert_eq!(p.posizione.formato, FormatoPreset::DalSorgente);
        assert_eq!(p.layout(Some((1280, 720))).larghezza, 1280);
        assert_eq!(p.layout(Some((1280, 720))).altezza, 720);
        // Senza sorgente si ricade sul verticale.
        assert_eq!(p.layout(None).larghezza, 1080);
    }

    #[test]
    fn il_formato_esplicito_ignora_il_sorgente() {
        let p = orizzontale();
        let l = p.layout(Some((1080, 1920)));
        assert_eq!((l.larghezza, l.altezza), (1920, 1080));
    }

    #[test]
    fn il_preset_sobrio_non_evidenzia() {
        let p = sobrio();
        assert_eq!(p.evidenziazione.forma, Evidenziazione::Nessuna);
        assert!(p.colori.bordo_px > 0.0, "senza evidenziazione serve almeno il contorno");
    }

    #[test]
    fn i_tre_preset_di_serie_hanno_nomi_distinti() {
        let nomi: Vec<String> = di_serie().into_iter().map(|p| p.nome).collect();
        assert_eq!(nomi, vec!["Verticale", "Orizzontale", "Sobrio"]);
    }

    #[test]
    fn un_preset_non_contiene_riferimenti_a_file() {
        // Un percorso non sopravvive al passaggio a un'altra macchina: il
        // preset deve poter essere condiviso.
        for p in di_serie() {
            let json = p.in_json().unwrap();
            for spia in ["/", "\\\\", ".ttf", ".mp4", ".wav"] {
                assert!(!json.contains(spia), "il preset «{}» contiene «{spia}»", p.nome);
            }
        }
    }

    #[test]
    fn un_preset_non_contiene_impostazioni_del_modello() {
        for p in di_serie() {
            let json = p.in_json().unwrap().to_lowercase();
            for spia in ["whisper", "modello", "lingua", "beam", "gpu", "cuda"] {
                assert!(!json.contains(spia), "il preset «{}» contiene «{spia}»", p.nome);
            }
        }
    }

    #[test]
    fn un_json_malformato_da_un_errore_leggibile() {
        let errore = Preset::da_json("{ questo non e' json").unwrap_err().to_string();
        assert!(errore.contains("preset di Verba valido"), "{errore}");
    }

    #[test]
    fn un_preset_di_una_versione_futura_viene_rifiutato() {
        let mut p = verticale();
        p.versione = VERSIONE + 1;
        let json = serde_json::to_string(&p).unwrap();
        let errore = Preset::da_json(&json).unwrap_err().to_string();
        assert!(errore.contains("versione piu' recente"), "{errore}");
    }

    #[test]
    fn un_colore_scritto_male_viene_segnalato_col_suo_nome() {
        let mut p = verticale();
        p.colori.evidenziazione = "non-un-colore".into();
        let errore = p.stile().unwrap_err().to_string();
        assert!(errore.contains("evidenziazione"), "{errore}");
    }

    #[test]
    fn salvare_e_ricaricare_da_disco() {
        let percorso = std::env::temp_dir().join("verba_test_preset.json");
        let p = orizzontale();
        p.salva(&percorso).unwrap();
        assert_eq!(Preset::carica(&percorso).unwrap(), p);
        let _ = std::fs::remove_file(&percorso);
    }
}
