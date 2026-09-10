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
use serde::Serialize;
use tracing::{debug, info, warn};

use crate::audio::{self, Pcm};
use crate::gpu::{self, Device};
use crate::onnx::{build_session, log_softmax};
use crate::transcribe::Transcript;

/// Una parola con i suoi tempi assoluti nel file originale.
#[derive(Debug, Clone, Serialize)]
pub struct Word {
    /// Testo come mostrato all'utente (punteggiatura e maiuscole preservate).
    pub text: String,
    pub start: f64,
    pub end: f64,
    /// Confidenza media dell'allineamento, in [0, 1].
    pub score: f32,
    /// Indice del segmento di provenienza.
    pub segment: usize,
}

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

    /// Allinea tutti i segmenti trascritti; ritorna le parole in ordine
    /// temporale con tempi assoluti.
    pub fn run(&mut self, pcm: &Pcm, transcripts: &[Transcript]) -> Result<Vec<Word>> {
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
                words.push(Word {
                    text: (*w).to_string(),
                    start: tr.segment.start + rel_start,
                    end: tr.segment.start + rel_end,
                    score,
                    segment: idx,
                });
            }
        }

        let words = ripulisci(words, pcm.duration_secs(), DURATA_MINIMA_PAROLA);

        let deboli = words.iter().filter(|w| w.score < self.cfg.low_score_warn).count();
        info!(
            parole = words.len(),
            segmenti_in_fallback = falliti,
            parole_a_bassa_confidenza = deboli,
            "allineamento parola-per-parola completato"
        );
        Ok(words)
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

/// Durata minima attribuita a una parola, in secondi.
pub const DURATA_MINIMA_PAROLA: f64 = 0.04;

/// Normalizza la sequenza di parole prodotta dall'allineamento.
///
/// A valle (raggruppamento in battute, resa SRT, export JSON) si assume una
/// sequenza **ordinata e senza buchi**: questa funzione e' il punto in cui
/// quell'invariante viene stabilita, una volta sola.
///
/// In ordine:
///
/// 1. **scarta le parole vuote** (testo assente o solo spazi);
/// 2. **riempie i timestamp mancanti**: l'allineatore CTC non aggancia numeri
///    e simboli, che non hanno una grafia nel vocabolario dei caratteri. Il
///    tempo si ricava interpolando fra i vicini noti — la fine della parola
///    valida precedente e l'inizio della successiva — e quando le parole senza
///    tempo sono piu' d'una di fila l'intervallo viene spartito equamente fra
///    loro. Agli estremi le ancore sono 0 e la durata dell'audio;
/// 3. **impone la monotonia**: nessuna parola inizia prima che finisca la
///    precedente;
/// 4. **impone la durata minima** `durata_minima` per ogni parola;
/// 5. **tronca alla durata dell'audio**: nessun timestamp la oltrepassa.
///
/// I due ultimi vincoli possono entrare in conflitto in coda al file (non
/// resta spazio per la durata minima): li' vince il troncamento, perche' un
/// sottotitolo che punta oltre la fine del media e' un errore visibile mentre
/// una battuta corta non lo e'.
///
/// Passare `durata_audio <= 0` disattiva il solo troncamento (utile quando la
/// durata non e' nota); il resto della normalizzazione viene comunque applicato.
pub fn ripulisci(words: Vec<Word>, durata_audio: f64, durata_minima: f64) -> Vec<Word> {
    let iniziali = words.len();

    // 1. parole vuote: non hanno nulla da mostrare e falserebbero le ancore
    //    temporali delle vicine.
    let mut words: Vec<Word> = words
        .into_iter()
        .filter_map(|mut w| {
            let testo = w.text.trim();
            if testo.is_empty() {
                return None;
            }
            if testo.len() != w.text.len() {
                w.text = testo.to_string();
            }
            Some(w)
        })
        .collect();

    let scartate = iniziali - words.len();
    if words.is_empty() {
        if scartate > 0 {
            warn!(scartate, "tutte le parole erano vuote");
        }
        return words;
    }

    // 2. timestamp mancanti. Si lavora nell'ordine di produzione, che e' gia'
    //    quello del parlato: ordinare adesso, con i NaN in mezzo, li
    //    ammasserebbe in fondo e distruggerebbe il contesto dei vicini.
    let interpolate = riempi_tempi_mancanti(&mut words, durata_audio);

    // 3. ora tutti i tempi sono finiti e l'ordinamento e' ben definito.
    //    `sort_by` e' stabile: a parita' di inizio l'ordine del parlato resta.
    words.sort_by(|a, b| a.start.total_cmp(&b.start));

    // 4+5. monotonia, durata minima, troncamento.
    let limite = if durata_audio > 0.0 { durata_audio } else { f64::INFINITY };
    let durata_minima = durata_minima.max(0.0);
    let mut corrette = 0usize;
    let mut fine_precedente = 0.0f64;

    for w in words.iter_mut() {
        let (start0, end0) = (w.start, w.end);

        w.start = w.start.clamp(0.0, limite).max(fine_precedente);
        w.end = w.end.max(w.start + durata_minima);

        if w.end > limite {
            // In coda al file il troncamento ha la precedenza sulla durata
            // minima: la parola puo' restare piu' corta, mai sforare.
            w.end = limite;
            w.start = w.start.min(w.end);
        }

        if (w.start - start0).abs() > 1e-9 || (w.end - end0).abs() > 1e-9 {
            corrette += 1;
        }
        fine_precedente = w.end;
    }

    if scartate > 0 || interpolate > 0 || corrette > 0 {
        debug!(
            scartate,
            interpolate,
            corrette,
            parole = words.len(),
            "sequenza di parole normalizzata"
        );
    }
    if interpolate > 0 {
        info!(
            parole = interpolate,
            "timestamp stimati per interpolazione (numeri o simboli non agganciabili dall'allineatore)"
        );
    }

    words
}

