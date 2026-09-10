//! Costruzione dell'*initial prompt* di Whisper a partire da un file CSV.
//!
//! Whisper accetta un testo di contesto che precede idealmente l'audio: e' la
//! via ufficiale per orientare il modello su nomi propri, sigle, termini
//! tecnici e stile di punteggiatura. Elencare li' i termini attesi riduce in
//! modo netto gli errori sui nomi ("Zerflyne" invece di "zer fline").
//!
//! Il CSV e' volutamente tollerante:
//!
//! * delimitatore riconosciuto da solo fra `,` `;` `\t` `|`;
//! * righe vuote e commenti `#` ignorati;
//! * riga di intestazione riconosciuta e scartata da sola;
//! * colonna selezionabile per nome o per indice (default: la prima);
//! * virgolette RFC 4180, quindi un termine puo' contenere virgole.
//!
//! Il prompt e' limitato in lunghezza: whisper.cpp accetta al massimo
//! `n_text_ctx / 2` token (224 per large-v3) e un prompt piu' lungo verrebbe
//! troncato in modo cieco, magari a meta' di un nome.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tracing::{debug, info, warn};

/// Intestazioni riconosciute: se la prima riga contiene solo questi valori,
/// viene trattata come header anche senza che l'utente lo dichiari.
const HEADER_HINTS: &[&str] = &[
    "termine", "termini", "parola", "parole", "vocabolario", "glossario", "nome", "nomi",
    "voce", "lemma", "testo", "term", "terms", "word", "words", "vocabulary", "glossary",
    "name", "names", "text", "phrase", "categoria", "category", "tipo", "type", "note",
    "peso", "weight", "descrizione", "description",
];

/// Limite prudenziale in caratteri. Il tokenizer di Whisper produce, per
/// l'italiano, circa un token ogni 3 caratteri: 700 caratteri stanno sotto i
/// 224 token ammessi anche nel caso peggiore.
pub const DEFAULT_MAX_CHARS: usize = 700;

#[derive(Debug, Clone, Default)]
pub struct PromptConfig {
    /// File CSV con i termini da suggerire al modello.
    pub csv: Option<PathBuf>,
    /// Colonna da leggere: nome dell'intestazione oppure indice base 0.
    pub column: Option<String>,
    /// Delimitatore forzato; se assente viene dedotto dal file.
    pub delimiter: Option<char>,
    /// Testo introduttivo posto prima dell'elenco (es. "Glossario:").
    pub preamble: Option<String>,
    /// Prompt libero, anteposto ai termini del CSV.
    pub free_text: Option<String>,
    /// Lunghezza massima del prompt finale, in caratteri.
    pub max_chars: usize,
}

impl PromptConfig {
    pub fn is_empty(&self) -> bool {
        self.csv.is_none() && self.free_text.is_none()
    }
}

/// Compone l'initial prompt. Ritorna `None` se non c'e' nulla da suggerire.
pub fn build(cfg: &PromptConfig) -> Result<Option<String>> {
    if cfg.is_empty() {
        return Ok(None);
    }
    let max_chars = if cfg.max_chars == 0 { DEFAULT_MAX_CHARS } else { cfg.max_chars };

    let terms = match cfg.csv.as_deref() {
        Some(path) => load_terms(path, cfg.column.as_deref(), cfg.delimiter)?,
        None => Vec::new(),
    };

    let mut prompt = String::new();
    if let Some(free) = cfg.free_text.as_deref() {
        let free = free.trim();
        if !free.is_empty() {
            prompt.push_str(free);
        }
    }

    if !terms.is_empty() {
        if !prompt.is_empty() {
            prompt.push(' ');
        }
        if let Some(pre) = cfg.preamble.as_deref() {
            let pre = pre.trim();
            if !pre.is_empty() {
                prompt.push_str(pre);
                prompt.push(' ');
            }
        }

        // Aggiunge i termini finche' c'e' spazio: troncare a termine intero
        // evita di lasciare nel prompt un nome mutilato.
        let mut inseriti = 0usize;
        for term in &terms {
            let sep = if inseriti == 0 { "" } else { ", " };
            // +1 per il punto finale
            if prompt.chars().count() + sep.len() + term.chars().count() + 1 > max_chars {
                break;
            }
            prompt.push_str(sep);
            prompt.push_str(term);
            inseriti += 1;
        }

        if inseriti > 0 {
            prompt.push('.');
        }
        if inseriti < terms.len() {
            warn!(
                inseriti,
                totali = terms.len(),
                max_chars,
                "prompt troncato: i termini oltre il limite sono stati esclusi \
                 (Whisper accetta al massimo ~224 token di contesto)"
            );
        }
    }

    // Il prompt libero da solo potrebbe comunque eccedere il limite.
    if prompt.chars().count() > max_chars {
        let cut: String = prompt.chars().take(max_chars).collect();
        warn!(max_chars, "prompt libero troncato");
        prompt = cut;
    }

    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Ok(None);
    }

    info!(
        caratteri = prompt.chars().count(),
        termini = terms.len(),
        "initial prompt costruito"
    );
    debug!(prompt = %prompt, "initial prompt");
    Ok(Some(prompt))
}

