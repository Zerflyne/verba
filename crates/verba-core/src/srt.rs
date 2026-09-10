//! Generazione dei sottotitoli SRT a partire dalla mappatura parola-per-parola.

use anyhow::Result;
use serde::Serialize;

use crate::trascrizione::Parola;
use crate::layout::Blocco;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtMode {
    /// Una battuta per blocco grafico: il file di testo dice esattamente cosa
    /// compare a schermo e quando. E' la modalita' coerente con il video.
    Blocchi,
    /// Una battuta per parola: e' la mappatura testuale parola-per-parola.
    Parola,
    /// Parole raggruppate in righe leggibili (sottotitolo classico).
    Riga,
    /// Come `Riga`, ma con una battuta per parola in cui la parola corrente e'
    /// evidenziata all'interno della riga (effetto karaoke).
    Karaoke,
}

#[derive(Debug, Clone)]
pub struct SrtConfig {
    pub mode: SrtMode,
    /// Caratteri massimi per battuta (modalita' Line/Karaoke).
    pub max_chars: usize,
    /// Durata massima di una battuta, in secondi.
    pub max_duration: f64,
    /// Una pausa piu' lunga di questo valore chiude la battuta.
    pub max_gap: f64,
    /// Durata minima visibile di una battuta.
    pub min_duration: f64,
}

