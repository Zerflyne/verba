//! I file di testo che Verba sa scrivere, e come si sceglie quale.
//!
//! Il formato non si dichiara: lo dice l'estensione del file chiesto. E'
//! l'unica convenzione che non richiede di ricordarsi un'opzione in piu'.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tracing::info;

use verba_core::layout::Blocco;
use verba_core::srt::{self, SrtConfig, SrtMode};
use verba_core::trascrizione::Parola;

use crate::opzioni::SrtStrutturaArg;

/// I quattro formati testuali della spec.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FormatoTesto {
    Srt,
    Vtt,
    Json,
    Txt,
}

impl FormatoTesto {
    pub fn da_estensione(percorso: &Path) -> Option<Self> {
        match percorso.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "srt" => Some(FormatoTesto::Srt),
            "vtt" | "webvtt" => Some(FormatoTesto::Vtt),
            "json" => Some(FormatoTesto::Json),
            "txt" => Some(FormatoTesto::Txt),
            _ => None,
        }
    }

    pub fn etichetta(self) -> &'static str {
        match self {
            FormatoTesto::Srt => "SRT",
            FormatoTesto::Vtt => "WebVTT",
            FormatoTesto::Json => "mappatura parola per parola",
            FormatoTesto::Txt => "solo testo",
        }
    }
}

/// Come costruire le battute di un file di sottotitoli.
pub struct Struttura {
    pub struttura: SrtStrutturaArg,
    pub caratteri_max: usize,
}

impl Struttura {
    fn cue(&self, blocchi: &[Blocco], parole: &[Parola]) -> Vec<srt::Cue> {
        match self.struttura {
            SrtStrutturaArg::Blocchi => srt::cues_da_blocchi(blocchi),
            altro => {
                let cfg = SrtConfig {
                    mode: match altro {
                        SrtStrutturaArg::Parola => SrtMode::Parola,
                        SrtStrutturaArg::Karaoke => SrtMode::Karaoke,
                        _ => SrtMode::Riga,
                    },
                    max_chars: self.caratteri_max,
                    ..Default::default()
                };
                srt::build_cues(parole, &cfg)
            }
        }
    }
}

/// Scrive un file nel formato che la sua estensione dichiara.
pub fn scrivi(
    percorso: &Path,
    formato: FormatoTesto,
    blocchi: &[Blocco],
    parole: &[Parola],
    struttura: &Struttura,
) -> Result<()> {
    let contenuto = match formato {
        FormatoTesto::Json => srt::render_json(parole)?,
        FormatoTesto::Srt => srt::render(&struttura.cue(blocchi, parole)),
        FormatoTesto::Vtt => srt::render_vtt(&struttura.cue(blocchi, parole)),
        // Il testo semplice segue sempre i blocchi: e' la trascrizione come
        // la si leggerebbe, non una struttura da rispettare al millisecondo.
        FormatoTesto::Txt => srt::render_testo(&srt::cues_da_blocchi(blocchi)),
    };
    if let Some(dir) = percorso.parent() {
        if !dir.as_os_str().is_empty() && !dir.exists() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("creazione della cartella {}", dir.display()))?;
        }
    }
    std::fs::write(percorso, contenuto)
        .with_context(|| format!("scrittura di {}", percorso.display()))?;
    let conteggio = match formato {
        FormatoTesto::Json => parole.len(),
        _ => struttura.cue(blocchi, parole).len(),
    };
    info!(
        file = %percorso.display(),
        formato = formato.etichetta(),
        elementi = conteggio,
        "scritto"
    );
    Ok(())
}

/// Riconosce il formato dall'estensione, con un errore che elenca le
/// alternative invece di limitarsi a dire di no.
pub fn formato_richiesto(percorso: &Path) -> Result<FormatoTesto> {
    FormatoTesto::da_estensione(percorso).ok_or_else(|| {
        anyhow::anyhow!(
            "«{}»: estensione non riconosciuta. \
             Il formato dei sottotitoli si sceglie con l'estensione: .srt, .vtt, .json o .txt.",
            percorso.display()
        )
    })
}

