//! Il catalogo dei modelli, e come arrivano sulla macchina.
//!
//! I modelli **non si impacchettano**: Whisper large-v3 da solo supera i tre
//! gigabyte, e un `.deb` o un `.exe` di quella dimensione non e'
//! distribuibile. Si scaricano una volta e vivono in [`crate::cartelle::modelli`].
//!
//! Di ogni file scaricato si verifica l'impronta SHA-256, e uno scaricamento
//! interrotto **riprende** invece di ricominciare: su tre gigabyte la
//! differenza fra le due cose e' fra un'attesa e una rinuncia.
//!
//! Non tutto si puo' scaricare. L'allineatore wav2vec2 non esiste in ONNX su
//! nessun repository pubblico di cui ci si possa fidare, e va esportato in
//! locale: il catalogo lo dice invece di far finta di niente — vedi
//! [`Provenienza`].

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::cartelle;
use crate::eventi::{Fase, Progresso};

/// Quanto grande e' il modello di trascrizione.
///
/// La differenza non e' un dettaglio da smanettoni: e' spazio su disco,
/// memoria richiesta e tempo di attesa. Ognuna delle tre e' scritta in
/// [`Dimensione::compromesso`], perche' non la si indovini.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Dimensione {
    Small,
    Medium,
    #[default]
    LargeV3,
}

impl Dimensione {
    pub const TUTTE: [Dimensione; 3] = [Dimensione::LargeV3, Dimensione::Medium, Dimensione::Small];

    /// Come si scrive nell'interfaccia e sulla riga di comando.
    pub fn nome(self) -> &'static str {
        match self {
            Dimensione::Small => "small",
            Dimensione::Medium => "medium",
            Dimensione::LargeV3 => "large-v3",
        }
    }

    /// Il file GGML corrispondente.
    pub fn file(self) -> &'static str {
        match self {
            Dimensione::Small => "ggml-small.bin",
            Dimensione::Medium => "ggml-medium.bin",
            Dimensione::LargeV3 => "ggml-large-v3.bin",
        }
    }

    /// La riga che sta sotto il selettore: spazio, memoria, velocita'.
    pub fn compromesso(self) -> &'static str {
        match self {
            Dimensione::LargeV3 => {
                "2,9 GB su disco, ~4,3 GB di memoria. La qualita' di riferimento; \
                 su CPU e' lento."
            }
            Dimensione::Medium => {
                "1,4 GB su disco, ~2,2 GB di memoria. Circa due volte piu' veloce, \
                 qualche nome proprio in meno."
            }
            Dimensione::Small => {
                "465 MB su disco, ~1 GB di memoria. Quattro-cinque volte piu' veloce; \
                 va bene per una bozza o per una macchina senza GPU."
            }
        }
    }

    /// La dimensione desumibile dal nome del file GGML.
    ///
    /// Serve a chi ha in mano solo il percorso — la pipeline, per stimare la
    /// memoria — e non vuole un secondo campo che possa andare fuori sincrono
    /// con il percorso stesso. Un nome che non si riconosce vale `LargeV3`:
    /// e' la stima piu' alta, e sbagliare per eccesso qui costa una staffetta
    /// in piu', non un errore di memoria esaurita.
    pub fn dal_file(percorso: &std::path::Path) -> Self {
        let nome = percorso.file_name().and_then(|n| n.to_str()).unwrap_or("");
        Self::TUTTE.into_iter().find(|d| d.file() == nome).unwrap_or(Dimensione::LargeV3)
    }

    pub fn da_nome(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "small" => Some(Dimensione::Small),
            "medium" => Some(Dimensione::Medium),
            "large-v3" | "large" | "largev3" => Some(Dimensione::LargeV3),
            _ => None,
        }
    }
}

