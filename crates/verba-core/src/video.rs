//! Esportazione dei sottotitoli in un file video con canale alfa.
//!
//! Mette insieme i tre pezzi: i blocchi impaginati da [`crate::layout`], il
//! disegno di [`crate::render`] e l'encoder ProRes di [`crate::encoder`].
//!
//! Il fotogramma viene ridisegnato **solo quando cambia qualcosa** — una nuova
//! riga, oppure il rettangolo che salta a un'altra parola o si spegne. Fra un
//! cambio e l'altro lo stesso fotogramma viene ricodificato tale e quale: a
//! 30 fps una parola dura in media dodici fotogrammi, e ridisegnarli tutti
//! sarebbe lavoro inutile.

use std::path::Path;
use std::time::Instant;

use anyhow::{bail, Result};
use tracing::debug;

use crate::encoder::{Encoder, EncoderConfig};
use crate::eventi::{Annullato, Evento, Fase, Progresso};
use crate::layout::{Blocco, LayoutConfig};
use crate::render::{Rasterizzatore, Tela};

#[derive(Debug, Clone)]
pub struct VideoConfig {
    pub fps_num: u32,
    pub fps_den: u32,
    /// Quantizzatore ProRes: piu' basso = piu' qualita' e file piu' grande.
    pub qualita: u32,
    pub thread: usize,
    /// Durata del video in secondi (di norma quella dell'audio).
    pub durata: f64,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self { fps_num: 30, fps_den: 1, qualita: 4, thread: 0, durata: 0.0 }
    }
}

