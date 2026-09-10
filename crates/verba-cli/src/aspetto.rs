//! Dalle opzioni all'impaginazione e allo stile.
//!
//! Il punto delicato e' la stratificazione: un preset fa da base e cio' che
//! e' stato scritto a mano lo scavalca. Perche' funzioni bisogna distinguere
//! un'opzione data davvero da una lasciata al valore predefinito di clap, e
//! quella distinzione la conosce solo [`clap::ArgMatches`]: da qui
//! [`DateAMano`].

use anyhow::{bail, Context, Result};

use verba_core::caratteri::{self, Catalogo, Esito, Richiesta};
use verba_core::layout::{self, Allineamento, Attivazione, LayoutConfig, Tipografo};
use verba_core::progetto::{FormatoPreset, Preset};
use verba_core::render::{Colore, Evidenziazione, Stile};

use crate::opzioni::{
    AllineamentoArg, Aspetto, EvidenziazioneArg, FormatoArg, PresetArg,
};

/// Le opzioni date davvero sulla riga di comando.
///
/// Senza questa distinzione un preset verrebbe sempre sovrascritto dai valori
/// predefiniti di clap, che nella struttura sono indistinguibili da una
/// scelta esplicita.
pub struct DateAMano(pub clap::ArgMatches);

impl DateAMano {
    pub fn ha(&self, nome: &str) -> bool {
        matches!(
            self.0.value_source(nome),
            Some(clap::parser::ValueSource::CommandLine)
        )
    }

    /// Il valore dell'opzione se e' stata data a mano, altrimenti il ripiego.
    pub fn oppure<T: Clone>(&self, nome: &str, dato: &T, ripiego: T) -> T {
        if self.ha(nome) {
            dato.clone()
        } else {
            ripiego
        }
    }
}

/// Il catalogo dei caratteri secondo le opzioni date.
pub fn catalogo(a: &Aspetto) -> Catalogo {
    let mut cartelle = caratteri::cartelle_predefinite();
    cartelle.extend(a.cartella_caratteri.iter().cloned());
    Catalogo::nuovo(&cartelle, a.caratteri_di_sistema)
}

/// Il preset di partenza, se ne e' stato chiesto uno.
pub fn preset_di_partenza(a: &Aspetto) -> Result<Option<Preset>> {
    if let Some(percorso) = &a.preset {
        return Ok(Some(Preset::carica(percorso)?));
    }
    Ok(a.preset_di_serie.map(PresetArg::preset))
}

/// Sceglie il carattere e prepara il motore di composizione.
pub fn costruisci_tipografo(
    a: &Aspetto,
    date: &DateAMano,
    base: Option<&Preset>,
    layout_cfg: &LayoutConfig,
) -> Result<(Tipografo, Esito)> {
    let mut cat = catalogo(a);
    let d = base.map(|b| b.carattere()).unwrap_or_default();
    let richiesta = Richiesta {
        famiglia: date.oppure("carattere", &a.carattere, d.famiglia),
        peso: date.oppure("peso", &a.peso, d.peso),
        file: a.font.clone(),
    };
    let esito = cat.risolvi(&richiesta)?;
    let tipografo = Tipografo::dal_catalogo(cat, &esito, layout_cfg.corpo(), layout_cfg.interlinea)
        .context("preparazione del carattere")?;
    Ok((tipografo, esito))
}

/// La durata minima per parola, stratificata come tutto il resto.
pub fn durata_minima_parola(a: &Aspetto, date: &DateAMano, base: Option<&Preset>) -> f64 {
    let d = base.map(|b| b.durata_minima_parola()).unwrap_or(verba_core::pulizia::DURATA_MINIMA_PAROLA);
    date.oppure("durata_minima_parola", &a.durata_minima_parola, d).max(0.0)
}

/// Cosa fare quando la risoluzione non e' libera.
///
/// Imprimere i sottotitoli su un filmato vuole il fotogramma del filmato:
/// non ci sono proporzioni da scegliere, ci sono proporzioni da rispettare.
pub enum Vincolo {
    /// Nessun vincolo: decidono le opzioni e il preset.
    Libero,
    /// I sottotitoli finiscono *dentro* quel fotogramma: la risoluzione non
    /// e' negoziabile.
    Impresso(u32, u32),
    /// I sottotitoli finiscono *sopra* quel fotogramma, in montaggio. Una
    /// risoluzione diversa e' legittima — si scala — ma quasi sempre non e'
    /// cio' che si voleva, e va detto.
    Sovrapposto(u32, u32),
}