/// A cosa serve un file del catalogo.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Ruolo {
    /// Whisper, in una delle tre dimensioni.
    Trascrizione(Dimensione),
    /// pyannote: dove c'e' parlato e dove no.
    Segmentazione,
    /// wav2vec2: i tempi di ogni singola parola.
    Allineamento,
    /// Il vocabolario del tokenizer dell'allineatore.
    Vocabolario,
}

/// Da dove arriva un file.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Provenienza {
    /// Si scarica, e l'impronta e' nota.
    Scaricabile {
        url: &'static str,
        sha256: &'static str,
    },
    /// Va prodotto sulla macchina di chi lo usa, con quel comando.
    ///
    /// Non e' una scorciatoia: di questo file non esiste una copia pubblica
    /// verificabile, e scaricarlo dal primo repository che capita sarebbe
    /// peggio che chiedere di generarlo.
    DaEsportare { comando: &'static str },
}

impl Provenienza {
    pub fn si_scarica(self) -> bool {
        matches!(self, Provenienza::Scaricabile { .. })
    }
}

/// Un file del catalogo.
#[derive(Copy, Clone, Debug)]
pub struct Modello {
    /// Nome breve, quello che si scrive sulla riga di comando.
    pub id: &'static str,
    /// Come si chiama per esteso.
    pub nome: &'static str,
    /// Il nome del file su disco.
    pub file: &'static str,
    pub ruolo: Ruolo,
    /// Dimensione attesa in byte: serve per la barra di avanzamento prima
    /// ancora che il server risponda.
    pub byte: u64,
    pub provenienza: Provenienza,
    /// Una riga su cosa fa e da dove viene.
    pub spiegazione: &'static str,
}

impl Modello {
    /// Dove sta, o dove starebbe.
    pub fn percorso(&self, cartella: &Path) -> PathBuf {
        cartella.join(self.file)
    }

    /// Il file temporaneo di uno scaricamento in corso.
    pub fn parziale(&self, cartella: &Path) -> PathBuf {
        cartella.join(format!("{}.parziale", self.file))
    }
}

/// Le impronte vengono dall'API di Hugging Face (`lfs.sha256`), non sono
/// state calcolate a mano: e' la stessa impronta che il repository dichiara.
pub const CATALOGO: [Modello; 5] = [
    Modello {
        id: "large-v3",
        nome: "Whisper large-v3",
        file: "ggml-large-v3.bin",
        ruolo: Ruolo::Trascrizione(Dimensione::LargeV3),
        byte: 3_095_033_483,
        provenienza: Provenienza::Scaricabile {
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin",
            sha256: "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        },
        spiegazione: "Il modello di trascrizione. Formato GGML per whisper.cpp.",
    },
    Modello {
        id: "medium",
        nome: "Whisper medium",
        file: "ggml-medium.bin",
        ruolo: Ruolo::Trascrizione(Dimensione::Medium),
        byte: 1_533_763_059,
        provenienza: Provenienza::Scaricabile {
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
            sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        },
        spiegazione: "Trascrizione a meta' strada fra qualita' e velocita'.",
    },
    Modello {
        id: "small",
        nome: "Whisper small",
        file: "ggml-small.bin",
        ruolo: Ruolo::Trascrizione(Dimensione::Small),
        byte: 487_601_967,
        provenienza: Provenienza::Scaricabile {
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
            sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        },
        spiegazione: "Trascrizione rapida, per una bozza o per una macchina modesta.",
    },
    Modello {
        id: "segmentazione",
        nome: "pyannote segmentation 3.0",
        file: "pyannote-segmentation-3.0.onnx",
        ruolo: Ruolo::Segmentazione,
        byte: 5_986_908,
        provenienza: Provenienza::Scaricabile {
            // L'esportazione ONNX pubblica di onnx-community: a differenza del
            // repository originale non e' *gated*, quindi non servono ne' un
            // account ne' un token per il primo avvio.
            url: "https://huggingface.co/onnx-community/pyannote-segmentation-3.0/resolve/main/onnx/model.onnx",
            sha256: "057ee564753071c0b09b5b611648b50ac188d50846bff5f01e9f7bbf1591ea25",
        },
        spiegazione: "Trova dove c'e' parlato: e' quello che divide l'audio in segmenti.",
    },
    Modello {
        id: "vocabolario",
        nome: "Vocabolario wav2vec2-italian",
        file: "wav2vec2-italian.vocab.json",
        ruolo: Ruolo::Vocabolario,
        byte: 410,
        provenienza: Provenienza::Scaricabile {
            url: "https://huggingface.co/jonatasgrosman/wav2vec2-large-xlsr-53-italian/resolve/main/vocab.json",
            sha256: "",
        },
        spiegazione: "Da qui il programma deduce blank CTC, delimitatore di parola e maiuscole.",
    },
];

/// L'allineatore, l'unico file che non si scarica.
pub const ALLINEATORE: Modello = Modello {
    id: "allineamento",
    nome: "wav2vec2-italian (CTC)",
    file: "wav2vec2-italian.onnx",
    ruolo: Ruolo::Allineamento,
    byte: 1_262_346_310,
    provenienza: Provenienza::DaEsportare {
        comando: "python scripts/export_models.py --w2v",
    },
    spiegazione: "Da' il tempo esatto di ogni parola. Va esportato: non esiste una \
                  versione ONNX pubblica di cui fidarsi.",
};

/// Il modello di trascrizione della dimensione chiesta.
pub fn whisper(d: Dimensione) -> &'static Modello {
    CATALOGO
        .iter()
        .find(|m| m.ruolo == Ruolo::Trascrizione(d))
        .expect("il catalogo contiene tutte e tre le dimensioni")
}

