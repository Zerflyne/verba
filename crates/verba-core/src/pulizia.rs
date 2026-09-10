//! Normalizzazione della sequenza di parole.
//!
//! E' la funzione su cui si regge tutto il resto: a valle di qui —
//! raggruppamento in righe, disegno dei fotogrammi, export — si assume una
//! sequenza ordinata, senza buchi e senza sovrapposizioni. Per questo sta in un
//! modulo suo e ha i suoi test sui casi limite, invece di essere un passaggio
//! interno dell'allineatore.

use tracing::{debug, info, warn};

use crate::trascrizione::Parola;

/// Durata minima attribuita a una parola, in secondi.
pub const DURATA_MINIMA_PAROLA: f64 = 0.08;

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
pub fn ripulisci(parole: Vec<Parola>, durata_audio: f64, durata_minima: f64) -> Vec<Parola> {
    let iniziali = parole.len();

    // 1. parole vuote: non hanno nulla da mostrare e falserebbero le ancore
    //    temporali delle vicine.
    let mut parole: Vec<Parola> = parole
        .into_iter()
        .filter_map(|mut p| {
            let testo = p.testo.trim();
            if testo.is_empty() {
                return None;
            }
            if testo.len() != p.testo.len() {
                p.testo = testo.to_string();
            }
            Some(p)
        })
        .collect();

    let scartate = iniziali - parole.len();
    if parole.is_empty() {
        if scartate > 0 {
            warn!(scartate, "tutte le parole erano vuote");
        }
        return parole;
    }

    // 2. timestamp mancanti. Si lavora nell'ordine di produzione, che e' gia'
    //    quello del parlato: ordinare adesso, con i NaN in mezzo, li
    //    ammasserebbe in fondo e distruggerebbe il contesto dei vicini.
    let interpolate = riempi_tempi_mancanti(&mut parole, durata_audio);

    // 3. ora tutti i tempi sono finiti e l'ordinamento e' ben definito.
    //    `sort_by` e' stabile: a parita' di inizio l'ordine del parlato resta.
    parole.sort_by(|a, b| a.inizio.total_cmp(&b.inizio));

    // 4+5. monotonia, durata minima, troncamento.
    let limite = if durata_audio > 0.0 { durata_audio } else { f64::INFINITY };
    let durata_minima = durata_minima.max(0.0);
    let mut corrette = 0usize;
    let mut fine_precedente = 0.0f64;

    for p in parole.iter_mut() {
        let (start0, end0) = (p.inizio, p.fine);

        p.inizio = p.inizio.clamp(0.0, limite).max(fine_precedente);
        p.fine = p.fine.max(p.inizio + durata_minima);

        if p.fine > limite {
            // In coda al file il troncamento ha la precedenza sulla durata
            // minima: la parola puo' restare piu' corta, mai sforare.
            p.fine = limite;
            p.inizio = p.inizio.min(p.fine);
        }

        if (p.inizio - start0).abs() > 1e-9 || (p.fine - end0).abs() > 1e-9 {
            corrette += 1;
        }
        fine_precedente = p.fine;
    }

    if scartate > 0 || interpolate > 0 || corrette > 0 {
        debug!(
            scartate,
            interpolate,
            corrette,
            parole = parole.len(),
            "sequenza di parole normalizzata"
        );
    }
    if interpolate > 0 {
        info!(
            parole = interpolate,
            "timestamp stimati per interpolazione (numeri o simboli non agganciabili dall'allineatore)"
        );
    }

    parole
}

/// Una parola ha un tempo utilizzabile solo se entrambi gli estremi sono
/// finiti: NaN e infiniti valgono "tempo mancante".
fn ha_tempo(p: &Parola) -> bool {
    p.inizio.is_finite() && p.fine.is_finite()
}