pub fn configura_layout(
    a: &Aspetto,
    date: &DateAMano,
    base: Option<&Preset>,
    sorgente: Option<(u32, u32)>,
    vincolo: &Vincolo,
) -> Result<LayoutConfig> {
    // Il formato: la risoluzione esplicita vince su tutto, poi il formato
    // scritto a mano, poi quello del preset, poi il predefinito.
    let (mut larghezza, mut altezza) = match (&a.risoluzione, base) {
        (Some(s), _) => analizza_risoluzione(s)?,
        (None, Some(b)) if !date.ha("formato") => b.posizione.formato.risoluzione(sorgente),
        _ => match a.formato {
            FormatoArg::Verticale => FormatoPreset::Verticale,
            FormatoArg::Orizzontale => FormatoPreset::Orizzontale,
            FormatoArg::DalSorgente => FormatoPreset::DalSorgente,
        }
        .risoluzione(sorgente),
    };

    // Un preset «16:9» applicato a un filmato 1280x720 chiederebbe 1920x1080,
    // che sul filmato non ci sta. Il filmato vince, e lo si dice: e' cio' che
    // una persona si aspetta chiedendo di imprimere i sottotitoli su *quel*
    // video. Una --risoluzione scritta a mano invece non si tocca: se e'
    // sbagliata deve emergere come errore, non come correzione silenziosa.
    match *vincolo {
        Vincolo::Impresso(l, h) if a.risoluzione.is_none() && (larghezza, altezza) != (l, h) => {
            tracing::warn!(
                chiesta = format!("{larghezza}x{altezza}"),
                filmato = format!("{l}x{h}"),
                "i sottotitoli vengono impaginati sul fotogramma del filmato"
            );
            larghezza = l;
            altezza = h;
        }
        Vincolo::Sovrapposto(l, h) if (larghezza, altezza) != (l, h) => {
            tracing::warn!(
                overlay = format!("{larghezza}x{altezza}"),
                filmato = format!("{l}x{h}"),
                "l'overlay non ha la risoluzione del filmato: in montaggio andra' scalato"
            );
        }
        _ => {}
    }
    if larghezza % 2 != 0 || altezza % 2 != 0 {
        bail!(
            "risoluzione {larghezza}x{altezza}: gli encoder vogliono dimensioni pari. \
             Indicane una con --risoluzione."
        );
    }

    let d = base.map(|b| b.layout(Some((larghezza, altezza)))).unwrap_or_default();

    let cfg = LayoutConfig {
        larghezza,
        altezza,
        margine: date.oppure("margine", &a.margine, d.margine),
        larghezza_max: date.oppure("larghezza_massima", &a.larghezza_massima, d.larghezza_max),
        // --posizione, se c'e', ha la precedenza: e' la forma per nome della
        // stessa grandezza.
        posizione_verticale: match a.posizione {
            Some(p) => p.frazione(),
            None => date.oppure("posizione_verticale", &a.posizione_verticale, d.posizione_verticale),
        },
        posizione_orizzontale: date.oppure(
            "posizione_orizzontale",
            &a.posizione_orizzontale,
            d.posizione_orizzontale,
        ),
        righe_max: date.oppure("righe_massime", &a.righe_massime, d.righe_max),
        allineamento: if date.ha("allineamento") {
            match a.allineamento {
                AllineamentoArg::Sinistra => Allineamento::Sinistra,
                AllineamentoArg::Centro => Allineamento::Centro,
                AllineamentoArg::Destra => Allineamento::Destra,
            }
        } else {
            d.allineamento
        },
        maiuscole: date.oppure("maiuscole", &a.maiuscole, d.maiuscole),
        dimensione_font: if date.ha("dimensione_font") { a.dimensione_font } else { d.dimensione_font },
        interlinea: date.oppure("interlinea", &a.interlinea, d.interlinea),
        durata_max: date.oppure("durata_blocco", &a.durata_blocco, d.durata_max),
        pausa_max: date.oppure("pausa_blocco", &a.pausa_blocco, d.pausa_max),
        tenuta: date.oppure("tenuta", &a.tenuta, d.tenuta),
        attivazione: Attivazione {
            anticipo: date.oppure("anticipo", &a.anticipo, d.attivazione.anticipo).max(0.0),
            pausa_max: date
                .oppure("pausa_massima", &a.pausa_massima, d.attivazione.pausa_max)
                .max(0.0),
            coda: date.oppure("coda", &a.coda, d.attivazione.coda).max(0.0),
        },
    };

    if !(0.0..0.45).contains(&cfg.margine) {
        bail!("--margine deve stare fra 0 e 0,45 (ricevuto {})", cfg.margine);
    }
    if !(0.05..=1.0).contains(&cfg.larghezza_max) {
        bail!("--larghezza-massima deve stare fra 0,05 e 1 (ricevuto {})", cfg.larghezza_max);
    }
    for (nome, valore) in [
        ("--posizione-verticale", cfg.posizione_verticale),
        ("--posizione-orizzontale", cfg.posizione_orizzontale),
    ] {
        if !(0.0..=1.0).contains(&valore) {
            bail!("{nome} deve stare fra 0 e 1 (ricevuto {valore})");
        }
    }
    if !(1..=layout::RIGHE_MAX_CONSENTITE).contains(&cfg.righe_max) {
        bail!(
            "--righe-massime deve stare fra 1 e {} (ricevuto {})",
            layout::RIGHE_MAX_CONSENTITE,
            cfg.righe_max
        );
    }
    Ok(cfg)
}

