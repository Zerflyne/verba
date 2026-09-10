//! Allineamento forzato parola-per-parola con **wav2vec2-italian esportato in
//! ONNX** (testa CTC).
//!
//! Il modello produce log-probabilita' per frame (~20 ms) su un vocabolario di
//! caratteri. Dato il testo gia' noto (da Whisper), l'allineamento e' un
//! problema di *forced alignment* CTC risolto con Viterbi sulla sequenza
//! estesa `[blank, c1, blank, c2, ..., blank]`.
//!
//! Il risultato e' l'intervallo temporale di ogni carattere, aggregato poi in
//! intervalli di parola.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use ndarray::Array2;
use ort::session::Session;
use ort::value::TensorRef;
use tracing::{debug, info, warn};

use crate::audio::{self, Pcm};
use crate::gpu::{self, Device};
use crate::onnx::{build_session, log_softmax};
use crate::transcribe::Transcript;
use crate::trascrizione::{Parola, Trascrizione};

#[derive(Debug, Clone)]
pub struct AlignConfig {
    /// Normalizzazione media-nulla/varianza-unitaria (feature extractor
    /// wav2vec2 con `do_normalize = true`).
    pub do_normalize: bool,
    /// Durata massima di un blocco inviato al modello, in secondi.
    pub max_chunk_secs: f64,
    /// Sotto questa confidenza la parola viene segnalata nel log.
    pub low_score_warn: f32,
}

impl Default for AlignConfig {
    fn default() -> Self {
        Self { do_normalize: true, max_chunk_secs: 30.0, low_score_warn: 0.10 }
    }
}

/// Vocabolario CTC caricato da `vocab.json` del tokenizer wav2vec2.
struct Vocab {
    map: HashMap<String, usize>,
    blank: usize,
    delimiter: Option<usize>,
    lowercase: bool,
}

impl Vocab {
    fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("lettura del vocabolario {}", path.display()))?;
        let parsed: HashMap<String, usize> =
            serde_json::from_str(&raw).context("vocab.json malformato (atteso {token: id})")?;
        if parsed.is_empty() {
            bail!("vocabolario vuoto");
        }

        // Il blank CTC e' <pad> nei tokenizer HuggingFace; in mancanza, id 0.
        let blank = parsed
            .get("<pad>")
            .or_else(|| parsed.get("[PAD]"))
            .copied()
            .unwrap_or(0);

        let delimiter = parsed.get("|").copied().or_else(|| parsed.get(" ").copied());

        // Alcuni modelli italiani usano il vocabolario minuscolo, altri
        // maiuscolo: lo deduciamo dai token presenti.
        let lowercase = parsed.contains_key("a");

        info!(
            token = parsed.len(),
            blank,
            delimiter = ?delimiter,
            minuscolo = lowercase,
            "vocabolario wav2vec2 caricato"
        );
        Ok(Self { map: parsed, blank, delimiter, lowercase })
    }

    fn id(&self, ch: char) -> Option<usize> {
        let mut buf = [0u8; 4];
        let s: &str = ch.encode_utf8(&mut buf);
        self.map.get(s).copied()
    }

    /// Normalizza una parola nella forma accettata dal vocabolario, scartando
    /// i caratteri non rappresentabili (punteggiatura, simboli).
    fn normalize_word(&self, word: &str) -> Vec<usize> {
        let folded: String = if self.lowercase {
            word.to_lowercase()
        } else {
            word.to_uppercase()
        };
        let mut ids = Vec::with_capacity(folded.chars().count());
        for ch in folded.chars() {
            if let Some(id) = self.id(ch) {
                ids.push(id);
            } else if let Some(stripped) = strip_accent(ch) {
                // "e'" per "è" quando il vocabolario non ha le accentate
                for c2 in stripped.chars() {
                    if let Some(id) = self.id(c2) {
                        ids.push(id);
                    }
                }
            }
        }
        ids
    }
}

/// Traslittera le accentate quando mancano dal vocabolario.
fn strip_accent(ch: char) -> Option<&'static str> {
    Some(match ch {
        'à' | 'á' | 'â' | 'ä' => "a",
        'è' | 'é' | 'ê' | 'ë' => "e",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'ò' | 'ó' | 'ô' | 'ö' => "o",
        'ù' | 'ú' | 'û' | 'ü' => "u",
        'ç' => "c",
        'À' | 'Á' | 'Â' | 'Ä' => "A",
        'È' | 'É' | 'Ê' | 'Ë' => "E",
        'Ì' | 'Í' | 'Î' | 'Ï' => "I",
        'Ò' | 'Ó' | 'Ô' | 'Ö' => "O",
        'Ù' | 'Ú' | 'Û' | 'Ü' => "U",
        'Ç' => "C",
        _ => return None,
    })
}