/// Legge i termini dal CSV: una colonna, senza duplicati, nell'ordine del file.
pub fn load_terms(
    path: &Path,
    column: Option<&str>,
    delimiter: Option<char>,
) -> Result<Vec<String>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("lettura del CSV dei termini {}", path.display()))?;
    if raw.trim().is_empty() {
        warn!(file = %path.display(), "il CSV dei termini e' vuoto");
        return Ok(Vec::new());
    }

    let delim = match delimiter {
        Some(c) => {
            let mut buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut buf);
            if encoded.len() != 1 {
                bail!("il delimitatore CSV deve essere un singolo byte ASCII, ricevuto {c:?}");
            }
            encoded.as_bytes()[0]
        }
        None => detect_delimiter(&raw),
    };
    debug!(delimitatore = %(delim as char), "delimitatore CSV");

    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(false)
        .flexible(true)
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_reader(raw.as_bytes());

    let records: Vec<csv::StringRecord> = reader
        .records()
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("parsing del CSV {}", path.display()))?;

    let Some(first) = records.first() else {
        return Ok(Vec::new());
    };

    // La colonna richiesta per nome implica che la prima riga sia l'header;
    // altrimenti l'header viene riconosciuto per euristica.
    let column_index_by_name = column.and_then(|c| c.parse::<usize>().err().map(|_| c));
    let has_header = column_index_by_name.is_some() || looks_like_header(first);

    let index = match column {
        None => 0,
        Some(spec) => match spec.parse::<usize>() {
            Ok(i) => i,
            Err(_) => first
                .iter()
                .position(|h| h.trim().eq_ignore_ascii_case(spec.trim()))
                .with_context(|| {
                    format!(
                        "colonna {spec:?} non trovata nell'intestazione di {}: colonne disponibili {:?}",
                        path.display(),
                        first.iter().collect::<Vec<_>>()
                    )
                })?,
        },
    };

    let rows = if has_header { &records[1..] } else { &records[..] };

    let mut out: Vec<String> = Vec::with_capacity(rows.len());
    let mut visti: HashSet<String> = HashSet::new();
    let mut righe_corte = 0usize;

    for rec in rows {
        let Some(field) = rec.get(index) else {
            righe_corte += 1;
            continue;
        };
        let term = field.trim();
        if term.is_empty() {
            continue;
        }
        // Deduplica ignorando maiuscole/minuscole, ma conserva la prima grafia:
        // e' quella che vogliamo suggerire a Whisper.
        if visti.insert(term.to_lowercase()) {
            out.push(term.to_string());
        }
    }

    if righe_corte > 0 {
        warn!(righe = righe_corte, colonna = index, "righe senza la colonna richiesta, ignorate");
    }

    info!(
        file = %path.display(),
        termini = out.len(),
        colonna = index,
        intestazione = has_header,
        "vocabolario CSV caricato"
    );
    Ok(out)
}

/// Deduce il delimitatore contando le occorrenze nella prima riga utile.
fn detect_delimiter(raw: &str) -> u8 {
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("");

    (*b",;\t|")
        .into_iter()
        .max_by_key(|&d| line.bytes().filter(|&b| b == d).count())
        .filter(|&d| line.bytes().any(|b| b == d))
        // File a colonna singola: qualunque delimitatore va bene.
        .unwrap_or(b',')
}