/// Un modello per identificativo.
pub fn per_id(id: &str) -> Option<&'static Modello> {
    if id == ALLINEATORE.id {
        return Some(&ALLINEATORE);
    }
    CATALOGO.iter().find(|m| m.id == id)
}

/// I quattro file che servono per lavorare con quella dimensione.
pub fn necessari(d: Dimensione) -> Vec<&'static Modello> {
    let mut v: Vec<&'static Modello> = vec![whisper(d)];
    v.extend(CATALOGO.iter().filter(|m| matches!(m.ruolo, Ruolo::Segmentazione | Ruolo::Vocabolario)));
    v.push(&ALLINEATORE);
    v
}

/// Come sta un modello sulla macchina.
#[derive(Clone, Debug)]
pub struct Stato {
    pub modello: &'static Modello,
    pub percorso: PathBuf,
    pub presente: bool,
    /// Byte gia' scaricati di un file interrotto, se ce ne sono.
    pub ripresa: u64,
    pub byte_su_disco: u64,
}

impl Stato {
    fn nuovo(modello: &'static Modello, cartella: &Path) -> Self {
        let percorso = modello.percorso(cartella);
        let byte_su_disco = fs::metadata(&percorso).map(|m| m.len()).unwrap_or(0);
        let ripresa = fs::metadata(modello.parziale(cartella)).map(|m| m.len()).unwrap_or(0);
        Self { modello, percorso, presente: byte_su_disco > 0, ripresa, byte_su_disco }
    }
}

/// Lo stato dei modelli necessari a quella dimensione.
pub fn stato(cartella: &Path, d: Dimensione) -> Vec<Stato> {
    necessari(d).into_iter().map(|m| Stato::nuovo(m, cartella)).collect()
}

/// Lo stato di tutto il catalogo, comprese le dimensioni non in uso.
pub fn stato_completo(cartella: &Path) -> Vec<Stato> {
    CATALOGO
        .iter()
        .chain(std::iter::once(&ALLINEATORE))
        .map(|m| Stato::nuovo(m, cartella))
        .collect()
}