pub struct Aligner {
    session: Session,
    vocab: Vocab,
    cfg: AlignConfig,
    input_name: String,
    input_rank: usize,
}

impl Aligner {
    pub fn new(
        model: &Path,
        vocab: &Path,
        device: &Device,
        cfg: AlignConfig,
        threads: usize,
    ) -> Result<Self> {
        let session = build_session(model, device, threads)?;
        let vocab = Vocab::load(vocab)?;

        let input = session.inputs.first().context("il modello di allineamento non espone input")?;
        let input_name = input.name.clone();
        let input_rank = input.input_type.tensor_shape().map(|d| d.len()).unwrap_or(2);

        gpu::log_vram(device, "dopo il caricamento dell'allineatore");
        Ok(Self { session, vocab, cfg, input_name, input_rank })
    }

    /// Allinea tutti i segmenti trascritti.
    ///
    /// Il risultato grezzo esce di qui e finisce in una [`Trascrizione`], che
    /// lo conserva accanto alla versione normalizzata: da quel momento in poi
    /// il resto del programma lavora sulla seconda e puo' sempre risalire alla
    /// prima.
    pub fn run(&mut self, pcm: &Pcm, transcripts: &[Transcript]) -> Result<Trascrizione> {
        let mut words = Vec::new();
        let mut falliti = 0usize;

        for (idx, tr) in transcripts.iter().enumerate() {
            let seg_words: Vec<&str> = tr.text.split_whitespace().collect();
            if seg_words.is_empty() {
                continue;
            }

            let samples = pcm.slice_secs(tr.segment.start, tr.segment.end);
            let aligned = match self.align_segment(samples, pcm.sample_rate, &seg_words) {
                Ok(spans) => spans,
                Err(e) => {
                    warn!(segmento = idx, error = %e, "allineamento fallito: ripartizione proporzionale");
                    falliti += 1;
                    fallback_spans(&seg_words, tr.segment.duration())
                }
            };

            for (w, (rel_start, rel_end, score)) in seg_words.iter().zip(aligned) {
                words.push(Parola {
                    id: Default::default(),
                    testo: (*w).to_string(),
                    inizio: tr.segment.start + rel_start,
                    fine: tr.segment.start + rel_end,
                    confidenza: score,
                    segmento: idx,
                });
            }
        }

        let trascrizione = Trascrizione::nuova(words, pcm.duration_secs());

        let deboli = trascrizione.incerte(self.cfg.low_score_warn).count();
        info!(
            parole = trascrizione.len(),
            segmenti_in_fallback = falliti,
            parole_a_bassa_confidenza = deboli,
            "allineamento parola-per-parola completato"
        );
        Ok(trascrizione)
    }

    /// Allinea un singolo segmento: ritorna (inizio, fine, score) relativi al
    /// segmento, una tupla per parola.
    fn align_segment(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
        words: &[&str],
    ) -> Result<Vec<(f64, f64, f32)>> {
        if samples.is_empty() {
            bail!("segmento vuoto");
        }
        let max_len = (self.cfg.max_chunk_secs * sample_rate as f64) as usize;
        if samples.len() > max_len {
            bail!("segmento piu' lungo del blocco massimo ({} s)", self.cfg.max_chunk_secs);
        }

        // 1. emissioni CTC
        let input: Vec<f32> = if self.cfg.do_normalize {
            audio::zero_mean_unit_var(samples)
        } else {
            samples.to_vec()
        };
        let (frames, vocab_size, mut logits) = self.infer(&input)?;

        for t in 0..frames {
            log_softmax(&mut logits[t * vocab_size..(t + 1) * vocab_size]);
        }

        // 2. sequenza di token da allineare, con i confini di parola
        let mut tokens: Vec<usize> = Vec::new();
        let mut word_ranges: Vec<(usize, usize)> = Vec::new(); // [inizio, fine) sui token
        for (i, w) in words.iter().enumerate() {
            let ids = self.vocab.normalize_word(w);
            if ids.is_empty() {
                // parola non rappresentabile (es. solo punteggiatura):
                // la registriamo come intervallo vuoto, sara' interpolata dopo
                word_ranges.push((tokens.len(), tokens.len()));
                continue;
            }
            if i > 0 && !tokens.is_empty() {
                if let Some(d) = self.vocab.delimiter {
                    tokens.push(d);
                }
            }
            let start = tokens.len();
            tokens.extend_from_slice(&ids);
            word_ranges.push((start, tokens.len()));
        }
        if tokens.is_empty() {
            bail!("nessun token allineabile nel testo");
        }

        // 3. Viterbi CTC
        let path = viterbi_ctc(&logits, frames, vocab_size, &tokens, self.vocab.blank)?;

        // 4. da frame a secondi
        let secs_per_frame = samples.len() as f64 / sample_rate as f64 / frames as f64;
        let seg_dur = samples.len() as f64 / sample_rate as f64;

        let mut out = Vec::with_capacity(words.len());
        for (i, &(a, b)) in word_ranges.iter().enumerate() {
            if a == b {
                // numeri, simboli, punteggiatura isolata: nessun token CTC da
                // agganciare. Il tempo lo assegna `ripulisci`.
                out.push((f64::NAN, f64::NAN, 0.0));
                continue;
            }
            let first = path[a..b].iter().map(|s| s.first_frame).min().unwrap_or(0);
            let last = path[a..b].iter().map(|s| s.last_frame).max().unwrap_or(first);
            let score = path[a..b].iter().map(|s| s.score).sum::<f32>() / (b - a) as f32;

            let start = (first as f64 * secs_per_frame).clamp(0.0, seg_dur);
            let end = (((last + 1) as f64) * secs_per_frame).clamp(start, seg_dur);
            debug!(parola = %words[i], inizio = start, fine = end, score, "allineata");
            out.push((start, end, score.exp().clamp(0.0, 1.0)));
        }

        // I buchi (parole non rappresentabili nel vocabolario) restano NaN:
        // li risolve `ripulisci` sull'intera sequenza, cosi' l'interpolazione
        // puo' usare anche i vicini che stanno nel segmento accanto.
        Ok(out)
    }