/// Assegna un tempo alle parole che non ne hanno, spartendo equamente
/// l'intervallo fra i due vicini con tempo noto. Ritorna quante ne ha corrette.
fn riempi_tempi_mancanti(parole: &mut [Parola], durata_audio: f64) -> usize {
    let n = parole.len();
    let fine_file = if durata_audio > 0.0 {
        durata_audio
    } else {
        // Senza durata nota, l'ancora destra e' la fine dell'ultimo tempo noto.
        parole.iter().filter(|p| ha_tempo(p)).map(|p| p.fine).fold(0.0, f64::max)
    };

    let mut totale = 0usize;
    let mut i = 0usize;

    while i < n {
        if ha_tempo(&parole[i]) {
            i += 1;
            continue;
        }

        // Estensione del gruppo di parole consecutive senza tempo.
        let mut j = i;
        while j < n && !ha_tempo(&parole[j]) {
            j += 1;
        }

        // Ancore: la fine del vicino sinistro (gia' risolto dai giri
        // precedenti) e l'inizio del vicino destro.
        let sinistra = if i > 0 { parole[i - 1].fine } else { 0.0 };
        let destra = if j < n { parole[j].inizio } else { fine_file };
        let destra = destra.max(sinistra);

        let quante = j - i;
        let passo = (destra - sinistra) / quante as f64;

        for (k, p) in parole[i..j].iter_mut().enumerate() {
            p.inizio = sinistra + k as f64 * passo;
            p.fine = sinistra + (k + 1) as f64 * passo;
            // Il tempo e' stimato, non misurato: la confidenza lo dichiara.
            p.confidenza = 0.0;
        }

        totale += quante;
        i = j;
    }

    totale
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parola(testo: &str, inizio: f64, fine: f64) -> Parola {
        Parola::nuova(testo, inizio, fine)
    }

    /// Parola senza timestamp, come la produce l'allineatore su numeri e simboli.
    fn senza_tempo(testo: &str) -> Parola {
        let mut p = Parola::nuova(testo, f64::NAN, f64::NAN);
        p.confidenza = 0.0;
        p
    }

    #[test]
    fn ripulisci_scarta_le_parole_vuote() {
        let p = vec![parola("ciao", 0.0, 0.5), parola("   ", 0.5, 0.6), parola("", 0.6, 0.7)];
        let out = ripulisci(p, 10.0, DURATA_MINIMA_PAROLA);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].testo, "ciao");
    }

    #[test]
    fn ripulisci_interpola_una_parola_isolata() {
        let p = vec![parola("il", 1.0, 2.0), senza_tempo("42"), parola("euro", 3.0, 4.0)];
        let out = ripulisci(p, 10.0, DURATA_MINIMA_PAROLA);
        assert_eq!(out[1].testo, "42");
        assert!((out[1].inizio - 2.0).abs() < 1e-9, "{:?}", out[1]);
        assert!((out[1].fine - 3.0).abs() < 1e-9, "{:?}", out[1]);
        // il tempo e' stimato: la confidenza lo dichiara
        assert_eq!(out[1].confidenza, 0.0);
    }

    #[test]
    fn ripulisci_spartisce_equamente_piu_parole_consecutive() {
        let p = vec![
            parola("sono", 0.0, 1.0),
            senza_tempo("3"),
            senza_tempo("+"),
            senza_tempo("4"),
            parola("totale", 4.0, 5.0),
        ];
        let out = ripulisci(p, 10.0, DURATA_MINIMA_PAROLA);
        for (i, atteso) in [1.0, 2.0, 3.0].into_iter().enumerate() {
            assert!(
                (out[i + 1].inizio - atteso).abs() < 1e-9,
                "parola {i}: {:?}",
                out[i + 1]
            );
        }
        assert!((out[3].fine - 4.0).abs() < 1e-9);
    }

    #[test]
    fn ripulisci_ancora_agli_estremi_del_file() {
        let p = vec![senza_tempo("1"), parola("euro", 2.0, 3.0), senza_tempo("2")];
        let out = ripulisci(p, 5.0, DURATA_MINIMA_PAROLA);
        // in testa l'ancora sinistra e' 0
        assert!((out[0].inizio - 0.0).abs() < 1e-9);
        assert!((out[0].fine - 2.0).abs() < 1e-9);
        // in coda l'ancora destra e' la durata dell'audio
        assert!((out[2].inizio - 3.0).abs() < 1e-9);
        assert!((out[2].fine - 5.0).abs() < 1e-9);
    }

    #[test]
    fn ripulisci_gestisce_tutte_le_parole_senza_tempo() {
        let p = vec![senza_tempo("uno"), senza_tempo("due"), senza_tempo("tre")];
        let out = ripulisci(p, 3.0, DURATA_MINIMA_PAROLA);
        assert!(out.iter().all(|w| w.inizio.is_finite() && w.fine.is_finite()));
        assert!((out[0].inizio - 0.0).abs() < 1e-9);
        assert!((out[1].inizio - 1.0).abs() < 1e-9);
        assert!((out[2].fine - 3.0).abs() < 1e-9);
    }

    #[test]
    fn ripulisci_impone_la_monotonia() {
        let p = vec![parola("a", 0.0, 1.0), parola("b", 0.5, 1.5), parola("c", 0.2, 2.0)];
        let out = ripulisci(p, 10.0, DURATA_MINIMA_PAROLA);
        for pair in out.windows(2) {
            assert!(pair[1].inizio >= pair[0].fine, "{pair:?}");
        }
    }

    #[test]
    fn ripulisci_impone_la_durata_minima() {
        let p = vec![parola("a", 1.0, 1.0), parola("b", 2.0, 2.001)];
        let out = ripulisci(p, 10.0, DURATA_MINIMA_PAROLA);
        assert!(out.iter().all(|w| w.fine - w.inizio >= DURATA_MINIMA_PAROLA - 1e-9), "{out:?}");
    }

    #[test]
    fn ripulisci_tronca_alla_durata_dellaudio() {
        let p = vec![parola("a", 4.0, 12.0), parola("b", 20.0, 30.0)];
        let out = ripulisci(p, 5.0, DURATA_MINIMA_PAROLA);
        assert!(out.iter().all(|w| w.fine <= 5.0 + 1e-9), "{out:?}");
        assert!(out.iter().all(|w| w.inizio <= w.fine), "{out:?}");
    }

    #[test]
    fn ripulisci_lascia_invariata_una_sequenza_gia_pulita() {
        let p = vec![parola("uno", 0.0, 0.5), parola("due", 0.6, 1.2), parola("tre", 1.2, 2.0)];
        let out = ripulisci(p.clone(), 10.0, DURATA_MINIMA_PAROLA);
        assert_eq!(out.len(), p.len());
        for (a, b) in out.iter().zip(p.iter()) {
            assert_eq!(a.testo, b.testo);
            assert!((a.inizio - b.inizio).abs() < 1e-9 && (a.fine - b.fine).abs() < 1e-9, "{a:?}");
        }
    }

    #[test]
    fn ripulisci_e_idempotente() {
        let p = vec![
            parola("a", 0.0, 1.0),
            senza_tempo("7"),
            parola("b", 0.5, 0.5),
            parola("", 9.0, 9.0),
        ];
        let una = ripulisci(p, 4.0, DURATA_MINIMA_PAROLA);
        let due = ripulisci(una.clone(), 4.0, DURATA_MINIMA_PAROLA);
        assert_eq!(una.len(), due.len());
        for (a, b) in una.iter().zip(due.iter()) {
            assert!((a.inizio - b.inizio).abs() < 1e-9 && (a.fine - b.fine).abs() < 1e-9, "{a:?} {b:?}");
        }
    }

    #[test]
    fn ripulisci_senza_durata_nota_non_tronca() {
        let p = vec![parola("a", 0.0, 1.0), parola("b", 100.0, 200.0)];
        let out = ripulisci(p, 0.0, DURATA_MINIMA_PAROLA);
        assert!((out[1].fine - 200.0).abs() < 1e-9, "{out:?}");
    }
}