/// Quello che manca per poter lavorare.
pub fn mancanti(cartella: &Path, d: Dimensione) -> Vec<&'static Modello> {
    stato(cartella, d).into_iter().filter(|s| !s.presente).map(|s| s.modello).collect()
}

/// Scarica tutti i modelli mancanti per quella dimensione.
///
/// Ritorna i file che non si possono scaricare e che restano da produrre a
/// mano: l'esito e' «cosa manca ancora», non un errore, perche' scaricare il
/// resto e' comunque un progresso.
pub fn scarica_mancanti(
    cartella: &Path,
    d: Dimensione,
    progresso: &Progresso,
) -> Result<Vec<&'static Modello>> {
    let da_fare: Vec<&'static Modello> = mancanti(cartella, d);
    let (scaricabili, a_mano): (Vec<&'static Modello>, Vec<&'static Modello>) =
        da_fare.into_iter().partition(|m| m.provenienza.si_scarica());

    if !scaricabili.is_empty() {
        let totale: u64 = scaricabili.iter().map(|m| m.byte).sum();
        info!(
            file = scaricabili.len(),
            totale = %cartelle::dimensione_leggibile(totale),
            cartella = %cartella.display(),
            "scaricamento dei modelli"
        );
        let _c = progresso.inizia(Fase::Scaricamento);
        let mut fatti: u64 = 0;
        for m in &scaricabili {
            scarica_uno(m, cartella, progresso, fatti, totale)?;
            fatti += m.byte;
        }
    }
    Ok(a_mano)
}

/// Scarica un modello, riprendendo se era stato interrotto.
pub fn scarica(m: &'static Modello, cartella: &Path, progresso: &Progresso) -> Result<PathBuf> {
    let _c = progresso.inizia(Fase::Scaricamento);
    scarica_uno(m, cartella, progresso, 0, m.byte)
}