/// Una parola ha un tempo utilizzabile solo se entrambi gli estremi sono
/// finiti: NaN e infiniti valgono "tempo mancante".
fn ha_tempo(w: &Word) -> bool {
    w.start.is_finite() && w.end.is_finite()
}

/// Assegna un tempo alle parole che non ne hanno, spartendo equamente
/// l'intervallo fra i due vicini con tempo noto. Ritorna quante ne ha corrette.
fn riempi_tempi_mancanti(words: &mut [Word], durata_audio: f64) -> usize {
    let n = words.len();
    let fine_file = if durata_audio > 0.0 {
        durata_audio
    } else {
        // Senza durata nota, l'ancora destra e' la fine dell'ultimo tempo noto.
        words.iter().filter(|w| ha_tempo(w)).map(|w| w.end).fold(0.0, f64::max)
    };

    let mut totale = 0usize;
    let mut i = 0usize;

    while i < n {
        if ha_tempo(&words[i]) {
            i += 1;
            continue;
        }

        // Estensione del gruppo di parole consecutive senza tempo.
        let mut j = i;
        while j < n && !ha_tempo(&words[j]) {
            j += 1;
        }

        // Ancore: la fine del vicino sinistro (gia' risolto dai giri
        // precedenti) e l'inizio del vicino destro.
        let sinistra = if i > 0 { words[i - 1].end } else { 0.0 };
        let destra = if j < n { words[j].start } else { fine_file };
        let destra = destra.max(sinistra);

        let quante = j - i;
        let passo = (destra - sinistra) / quante as f64;

        for (k, w) in words[i..j].iter_mut().enumerate() {
            w.start = sinistra + k as f64 * passo;
            w.end = sinistra + (k + 1) as f64 * passo;
            // Il tempo e' stimato, non misurato: la confidenza lo dichiara.
            w.score = 0.0;
        }

        totale += quante;
        i = j;
    }

    totale
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

    fn parola(text: &str, start: f64, end: f64) -> Word {
        Word { text: text.into(), start, end, score: 1.0, segment: 0 }
    }

    /// Parola senza timestamp, come la produce l'allineatore su numeri e simboli.
    fn senza_tempo(text: &str) -> Word {
        Word { text: text.into(), start: f64::NAN, end: f64::NAN, score: 0.0, segment: 0 }
    }

    #[test]
    fn ripulisci_scarta_le_parole_vuote() {
        let w = vec![parola("ciao", 0.0, 0.5), parola("   ", 0.5, 0.6), parola("", 0.6, 0.7)];
        let out = ripulisci(w, 10.0, DURATA_MINIMA_PAROLA);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "ciao");
    }

    #[test]
    fn ripulisci_interpola_una_parola_isolata() {
        let w = vec![parola("il", 1.0, 2.0), senza_tempo("42"), parola("euro", 3.0, 4.0)];
        let out = ripulisci(w, 10.0, DURATA_MINIMA_PAROLA);
        assert_eq!(out[1].text, "42");
        assert!((out[1].start - 2.0).abs() < 1e-9, "{:?}", out[1]);
        assert!((out[1].end - 3.0).abs() < 1e-9, "{:?}", out[1]);
        // il tempo e' stimato: la confidenza lo dichiara
        assert_eq!(out[1].score, 0.0);
    }

    #[test]
    fn ripulisci_spartisce_equamente_piu_parole_consecutive() {
        let w = vec![
            parola("sono", 0.0, 1.0),
            senza_tempo("3"),
            senza_tempo("+"),
            senza_tempo("4"),
            parola("totale", 4.0, 5.0),
        ];
        let out = ripulisci(w, 10.0, DURATA_MINIMA_PAROLA);
        for (i, atteso) in [1.0, 2.0, 3.0].into_iter().enumerate() {
            assert!(
                (out[i + 1].start - atteso).abs() < 1e-9,
                "parola {i}: {:?}",
                out[i + 1]
            );
        }
        assert!((out[3].end - 4.0).abs() < 1e-9);
    }

    #[test]
    fn ripulisci_ancora_agli_estremi_del_file() {
        let w = vec![senza_tempo("1"), parola("euro", 2.0, 3.0), senza_tempo("2")];
        let out = ripulisci(w, 5.0, DURATA_MINIMA_PAROLA);
        // in testa l'ancora sinistra e' 0
        assert!((out[0].start - 0.0).abs() < 1e-9);
        assert!((out[0].end - 2.0).abs() < 1e-9);
        // in coda l'ancora destra e' la durata dell'audio
        assert!((out[2].start - 3.0).abs() < 1e-9);
        assert!((out[2].end - 5.0).abs() < 1e-9);
    }

    #[test]
    fn ripulisci_gestisce_tutte_le_parole_senza_tempo() {
        let w = vec![senza_tempo("uno"), senza_tempo("due"), senza_tempo("tre")];
        let out = ripulisci(w, 3.0, DURATA_MINIMA_PAROLA);
        assert!(out.iter().all(|w| w.start.is_finite() && w.end.is_finite()));
        assert!((out[0].start - 0.0).abs() < 1e-9);
        assert!((out[1].start - 1.0).abs() < 1e-9);
        assert!((out[2].end - 3.0).abs() < 1e-9);
    }

    #[test]
    fn ripulisci_impone_la_monotonia() {
        let w = vec![parola("a", 0.0, 1.0), parola("b", 0.5, 1.5), parola("c", 0.2, 2.0)];
        let out = ripulisci(w, 10.0, DURATA_MINIMA_PAROLA);
        for pair in out.windows(2) {
            assert!(pair[1].start >= pair[0].end, "{pair:?}");
        }
    }

    #[test]
    fn ripulisci_impone_la_durata_minima() {
        let w = vec![parola("a", 1.0, 1.0), parola("b", 2.0, 2.001)];
        let out = ripulisci(w, 10.0, DURATA_MINIMA_PAROLA);
        assert!(out.iter().all(|w| w.end - w.start >= DURATA_MINIMA_PAROLA - 1e-9), "{out:?}");
    }

    #[test]
    fn ripulisci_tronca_alla_durata_dellaudio() {
        let w = vec![parola("a", 4.0, 12.0), parola("b", 20.0, 30.0)];
        let out = ripulisci(w, 5.0, DURATA_MINIMA_PAROLA);
        assert!(out.iter().all(|w| w.end <= 5.0 + 1e-9), "{out:?}");
        assert!(out.iter().all(|w| w.start <= w.end), "{out:?}");
    }

    #[test]
    fn ripulisci_lascia_invariata_una_sequenza_gia_pulita() {
        let w = vec![parola("uno", 0.0, 0.5), parola("due", 0.6, 1.2), parola("tre", 1.2, 2.0)];
        let out = ripulisci(w.clone(), 10.0, DURATA_MINIMA_PAROLA);
        assert_eq!(out.len(), w.len());
        for (a, b) in out.iter().zip(w.iter()) {
            assert_eq!(a.text, b.text);
            assert!((a.start - b.start).abs() < 1e-9 && (a.end - b.end).abs() < 1e-9, "{a:?}");
        }
    }

    #[test]
    fn ripulisci_e_idempotente() {
        let w = vec![
            parola("a", 0.0, 1.0),
            senza_tempo("7"),
            parola("b", 0.5, 0.5),
            parola("", 9.0, 9.0),
        ];
        let una = ripulisci(w, 4.0, DURATA_MINIMA_PAROLA);
        let due = ripulisci(una.clone(), 4.0, DURATA_MINIMA_PAROLA);
        assert_eq!(una.len(), due.len());
        for (a, b) in una.iter().zip(due.iter()) {
            assert!((a.start - b.start).abs() < 1e-9 && (a.end - b.end).abs() < 1e-9, "{a:?} {b:?}");
        }
    }

    #[test]
    fn ripulisci_senza_durata_nota_non_tronca() {
        let w = vec![parola("a", 0.0, 1.0), parola("b", 100.0, 200.0)];
        let out = ripulisci(w, 0.0, DURATA_MINIMA_PAROLA);
        assert!((out[1].end - 200.0).abs() < 1e-9, "{out:?}");
    }

    #[test]
    fn fallback_copre_tutta_la_durata() {
        let words = ["ciao", "mondo"];
        let refs: Vec<&str> = words.to_vec();
        let spans = fallback_spans(&refs, 2.0);
        assert!((spans[1].1 - 2.0).abs() < 1e-6);
    }
}