/// I file da scrivere per `verba trascrivi`.
///
/// Senza `--out` si scrive un SRT accanto al sorgente: e' cio' che serve nove
/// volte su dieci, e resta esplicito nel log.
pub fn richieste(out: &[PathBuf], sorgente: &str) -> Result<Vec<(PathBuf, FormatoTesto)>> {
    if out.is_empty() {
        let percorso = accanto(sorgente, "", "srt");
        return Ok(vec![(percorso, FormatoTesto::Srt)]);
    }
    let mut fatti: Vec<(PathBuf, FormatoTesto)> = Vec::with_capacity(out.len());
    for percorso in out {
        let formato = formato_richiesto(percorso)?;
        if fatti.iter().any(|(p, _)| p == percorso) {
            bail!("«{}» e' stato chiesto due volte", percorso.display());
        }
        fatti.push((percorso.clone(), formato));
    }
    Ok(fatti)
}

/// Il nome del sorgente con un suffisso e un'altra estensione, nella stessa
/// cartella.
pub fn accanto(sorgente: &str, suffisso: &str, estensione: &str) -> PathBuf {
    let base = if sorgente == "-" { PathBuf::from("uscita") } else { PathBuf::from(sorgente) };
    let radice = base.file_stem().and_then(|s| s.to_str()).unwrap_or("uscita");
    let nome = format!("{radice}{suffisso}.{estensione}");
    match base.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(nome),
        _ => PathBuf::from(nome),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_estensione_sceglie_il_formato() {
        assert_eq!(FormatoTesto::da_estensione(Path::new("a.srt")), Some(FormatoTesto::Srt));
        assert_eq!(FormatoTesto::da_estensione(Path::new("a.VTT")), Some(FormatoTesto::Vtt));
        assert_eq!(FormatoTesto::da_estensione(Path::new("a.json")), Some(FormatoTesto::Json));
        assert_eq!(FormatoTesto::da_estensione(Path::new("a.txt")), Some(FormatoTesto::Txt));
        assert_eq!(FormatoTesto::da_estensione(Path::new("a.mp4")), None);
        assert_eq!(FormatoTesto::da_estensione(Path::new("senza")), None);
    }

    #[test]
    fn senza_out_si_scrive_un_srt_accanto_al_sorgente() {
        let r = richieste(&[], "/tmp/discorso.mp3").unwrap();
        assert_eq!(r, vec![(PathBuf::from("/tmp/discorso.srt"), FormatoTesto::Srt)]);
    }

    #[test]
    fn piu_uscite_insieme_e_ognuna_col_suo_formato() {
        let out = vec![PathBuf::from("a.srt"), PathBuf::from("b.json")];
        let r = richieste(&out, "x.mp3").unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].1, FormatoTesto::Json);
    }

    #[test]
    fn un_estensione_sconosciuta_dice_quali_sono_quelle_buone() {
        let e = richieste(&[PathBuf::from("a.ass")], "x.mp3").unwrap_err().to_string();
        assert!(e.contains(".srt"), "{e}");
        assert!(e.contains(".vtt"), "{e}");
    }

    #[test]
    fn lo_stesso_file_due_volte_e_un_errore() {
        let out = vec![PathBuf::from("a.srt"), PathBuf::from("a.srt")];
        assert!(richieste(&out, "x.mp3").is_err());
    }

    #[test]
    fn il_nome_proposto_sta_accanto_al_sorgente() {
        assert_eq!(accanto("/casa/video.mp4", "_sub", "mp4"), PathBuf::from("/casa/video_sub.mp4"));
        assert_eq!(accanto("video.mkv", "_overlay", "mov"), PathBuf::from("video_overlay.mov"));
        assert_eq!(accanto("-", "", "srt"), PathBuf::from("uscita.srt"));
    }
}