/// Il corpo dello scaricamento.
///
/// `gia_fatti` e `totale` servono a far avanzare una barra sola quando i file
/// sono piu' d'uno: chi guarda vuole sapere quanto manca in tutto, non a che
/// punto e' il terzo file di quattro.
fn scarica_uno(
    m: &Modello,
    cartella: &Path,
    progresso: &Progresso,
    gia_fatti: u64,
    totale: u64,
) -> Result<PathBuf> {
    let Provenienza::Scaricabile { url, sha256 } = m.provenienza else {
        bail!("{} non si scarica: {}", m.nome, istruzioni_a_mano(m));
    };

    fs::create_dir_all(cartella)
        .with_context(|| format!("creazione di {}", cartella.display()))?;
    let finale = m.percorso(cartella);
    if finale.exists() {
        return Ok(finale);
    }
    let parziale = m.parziale(cartella);

    // Quanto c'e' gia': e' il byte da cui chiedere al server di ripartire.
    let da = fs::metadata(&parziale).map(|x| x.len()).unwrap_or(0);
    if da > 0 {
        info!(
            file = m.file,
            ripresa = %cartelle::dimensione_leggibile(da),
            "scaricamento ripreso"
        );
    }

    let mut richiesta = ureq::get(url);
    if da > 0 {
        richiesta = richiesta.set("Range", &format!("bytes={da}-"));
    }
    let risposta = richiesta
        .call()
        .with_context(|| format!("scaricamento di {} da {url}", m.nome))?;

    // 206 = il server ha accettato la ripresa; 200 = manda tutto da capo, e
    // allora il pezzo che avevamo non serve piu'.
    let riprende = risposta.status() == 206;
    if da > 0 && !riprende {
        warn!(file = m.file, "il server non riprende: si ricomincia da capo");
    }
    let inizio = if riprende { da } else { 0 };

    let lunghezza: u64 = risposta
        .header("content-length")
        .and_then(|v| v.parse::<u64>().ok())
        .map(|l| l + inizio)
        .unwrap_or(m.byte.max(inizio));

    let mut uscita = if riprende {
        let mut f = OpenOptions::new()
            .append(true)
            .open(&parziale)
            .with_context(|| format!("apertura di {}", parziale.display()))?;
        f.seek(SeekFrom::End(0))?;
        f
    } else {
        File::create(&parziale).with_context(|| format!("creazione di {}", parziale.display()))?
    };

    let mut sorgente = risposta.into_reader();
    let mut buffer = vec![0u8; 1 << 20];
    let mut scritti = inizio;
    let mut ultima_frazione = -1.0f32;

    loop {
        // Un annullamento lascia il `.parziale` dov'e': la prossima volta
        // riparte da li'.
        if let Err(e) = progresso.verifica() {
            uscita.flush().ok();
            return Err(e.into());
        }
        let n = sorgente.read(&mut buffer).with_context(|| format!("lettura di {url}"))?;
        if n == 0 {
            break;
        }
        uscita.write_all(&buffer[..n]).with_context(|| format!("scrittura di {}", parziale.display()))?;
        scritti += n as u64;

        let frazione = if totale > 0 {
            (gia_fatti + scritti.min(m.byte)) as f32 / totale as f32
        } else {
            scritti as f32 / lunghezza.max(1) as f32
        };
        if frazione - ultima_frazione >= 0.002 {
            ultima_frazione = frazione;
            progresso.passo(Fase::Scaricamento, frazione.clamp(0.0, 1.0));
        }
    }
    uscita.flush()?;
    drop(uscita);

    // La verifica dell'impronta e' il motivo per cui il file passa da
    // `.parziale`: un file troncato o corrotto non deve mai comparire col
    // nome definitivo, o al giro dopo verrebbe preso per buono.
    if !sha256.is_empty() {
        let ottenuto = impronta(&parziale)?;
        if !ottenuto.eq_ignore_ascii_case(sha256) {
            fs::remove_file(&parziale).ok();
            bail!(
                "{} e' arrivato danneggiato: l'impronta e' {ottenuto} invece di {sha256}. \
                 Il file scaricato e' stato cancellato; riprova.",
                m.nome
            );
        }
    }

    fs::rename(&parziale, &finale)
        .with_context(|| format!("spostamento in {}", finale.display()))?;
    info!(
        file = %finale.display(),
        dimensione = %cartelle::dimensione_leggibile(scritti),
        "modello scaricato"
    );
    Ok(finale)
}