impl VideoConfig {
    pub fn fps(&self) -> f64 {
        self.fps_num as f64 / self.fps_den.max(1) as f64
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Statistiche {
    pub fotogrammi: u64,
    pub fotogrammi_disegnati: u64,
    pub blocchi: usize,
    pub secondi: f64,
}

/// Stato visibile in un dato istante: quale riga, e quale parola vi e' indicata
/// dal rettangolo (`None` quando non ce n'e' nessuna: la riga resta, il
/// rettangolo no).
type Stato = Option<(usize, Option<usize>)>;

/// Rende i sottotitoli e scrive il file video.
pub fn esporta(
    blocchi: &[Blocco],
    rasterizzatore: &mut Rasterizzatore,
    cfg: &LayoutConfig,
    vcfg: &VideoConfig,
    percorso: &Path,
    progresso: &Progresso,
) -> Result<Statistiche> {
    if vcfg.fps_num == 0 || vcfg.fps_den == 0 {
        bail!("frame rate non valido: {}/{}", vcfg.fps_num, vcfg.fps_den);
    }
    if vcfg.durata <= 0.0 {
        bail!("durata del video non valida: {} s", vcfg.durata);
    }

    let fps = vcfg.fps();
    let totale = (vcfg.durata * fps).ceil().max(1.0) as u64;
    let stati = calcola_stati(blocchi, totale, vcfg);

    let mut encoder = Encoder::apri(
        percorso,
        &EncoderConfig {
            larghezza: cfg.larghezza,
            altezza: cfg.altezza,
            fps_num: vcfg.fps_num,
            fps_den: vcfg.fps_den,
            qualita: vcfg.qualita,
            thread: vcfg.thread,
        },
    )?;

    let mut tela = Tela::nuova(cfg.larghezza, cfg.altezza);
    let mut blocco_preparato: Option<usize> = None;
    let mut disegnati = 0u64;
    let inizio = Instant::now();

    let mut fermato = false;
    let mut i = 0usize;
    while i < stati.len() {
        // Quanti fotogrammi consecutivi condividono lo stesso stato.
        let stato = stati[i];
        let mut j = i + 1;
        while j < stati.len() && stati[j] == stato {
            j += 1;
        }
        let ripetizioni = (j - i) as u32;

        match stato {
            None => tela.pulisci(),
            Some((b, parola)) => {
                if blocco_preparato != Some(b) {
                    rasterizzatore.prepara(&blocchi[b]);
                    blocco_preparato = Some(b);
                }
                rasterizzatore.componi(parola, &mut tela);
            }
        }
        disegnati += 1;
        encoder.scrivi(tela.pixel(), ripetizioni)?;

        progresso.passo(Fase::Codifica, j as f32 / stati.len() as f32);

        if progresso.annullato() {
            fermato = true;
            break;
        }
        i = j;
    }

    // Annullare durante la codifica deve fermarla davvero e non lasciare in
    // giro un file mezzo scritto: l'encoder viene abbandonato senza chiudere
    // il contenitore, e il file parziale cancellato.
    if fermato {
        drop(encoder);
        let _ = std::fs::remove_file(percorso);
        progresso.emetti(Evento::Annullata);
        return Err(Annullato.into());
    }

    let fotogrammi = encoder.frame_scritti() as u64;
    encoder.chiudi()?;
    let secondi = inizio.elapsed().as_secs_f64();
    debug!(fotogrammi, disegnati, "codifica conclusa");

    Ok(Statistiche { fotogrammi, fotogrammi_disegnati: disegnati, blocchi: blocchi.len(), secondi })
}

/// Per ogni fotogramma, quale riga e' visibile e quale parola e' indicata.
///
/// Il tempo campionato e' il **centro** del fotogramma: un sottotitolo che
/// compare a meta' fotogramma viene mostrato dal fotogramma che lo contiene
/// per piu' della meta' della sua durata, che e' il comportamento atteso.
fn calcola_stati(blocchi: &[Blocco], totale: u64, vcfg: &VideoConfig) -> Vec<Stato> {
    let passo = vcfg.fps_den.max(1) as f64 / vcfg.fps_num as f64;
    let mut stati = Vec::with_capacity(totale as usize);
    let mut cursore = 0usize;

    for f in 0..totale {
        let t = (f as f64 + 0.5) * passo;
        while cursore < blocchi.len() && blocchi[cursore].end <= t {
            cursore += 1;
        }
        let stato = match blocchi.get(cursore) {
            Some(b) if t >= b.start => Some((cursore, b.parola_attiva(t))),
            _ => None,
        };
        stati.push(stato);
    }
    stati
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trascrizione::Parola;
    use crate::layout::{impagina, Attivazione, Formato, Tipografo};

    const FONT: &[u8] = include_bytes!("../assets/Inter-Bold.ttf");

    fn blocchi_di_prova(cfg: &LayoutConfig) -> Vec<Blocco> {
        let mut t = Tipografo::nuovo(FONT, cfg.corpo(), cfg.interlinea).unwrap();
        let parole: Vec<Parola> = "una prova di sottotitoli"
            .split(' ')
            .enumerate()
            .map(|(i, p)| Parola::nuova(p, 1.0 + i as f64 * 0.5, 1.0 + i as f64 * 0.5 + 0.45))
            .collect();
        impagina(&parole, &mut t, cfg).unwrap()
    }

    #[test]
    fn prima_del_primo_blocco_lo_schermo_e_vuoto() {
        let (larghezza, altezza) = Formato::Verticale.risoluzione();
        let cfg = LayoutConfig { larghezza, altezza, dimensione_font: Some(72.0), ..Default::default() };
        let blocchi = blocchi_di_prova(&cfg);
        let vcfg = VideoConfig { durata: 5.0, ..Default::default() };
        let stati = calcola_stati(&blocchi, 150, &vcfg);
        assert_eq!(stati[0], None, "a 0,0167 s non c'e' ancora parlato");
        assert!(stati.iter().any(|s| s.is_some()), "nessun fotogramma con sottotitolo");
    }

    #[test]
    fn la_parola_indicata_avanza_nel_tempo() {
        let (larghezza, altezza) = Formato::Verticale.risoluzione();
        let cfg = LayoutConfig { larghezza, altezza, dimensione_font: Some(72.0), ..Default::default() };
        let blocchi = blocchi_di_prova(&cfg);
        let vcfg = VideoConfig { durata: 5.0, ..Default::default() };
        let stati = calcola_stati(&blocchi, 150, &vcfg);
        let sequenza: Vec<usize> = stati.iter().flatten().filter_map(|(_, p)| *p).collect();
        assert!(sequenza.windows(2).all(|c| c[1] >= c[0]), "la parola indicata torna indietro");
        assert!(sequenza.contains(&0) && sequenza.iter().any(|&p| p > 0));
    }

    #[test]
    fn nel_silenzio_la_riga_resta_e_il_rettangolo_no() {
        let (larghezza, altezza) = Formato::Verticale.risoluzione();
        let cfg = LayoutConfig {
            larghezza,
            altezza,
            dimensione_font: Some(72.0),
            pausa_max: 10.0,
            attivazione: Attivazione { anticipo: 0.0, pausa_max: 0.20, coda: 0.0 },
            ..Default::default()
        };
        let mut t = Tipografo::nuovo(FONT, cfg.corpo(), cfg.interlinea).unwrap();
        let parole = vec![
            Parola::nuova("alfa", 0.0, 0.4),
            Parola::nuova("beta", 2.0, 2.4),
        ];
        let blocchi = impagina(&parole, &mut t, &cfg).unwrap();
        let vcfg = VideoConfig { durata: 3.0, ..Default::default() };
        let stati = calcola_stati(&blocchi, 90, &vcfg);
        // a 1,0 s (fotogramma 30) la riga c'e', ma nessuna parola e' indicata
        assert_eq!(stati[30], Some((0, None)), "{:?}", stati[30]);
        assert_eq!(stati[0].map(|(_, p)| p), Some(Some(0)));
    }

    /// Un rasterizzatore pronto sui blocchi dati, per le prove di scrittura.
    fn rasterizzatore(cfg: &LayoutConfig) -> Rasterizzatore {
        let t = Tipografo::nuovo(FONT, cfg.corpo(), cfg.interlinea).unwrap();
        Rasterizzatore::nuovo(t, cfg.clone(), crate::render::Stile::default())
    }

    fn percorso_di_prova(nome: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("verba_test_{nome}.mov"))
    }

    #[test]
    fn esporta_scrive_il_file_e_conta_i_fotogrammi() {
        // Piccolo apposta: qui si verifica il percorso completo fino a
        // libavcodec, non la qualita' del disegno.
        let cfg = LayoutConfig {
            larghezza: 320,
            altezza: 240,
            dimensione_font: Some(24.0),
            ..Default::default()
        };
        let blocchi = blocchi_di_prova(&cfg);
        let vcfg = VideoConfig { fps_num: 25, fps_den: 1, durata: 2.0, ..Default::default() };
        let percorso = percorso_di_prova("esporta");
        let _ = std::fs::remove_file(&percorso);

        let stat = esporta(
            &blocchi,
            &mut rasterizzatore(&cfg),
            &cfg,
            &vcfg,
            &percorso,
            &Progresso::silenzioso(),
        )
        .unwrap();

        assert_eq!(stat.fotogrammi, 50, "2 s a 25 fps");
        assert!(stat.fotogrammi_disegnati < stat.fotogrammi, "ridisegnati tutti i fotogrammi");
        let scritto = std::fs::metadata(&percorso).unwrap().len();
        assert!(scritto > 0, "file vuoto");
        let _ = std::fs::remove_file(&percorso);
    }

    #[test]
    fn annullare_ferma_la_codifica_e_cancella_il_file_parziale() {
        let cfg = LayoutConfig {
            larghezza: 320,
            altezza: 240,
            dimensione_font: Some(24.0),
            ..Default::default()
        };
        let blocchi = blocchi_di_prova(&cfg);
        let vcfg = VideoConfig { fps_num: 25, fps_den: 1, durata: 60.0, ..Default::default() };
        let percorso = percorso_di_prova("annullata");
        let _ = std::fs::remove_file(&percorso);

        let progresso = Progresso::silenzioso();
        progresso.interruttore().annulla();

        let esito = esporta(&blocchi, &mut rasterizzatore(&cfg), &cfg, &vcfg, &percorso, &progresso);

        let errore = esito.expect_err("la codifica doveva fermarsi");
        assert!(
            errore.downcast_ref::<Annullato>().is_some(),
            "annullare non e' un errore qualsiasi: {errore}"
        );
        assert!(!percorso.exists(), "il file parziale e' rimasto: {}", percorso.display());
    }

    #[test]
    fn il_numero_di_fotogrammi_segue_durata_e_frame_rate() {
        let vcfg = VideoConfig { fps_num: 25, fps_den: 1, durata: 4.0, ..Default::default() };
        assert_eq!((vcfg.durata * vcfg.fps()).ceil() as u64, 100);
        let vcfg = VideoConfig { fps_num: 30000, fps_den: 1001, durata: 1.0, ..Default::default() };
        assert_eq!((vcfg.durata * vcfg.fps()).ceil() as u64, 30);
    }
}