    /// Esegue il modello ONNX; ritorna (frame, dimensione vocabolario, logits).
    fn infer(&mut self, input: &[f32]) -> Result<(usize, usize, Vec<f32>)> {
        let n = input.len();
        let array = if self.input_rank == 3 {
            Array2::from_shape_vec((1, n), input.to_vec())?
                .into_shape_with_order((1, 1, n))?
                .into_dyn()
        } else {
            Array2::from_shape_vec((1, n), input.to_vec())?.into_dyn()
        };

        let tensor = TensorRef::from_array_view(&array)?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => tensor])
            .context("inferenza di allineamento")?;

        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .context("estrazione dei logits CTC")?;

        let dims: Vec<usize> = shape.iter().map(|d| *d as usize).collect();
        let (frames, vocab_size) = match dims.as_slice() {
            [_b, t, v] => (*t, *v),
            [t, v] => (*t, *v),
            other => bail!("forma inattesa dei logits: {other:?}"),
        };
        if frames == 0 {
            bail!("il modello non ha prodotto frame");
        }
        Ok((frames, vocab_size, data.to_vec()))
    }
}

/// Intervallo di frame assegnato a un token della sequenza.
#[derive(Debug, Clone, Copy)]
struct TokenSpan {
    first_frame: usize,
    last_frame: usize,
    /// log-probabilita' media dei frame assegnati.
    score: f32,
}