pub fn configura_stile(a: &Aspetto, date: &DateAMano, base: Option<&Preset>) -> Result<Stile> {
    let d = match base {
        Some(b) => b.stile()?,
        None => Stile::default(),
    };
    let leggi = |nome: &str, valore: &str| -> Result<Colore> {
        Colore::da_esadecimale(valore).map_err(|e| anyhow::anyhow!("{nome}: {e}"))
    };
    let colore = |opzione: &str, valore: &str, ripiego: Colore| -> Result<Colore> {
        if date.ha(opzione) {
            leggi(&format!("--{}", opzione.replace('_', "-")), valore)
        } else {
            Ok(ripiego)
        }
    };

    Ok(Stile {
        colore: colore("colore", &a.colore, d.colore)?,
        colore_attivo: colore("colore_attivo", &a.colore_attivo, d.colore_attivo)?,
        colore_evidenziazione: colore(
            "colore_evidenziazione",
            &a.colore_evidenziazione,
            d.colore_evidenziazione,
        )?,
        colore_bordo: colore("colore_bordo", &a.colore_bordo, d.colore_bordo)?,
        bordo: date.oppure("bordo", &a.bordo, d.bordo).max(0.0),
        // --senza-evidenziazione e' la forma breve di --evidenziazione nessuna
        // e ha la precedenza.
        evidenziazione: if a.senza_evidenziazione {
            Evidenziazione::Nessuna
        } else if date.ha("evidenziazione") {
            match a.evidenziazione {
                EvidenziazioneArg::Rettangolo => Evidenziazione::Rettangolo,
                EvidenziazioneArg::Sottolineatura => Evidenziazione::Sottolineatura,
                EvidenziazioneArg::SoloColore => Evidenziazione::SoloColore,
                EvidenziazioneArg::Nessuna => Evidenziazione::Nessuna,
            }
        } else {
            d.evidenziazione
        },
        padding: date.oppure("padding_evidenziazione", &a.padding_evidenziazione, d.padding).max(0.0),
        altezza: date.oppure("altezza_evidenziazione", &a.altezza_evidenziazione, d.altezza).max(0.0),
        raggio: date.oppure("raggio_evidenziazione", &a.raggio_evidenziazione, d.raggio).max(0.0),
        spessore_sottolineatura: date
            .oppure(
                "spessore_sottolineatura",
                &a.spessore_sottolineatura,
                d.spessore_sottolineatura,
            )
            .max(0.0),
        ombra: if a.senza_ombra { false } else { d.ombra },
        colore_ombra: colore("colore_ombra", &a.colore_ombra, d.colore_ombra)?,
        ombra_spostamento: date
            .oppure("ombra_spostamento", &a.ombra_spostamento, d.ombra_spostamento)
            .max(0.0),
        ombra_sfocatura: date
            .oppure("ombra_sfocatura", &a.ombra_sfocatura, d.ombra_sfocatura)
            .max(0.0),
    })
}