/// Riconosce una riga di intestazione: tutti i campi sono nomi di colonna
/// tipici. Un elenco di nomi propri non viene mai scambiato per header.
fn looks_like_header(rec: &csv::StringRecord) -> bool {
    let mut campi = 0usize;
    for f in rec.iter() {
        let f = f.trim().to_lowercase();
        if f.is_empty() {
            continue;
        }
        campi += 1;
        if !HEADER_HINTS.contains(&f.as_str()) {
            return false;
        }
    }
    campi > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scrivi(nome: &str, contenuto: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("autosubtitler_test_{nome}.csv"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contenuto.as_bytes()).unwrap();
        path
    }

    #[test]
    fn colonna_singola_senza_intestazione() {
        let p = scrivi("singola", "Anthropic\nZerflyne\n\n# commento\nOpus\n");
        let t = load_terms(&p, None, None).unwrap();
        assert_eq!(t, vec!["Anthropic", "Zerflyne", "Opus"]);
    }

    #[test]
    fn intestazione_riconosciuta_e_scartata() {
        let p = scrivi("header", "termine,categoria\nAnthropic,azienda\nOpus,modello\n");
        let t = load_terms(&p, None, None).unwrap();
        assert_eq!(t, vec!["Anthropic", "Opus"]);
    }

    #[test]
    fn nomi_propri_non_scambiati_per_intestazione() {
        let p = scrivi("noheader", "Anthropic,azienda\nOpus,modello\n");
        let t = load_terms(&p, None, None).unwrap();
        assert_eq!(t, vec!["Anthropic", "Opus"]);
    }

    #[test]
    fn colonna_per_nome() {
        let p = scrivi("byname", "categoria,termine\nazienda,Anthropic\nmodello,Opus\n");
        let t = load_terms(&p, Some("termine"), None).unwrap();
        assert_eq!(t, vec!["Anthropic", "Opus"]);
    }

    #[test]
    fn colonna_per_indice() {
        let p = scrivi("byindex", "azienda,Anthropic\nmodello,Opus\n");
        let t = load_terms(&p, Some("1"), None).unwrap();
        assert_eq!(t, vec!["Anthropic", "Opus"]);
    }

    #[test]
    fn punto_e_virgola_riconosciuto() {
        let p = scrivi("semicolon", "Anthropic;azienda\nOpus;modello\n");
        let t = load_terms(&p, None, None).unwrap();
        assert_eq!(t, vec!["Anthropic", "Opus"]);
    }

    #[test]
    fn virgolette_preservano_le_virgole() {
        let p = scrivi("quoted", "\"Milano, Italia\"\nRoma\n");
        let t = load_terms(&p, None, None).unwrap();
        assert_eq!(t, vec!["Milano, Italia", "Roma"]);
    }

    #[test]
    fn duplicati_rimossi_conservando_la_prima_grafia() {
        let p = scrivi("dup", "Anthropic\nANTHROPIC\nanthropic\nOpus\n");
        let t = load_terms(&p, None, None).unwrap();
        assert_eq!(t, vec!["Anthropic", "Opus"]);
    }

    #[test]
    fn colonna_inesistente_e_un_errore_esplicito() {
        let p = scrivi("missing", "termine,categoria\nAnthropic,azienda\n");
        let err = load_terms(&p, Some("inesistente"), None).unwrap_err();
        assert!(err.to_string().contains("inesistente"), "{err}");
    }

    #[test]
    fn prompt_composto_da_testo_libero_e_termini() {
        let p = scrivi("build", "Anthropic\nZerflyne\n");
        let cfg = PromptConfig {
            csv: Some(p),
            free_text: Some("Intervista tecnica.".into()),
            preamble: Some("Termini:".into()),
            max_chars: DEFAULT_MAX_CHARS,
            ..Default::default()
        };
        let out = build(&cfg).unwrap().unwrap();
        assert_eq!(out, "Intervista tecnica. Termini: Anthropic, Zerflyne.");
    }

    #[test]
    fn prompt_troncato_a_termine_intero() {
        let p = scrivi("trunc", "aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc\n");
        let cfg = PromptConfig { csv: Some(p), max_chars: 25, ..Default::default() };
        let out = build(&cfg).unwrap().unwrap();
        // due termini (10 + 2 + 10 + 1 = 23) entrano, il terzo no
        assert_eq!(out, "aaaaaaaaaa, bbbbbbbbbb.");
        assert!(out.chars().count() <= 25);
    }

    #[test]
    fn nessuna_configurazione_nessun_prompt() {
        assert!(build(&PromptConfig::default()).unwrap().is_none());
    }

    #[test]
    fn csv_vuoto_non_produce_prompt() {
        let p = scrivi("vuoto", "\n# solo commenti\n");
        let cfg = PromptConfig { csv: Some(p), ..Default::default() };
        assert!(build(&cfg).unwrap().is_none());
    }
}