/// Allineamento forzato CTC (Viterbi) sulla sequenza estesa con blank.
///
/// `logits` e' log-softmax in layout riga-maggiore `[frames, vocab]`.
fn viterbi_ctc(
    logits: &[f32],
    frames: usize,
    vocab: usize,
    tokens: &[usize],
    blank: usize,
) -> Result<Vec<TokenSpan>> {
    let n = tokens.len();
    let s_len = 2 * n + 1;
    if frames < s_len.div_ceil(2) {
        bail!("frame insufficienti ({frames}) per {n} token");
    }

    // sequenza estesa: blank fra ogni coppia di token e agli estremi
    let mut ext = vec![blank; s_len];
    for (i, &t) in tokens.iter().enumerate() {
        if t >= vocab {
            bail!("id di token {t} fuori dal vocabolario ({vocab})");
        }
        ext[2 * i + 1] = t;
    }

    const NEG: f32 = -1.0e30;
    let lp = |t: usize, sym: usize| logits[t * vocab + sym];

    let mut prev = vec![NEG; s_len];
    let mut cur = vec![NEG; s_len];
    // 0 = resta, 1 = da s-1, 2 = da s-2
    let mut back = vec![0u8; frames * s_len];

    prev[0] = lp(0, ext[0]);
    if s_len > 1 {
        prev[1] = lp(0, ext[1]);
    }

    for t in 1..frames {
        for s in 0..s_len {
            let mut best = prev[s];
            let mut arg = 0u8;
            if s >= 1 && prev[s - 1] > best {
                best = prev[s - 1];
                arg = 1;
            }
            // il salto di 2 e' vietato verso un blank e fra token identici
            // consecutivi (che richiedono un blank di separazione)
            if s >= 2 && ext[s] != blank && ext[s] != ext[s - 2] && prev[s - 2] > best {
                best = prev[s - 2];
                arg = 2;
            }
            cur[s] = if best <= NEG { NEG } else { best + lp(t, ext[s]) };
            back[t * s_len + s] = arg;
        }
        std::mem::swap(&mut prev, &mut cur);
    }

    // stato finale: ultimo token o blank finale
    let mut s = if s_len >= 2 && prev[s_len - 2] > prev[s_len - 1] {
        s_len - 2
    } else {
        s_len - 1
    };
    if prev[s] <= NEG {
        bail!("nessun percorso di allineamento valido");
    }

    // backtracking: per ogni frame lo stato attraversato
    let mut path_states = vec![0usize; frames];
    for t in (0..frames).rev() {
        path_states[t] = s;
        if t > 0 {
            s -= back[t * s_len + s] as usize;
        }
    }

    // aggregazione per token reale (stati dispari)
    let mut spans = vec![
        TokenSpan { first_frame: usize::MAX, last_frame: 0, score: 0.0 };
        n
    ];
    let mut counts = vec![0usize; n];
    for (t, &st) in path_states.iter().enumerate() {
        if st % 2 == 0 {
            continue; // blank
        }
        let k = (st - 1) / 2;
        let sp = &mut spans[k];
        sp.first_frame = sp.first_frame.min(t);
        sp.last_frame = sp.last_frame.max(t);
        sp.score += lp(t, ext[st]);
        counts[k] += 1;
    }

    // token mai emessi (possibile ai bordi): eredita dal vicino
    for k in 0..n {
        if counts[k] == 0 {
            let neighbour = if k > 0 { spans[k - 1] } else { spans[(k + 1).min(n - 1)] };
            spans[k] = TokenSpan {
                first_frame: neighbour.last_frame,
                last_frame: neighbour.last_frame,
                score: -6.0, // confidenza bassa: exp(-6) ~ 0.0025
            };
        } else {
            spans[k].score /= counts[k] as f32;
        }
    }

    Ok(spans)
}

/// Ripartizione proporzionale alla lunghezza delle parole: usata quando
/// l'allineamento CTC non e' applicabile.
fn fallback_spans(words: &[&str], duration: f64) -> Vec<(f64, f64, f32)> {
    let total: usize = words.iter().map(|w| w.chars().count().max(1)).sum();
    let mut out = Vec::with_capacity(words.len());
    let mut t = 0.0;
    for w in words {
        let share = duration * w.chars().count().max(1) as f64 / total as f64;
        out.push((t, (t + share).min(duration), 0.0));
        t += share;
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    fn logits_from(seq: &[usize], vocab: usize, repeat: usize) -> (Vec<f32>, usize) {
        // costruisce emissioni "pulite": ogni simbolo domina per `repeat` frame
        let frames = seq.len() * repeat;
        let mut v = vec![-10.0f32; frames * vocab];
        for (i, &s) in seq.iter().enumerate() {
            for r in 0..repeat {
                v[(i * repeat + r) * vocab + s] = 0.0;
            }
        }
        (v, frames)
    }

    #[test]
    fn viterbi_recupera_gli_intervalli() {
        // vocabolario: 0=blank, 1='a', 2='b'
        let (logits, frames) = logits_from(&[0, 1, 1, 0, 2, 2, 0], 3, 1);
        let spans = viterbi_ctc(&logits, frames, 3, &[1, 2], 0).unwrap();
        assert_eq!(spans[0].first_frame, 1);
        assert_eq!(spans[0].last_frame, 2);
        assert_eq!(spans[1].first_frame, 4);
        assert_eq!(spans[1].last_frame, 5);
    }

    #[test]
    fn viterbi_gestisce_token_ripetuti() {
        // "aa" richiede un blank di separazione
        let (logits, frames) = logits_from(&[1, 0, 1], 3, 1);
        let spans = viterbi_ctc(&logits, frames, 3, &[1, 1], 0).unwrap();
        assert_eq!(spans[0].first_frame, 0);
        assert_eq!(spans[1].first_frame, 2);
    }

    #[test]
    fn frame_insufficienti_falliscono() {
        let (logits, frames) = logits_from(&[1], 3, 1);
        assert!(viterbi_ctc(&logits, frames, 3, &[1, 1, 1, 1], 0).is_err());
    }

    #[test]
    fn fallback_copre_tutta_la_durata() {
        let words = ["ciao", "mondo"];
        let refs: Vec<&str> = words.to_vec();
        let spans = fallback_spans(&refs, 2.0);
        assert!((spans[1].1 - 2.0).abs() < 1e-6);
    }
}