/// Salva l'aspetto risultante, se e' stato chiesto.
pub fn salva_preset(
    a: &Aspetto,
    layout_cfg: &LayoutConfig,
    stile: &Stile,
    esito: &Esito,
) -> Result<()> {
    let Some(percorso) = &a.salva_preset else { return Ok(()) };
    let formato = if a.risoluzione.is_some() {
        FormatoPreset::DalSorgente
    } else if layout_cfg.larghezza >= layout_cfg.altezza {
        FormatoPreset::Orizzontale
    } else {
        FormatoPreset::Verticale
    };
    let nome = percorso
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Personalizzato")
        .to_string();
    let preset = Preset::da(
        &nome,
        layout_cfg,
        stile,
        &Richiesta { famiglia: esito.famiglia().to_string(), peso: esito.peso(), file: None },
        formato,
    );
    preset.salva(percorso)?;
    tracing::info!(file = %percorso.display(), nome = %preset.nome, "preset salvato");
    Ok(())
}

pub fn analizza_risoluzione(s: &str) -> Result<(u32, u32)> {
    let (l, a) = s
        .split_once(['x', 'X', '*'])
        .with_context(|| format!("risoluzione «{s}»: formato atteso LARGHEZZAxALTEZZA"))?;
    let larghezza: u32 = l.trim().parse().with_context(|| format!("larghezza «{l}»"))?;
    let altezza: u32 = a.trim().parse().with_context(|| format!("altezza «{a}»"))?;
    if larghezza == 0 || altezza == 0 {
        bail!("risoluzione «{s}»: le dimensioni devono essere positive");
    }
    if larghezza % 2 != 0 || altezza % 2 != 0 {
        bail!("risoluzione «{s}»: larghezza e altezza devono essere pari");
    }
    Ok((larghezza, altezza))
}

/// Frame rate come frazione esatta.
///
/// I valori NTSC (23,976 / 29,97 / 59,94 …) sono scritture arrotondate di
/// frazioni con denominatore 1001: passarli come decimali produrrebbe una
/// deriva di alcuni fotogrammi all'ora, per cui vengono riconosciuti a parte.
pub fn analizza_fps(s: &str) -> Result<(u32, u32)> {
    let s = s.trim();
    if let Some((n, d)) = s.split_once('/') {
        let num: u32 = n.trim().parse().with_context(|| format!("numeratore «{n}»"))?;
        let den: u32 = d.trim().parse().with_context(|| format!("denominatore «{d}»"))?;
        if num == 0 || den == 0 {
            bail!("frame rate «{s}»: numeratore e denominatore devono essere positivi");
        }
        return Ok((num, den));
    }
    let valore: f64 = s.parse().with_context(|| format!("frame rate «{s}»"))?;
    if valore <= 0.0 {
        bail!("frame rate «{s}»: deve essere positivo");
    }
    for (decimale, num) in
        [(23.976, 24000), (29.97, 30000), (47.952, 48000), (59.94, 60000), (119.88, 120000)]
    {
        if (valore - decimale).abs() < 0.005 {
            return Ok((num, 1001));
        }
    }
    if (valore - valore.round()).abs() < 1e-9 {
        return Ok((valore.round() as u32, 1));
    }
    Ok(((valore * 1000.0).round() as u32, 1000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn il_frame_rate_ntsc_resta_una_frazione_esatta() {
        assert_eq!(analizza_fps("29.97").unwrap(), (30000, 1001));
        assert_eq!(analizza_fps("23.976").unwrap(), (24000, 1001));
        assert_eq!(analizza_fps("30").unwrap(), (30, 1));
        assert_eq!(analizza_fps("30000/1001").unwrap(), (30000, 1001));
        assert_eq!(analizza_fps("12.5").unwrap(), (12500, 1000));
        assert!(analizza_fps("0").is_err());
        assert!(analizza_fps("boh").is_err());
    }

    #[test]
    fn la_risoluzione_richiede_dimensioni_pari() {
        assert_eq!(analizza_risoluzione("1080x1920").unwrap(), (1080, 1920));
        assert_eq!(analizza_risoluzione(" 1920 X 1080 ").unwrap(), (1920, 1080));
        assert!(analizza_risoluzione("1081x1920").is_err());
        assert!(analizza_risoluzione("1080").is_err());
        assert!(analizza_risoluzione("0x0").is_err());
    }
}