/// L'impronta SHA-256 di un file.
pub fn impronta(percorso: &Path) -> Result<String> {
    let mut f = File::open(percorso)
        .with_context(|| format!("apertura di {}", percorso.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Vero se il file su disco corrisponde all'impronta dichiarata.
///
/// Un modello senza impronta nota (il vocabolario, che non e' un file LFS e
/// quindi non ne ha una pubblicata) risulta sempre buono: meglio dirlo cosi'
/// che inventarsi una verifica che non c'e'.
pub fn verifica(m: &Modello, cartella: &Path) -> Result<bool> {
    let Provenienza::Scaricabile { sha256, .. } = m.provenienza else {
        return Ok(true);
    };
    if sha256.is_empty() {
        return Ok(true);
    }
    let percorso = m.percorso(cartella);
    if !percorso.is_file() {
        return Ok(false);
    }
    Ok(impronta(&percorso)?.eq_ignore_ascii_case(sha256))
}

/// Cancella un modello dal disco.
pub fn rimuovi(m: &Modello, cartella: &Path) -> Result<bool> {
    let percorso = m.percorso(cartella);
    if !percorso.exists() {
        return Ok(false);
    }
    fs::remove_file(&percorso)
        .with_context(|| format!("rimozione di {}", percorso.display()))?;
    info!(file = %percorso.display(), "modello rimosso");
    Ok(true)
}

/// Cosa dire di un modello che non si scarica.
pub fn istruzioni_a_mano(m: &Modello) -> String {
    match m.provenienza {
        Provenienza::DaEsportare { comando } => format!(
            "Va esportato sulla tua macchina, una volta sola:\n    {comando}\n\
             Il file prodotto va poi messo in {}, col nome {}.",
            cartelle::modelli().display(),
            m.file
        ),
        Provenienza::Scaricabile { url, .. } => format!("si scarica da {url}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn il_catalogo_ha_le_tre_dimensioni_e_nessun_doppione() {
        for d in Dimensione::TUTTE {
            let m = whisper(d);
            assert_eq!(m.file, d.file());
            assert!(m.provenienza.si_scarica());
        }
        let mut id: Vec<&str> = CATALOGO.iter().map(|m| m.id).collect();
        id.push(ALLINEATORE.id);
        let quanti = id.len();
        id.sort_unstable();
        id.dedup();
        assert_eq!(id.len(), quanti, "due modelli con lo stesso identificativo");
    }

    #[test]
    fn per_lavorare_servono_quattro_file() {
        let n = necessari(Dimensione::LargeV3);
        assert_eq!(n.len(), 4);
        assert!(n.iter().any(|m| m.ruolo == Ruolo::Trascrizione(Dimensione::LargeV3)));
        assert!(n.iter().any(|m| m.ruolo == Ruolo::Segmentazione));
        assert!(n.iter().any(|m| m.ruolo == Ruolo::Allineamento));
        assert!(n.iter().any(|m| m.ruolo == Ruolo::Vocabolario));
        // Cambiando dimensione cambia un file solo.
        let s = necessari(Dimensione::Small);
        assert_eq!(s.len(), 4);
        assert_eq!(s[0].file, "ggml-small.bin");
    }

    #[test]
    fn le_impronte_dichiarate_sono_sha256_plausibili() {
        for m in CATALOGO {
            if let Provenienza::Scaricabile { sha256, url } = m.provenienza {
                assert!(url.starts_with("https://"), "{}: URL non cifrato", m.id);
                if !sha256.is_empty() {
                    assert_eq!(sha256.len(), 64, "{}: impronta di lunghezza sbagliata", m.id);
                    assert!(
                        sha256.chars().all(|c| c.is_ascii_hexdigit()),
                        "{}: impronta non esadecimale",
                        m.id
                    );
                }
            }
        }
    }

    #[test]
    fn l_allineatore_dice_come_si_produce() {
        assert!(!ALLINEATORE.provenienza.si_scarica());
        let testo = istruzioni_a_mano(&ALLINEATORE);
        assert!(testo.contains("export_models.py"), "{testo}");
    }

    #[test]
    fn i_nomi_delle_dimensioni_si_leggono_e_si_riscrivono() {
        for d in Dimensione::TUTTE {
            assert_eq!(Dimensione::da_nome(d.nome()), Some(d));
        }
        assert_eq!(Dimensione::da_nome("LARGE"), Some(Dimensione::LargeV3));
        assert_eq!(Dimensione::da_nome("gigante"), None);
    }

    #[test]
    fn su_una_macchina_vuota_manca_tutto() {
        let vuota = std::env::temp_dir().join("verba-modelli-inesistente-per-i-test");
        assert_eq!(mancanti(&vuota, Dimensione::LargeV3).len(), 4);
        let s = stato(&vuota, Dimensione::LargeV3);
        assert!(s.iter().all(|x| !x.presente && x.ripresa == 0));
    }

    #[test]
    fn l_impronta_e_quella_che_dice_sha256sum() {
        let f = std::env::temp_dir().join("verba-impronta.txt");
        std::fs::write(&f, b"verba").unwrap();
        // echo -n verba | sha256sum
        assert_eq!(
            impronta(&f).unwrap(),
            "db15cdcaefb22a2e607d2e29c1f3a07723eb662c59edf349d99d4b1f7d4d0075"
        );
        std::fs::remove_file(&f).ok();
    }
}