impl Default for SrtConfig {
    fn default() -> Self {
        Self { mode: SrtMode::Parola, max_chars: 84, max_duration: 6.0, max_gap: 0.6, min_duration: 0.30 }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Cue {
    pub index: usize,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Costruisce le battute secondo la modalita' scelta.
///
/// `words` deve essere gia' passata per `align::ripulisci`: qui si assume una
/// sequenza ordinata, senza buchi e monotona. L'allungamento a `min_duration`
/// puo' pero' far sconfinare una battuta in quella successiva, per cui la
/// monotonia viene ristabilita alla fine da [`evita_sovrapposizioni`].
pub fn build_cues(words: &[Parola], cfg: &SrtConfig) -> Vec<Cue> {
    let mut cues = build_cues_grezze(words, cfg);
    evita_sovrapposizioni(&mut cues);
    cues
}

/// Tronca ogni battuta all'inizio della successiva.
///
/// Un SRT con battute sovrapposte non e' valido e, in modalita' parola per
/// parola, fa comparire due parole insieme sullo schermo. Fra durata minima e
/// assenza di sovrapposizione vince la seconda: una battuta puo' risultare
/// piu' corta del minimo, mai accavallarsi.
fn evita_sovrapposizioni(cues: &mut [Cue]) {
    for i in 0..cues.len().saturating_sub(1) {
        let inizio_successiva = cues[i + 1].start;
        if cues[i].end > inizio_successiva {
            cues[i].end = inizio_successiva.max(cues[i].start);
        }
    }
}

fn build_cues_grezze(words: &[Parola], cfg: &SrtConfig) -> Vec<Cue> {
    match cfg.mode {
        SrtMode::Parola => words
            .iter()
            .enumerate()
            .map(|(i, w)| Cue {
                index: i + 1,
                start: w.inizio,
                end: w.fine.max(w.inizio + cfg.min_duration),
                text: w.testo.clone(),
            })
            .collect(),
        // `Blocchi` passa di norma da `cues_da_blocchi`; se arriva qui si hanno
        // solo le parole, e la resa piu' vicina e' il raggruppamento in righe.
        SrtMode::Riga | SrtMode::Blocchi => group(words, cfg)
            .into_iter()
            .enumerate()
            .map(|(i, g)| Cue {
                index: i + 1,
                start: g[0].inizio,
                end: g.last().unwrap().fine.max(g[0].inizio + cfg.min_duration),
                text: g.iter().map(|w| w.testo.as_str()).collect::<Vec<_>>().join(" "),
            })
            .collect(),
        SrtMode::Karaoke => {
            let mut cues = Vec::new();
            let mut index = 1;
            for g in group(words, cfg) {
                for (i, w) in g.iter().enumerate() {
                    let line = g
                        .iter()
                        .enumerate()
                        .map(|(j, x)| {
                            if j == i {
                                format!("<u>{}</u>", x.testo)
                            } else {
                                x.testo.clone()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    cues.push(Cue {
                        index,
                        start: w.inizio,
                        end: w.fine.max(w.inizio + 0.04),
                        text: line,
                    });
                    index += 1;
                }
            }
            cues
        }
    }
}

/// Raggruppa le parole in righe rispettando lunghezza, durata, pause e
/// punteggiatura di fine frase.
fn group<'a>(words: &'a [Parola], cfg: &SrtConfig) -> Vec<Vec<&'a Parola>> {
    let mut out: Vec<Vec<&Parola>> = Vec::new();
    let mut cur: Vec<&Parola> = Vec::new();
    let mut chars = 0usize;

    for (i, w) in words.iter().enumerate() {
        let gap = if i == 0 { 0.0 } else { w.inizio - words[i - 1].fine };
        let dur = if cur.is_empty() { 0.0 } else { w.fine - cur[0].inizio };
        let would_be = chars + w.testo.chars().count() + usize::from(!cur.is_empty());

        let deve_chiudere = !cur.is_empty()
            && (would_be > cfg.max_chars
                || dur > cfg.max_duration
                || gap > cfg.max_gap
                // cambio di segmento: mai unire due frasi separate da pyannote
                || w.segmento != cur[cur.len() - 1].segmento);

        if deve_chiudere {
            out.push(std::mem::take(&mut cur));
            chars = 0;
        }

        chars += w.testo.chars().count() + usize::from(!cur.is_empty());
        cur.push(w);

        // fine frase: chiudi subito, e' il punto di taglio piu' naturale
        if w.testo.ends_with(['.', '!', '?', '…']) {
            out.push(std::mem::take(&mut cur));
            chars = 0;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Converte i blocchi grafici in battute SRT.
///
/// I tempi sono quelli con cui il blocco compare e sparisce nel video, per cui
/// il file di testo e le immagini restano allineati al millisecondo.
pub fn cues_da_blocchi(blocchi: &[Blocco]) -> Vec<Cue> {
    blocchi
        .iter()
        .enumerate()
        .map(|(i, b)| Cue { index: i + 1, start: b.start, end: b.end, text: b.testo() })
        .collect()
}

/// Serializza le battute nel formato SRT (timestamp `HH:MM:SS,mmm`).
pub fn render(cues: &[Cue]) -> String {
    let mut s = String::with_capacity(cues.len() * 64);
    for (i, cue) in cues.iter().enumerate() {
        s.push_str(&format!("{}\n", i + 1));
        s.push_str(&format!("{} --> {}\n", timestamp(cue.start), timestamp(cue.end)));
        s.push_str(cue.text.trim());
        s.push_str("\n\n");
    }
    s
}

/// `HH:MM:SS,mmm` — la virgola decimale e' obbligatoria nello standard SRT.
pub fn timestamp(secs: f64) -> String {
    let total_ms = (secs.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

/// Esporta la mappatura parola-per-parola in JSON (utile per editor esterni).
pub fn render_json(words: &[Parola]) -> Result<String> {
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "words": words,
        "count": words.len(),
    }))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(testo: &str, inizio: f64, fine: f64, segmento: usize) -> Parola {
        Parola { segmento, ..Parola::nuova(testo, inizio, fine) }
    }

    #[test]
    fn timestamp_formatta_correttamente() {
        assert_eq!(timestamp(0.0), "00:00:00,000");
        assert_eq!(timestamp(3661.5), "01:01:01,500");
        assert_eq!(timestamp(59.9994), "00:00:59,999");
    }

    #[test]
    fn modalita_word_una_battuta_per_parola() {
        let words = vec![w("ciao", 0.0, 0.4, 0), w("mondo", 0.4, 0.9, 0)];
        let cues = build_cues(&words, &SrtConfig::default());
        assert_eq!(cues.len(), 2);
        let out = render(&cues);
        assert!(out.contains("00:00:00,000 --> 00:00:00,400"));
        assert!(out.contains("ciao"));
    }

    #[test]
    fn la_pausa_lunga_spezza_la_riga() {
        let words = vec![w("uno", 0.0, 0.3, 0), w("due", 2.0, 2.3, 0)];
        let cfg = SrtConfig { mode: SrtMode::Riga, ..Default::default() };
        let cues = build_cues(&words, &cfg);
        assert_eq!(cues.len(), 2);
    }

    #[test]
    fn il_punto_fermo_chiude_la_battuta() {
        let words = vec![w("Ciao.", 0.0, 0.3, 0), w("Come", 0.35, 0.6, 0)];
        let cfg = SrtConfig { mode: SrtMode::Riga, ..Default::default() };
        let cues = build_cues(&words, &cfg);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "Ciao.");
    }

    #[test]
    fn nessuna_battuta_si_sovrappone_alla_successiva() {
        // parole fitte: la durata minima di 0,30 s le farebbe accavallare
        let words: Vec<Parola> = (0..10)
            .map(|i| w("parola", i as f64 * 0.12, i as f64 * 0.12 + 0.08, 0))
            .collect();
        for mode in [SrtMode::Parola, SrtMode::Riga, SrtMode::Karaoke] {
            let cfg = SrtConfig { mode, ..Default::default() };
            let cues = build_cues(&words, &cfg);
            for pair in cues.windows(2) {
                assert!(
                    pair[1].start >= pair[0].end - 1e-9,
                    "{mode:?}: {:?} si sovrappone a {:?}",
                    pair[0],
                    pair[1]
                );
            }
            assert!(cues.iter().all(|c| c.end >= c.start), "{mode:?}: battuta invertita");
        }
    }

    #[test]
    fn la_durata_minima_si_applica_quando_ce_spazio() {
        // parole distanziate: nessun conflitto, il minimo viene rispettato
        let words = vec![w("uno", 0.0, 0.05, 0), w("due", 2.0, 2.05, 0)];
        let cues = build_cues(&words, &SrtConfig::default());
        assert!((cues[0].end - 0.30).abs() < 1e-9, "{:?}", cues[0]);
    }

    #[test]
    fn le_battute_dai_blocchi_ricalcano_il_video() {
        use crate::layout::{impagina, LayoutConfig, Tipografo};
        let cfg = LayoutConfig { dimensione_font: Some(64.0), ..Default::default() };
        let mut tipo =
            Tipografo::nuovo(crate::FONT_INTER_BOLD, cfg.corpo(), cfg.interlinea).unwrap();
        let words = vec![w("Ciao.", 0.0, 0.3, 0), w("Come", 0.4, 0.7, 0), w("stai?", 0.7, 1.0, 0)];
        let blocchi = impagina(&words, &mut tipo, &cfg).unwrap();
        let cues = cues_da_blocchi(&blocchi);
        assert_eq!(cues.len(), blocchi.len());
        for (c, b) in cues.iter().zip(&blocchi) {
            assert_eq!(c.text, b.testo());
            assert_eq!(c.start, b.start);
            assert_eq!(c.end, b.end);
        }
        for coppia in cues.windows(2) {
            assert!(coppia[0].end <= coppia[1].start + 1e-9);
        }
    }

    #[test]
    fn karaoke_evidenzia_una_parola_per_volta() {
        let words = vec![w("uno", 0.0, 0.3, 0), w("due", 0.3, 0.6, 0)];
        let cfg = SrtConfig { mode: SrtMode::Karaoke, ..Default::default() };
        let cues = build_cues(&words, &cfg);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "<u>uno</u> due");
        assert_eq!(cues[1].text, "uno <u>due</u>");
    }
}
