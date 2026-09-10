//! I caratteri disponibili: quelli di serie, quelli del sistema, quelli che
//! l'utente aggiunge.
//!
//! Il motore non scarica nulla: i caratteri di serie stanno in `assets/fonts`
//! accanto all'applicazione, e chi ne vuole un altro scarica il `.ttf` e lo
//! mette nella stessa cartella (o lo indica per percorso). E' la stessa scelta
//! fatta per i modelli, per la ragione opposta: i modelli sono troppo grandi
//! per essere impacchettati, i caratteri sono abbastanza piccoli.
//!
//! **Uno solo e' incorporato nel binario**: Inter peso 700. E' la ricaduta che
//! non puo' mai mancare, cosi' un'installazione senza `assets/fonts` e senza
//! caratteri di sistema produce comunque un risultato invece di un errore.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cosmic_text::fontdb;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Il peso predefinito: la spec chiede 700 per i sottotitoli.
pub const PESO_PREDEFINITO: u16 = 700;

/// La famiglia predefinita.
pub const FAMIGLIA_PREDEFINITA: &str = "Inter";

/// Un carattere richiesto: famiglia e peso, oppure un file preciso.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Richiesta {
    /// Nome della famiglia, come compare nell'elenco.
    pub famiglia: String,
    /// Peso desiderato, da 100 a 900.
    pub peso: u16,
    /// Un file `.ttf` o `.otf` preciso, che ha la precedenza sulla famiglia.
    /// Serve a chi ha un carattere suo e non vuole installarlo.
    pub file: Option<PathBuf>,
}

impl Default for Richiesta {
    fn default() -> Self {
        Self {
            famiglia: FAMIGLIA_PREDEFINITA.to_string(),
            peso: PESO_PREDEFINITO,
            file: None,
        }
    }
}

/// Una famiglia disponibile, con i pesi che ha davvero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Famiglia {
    pub nome: String,
    /// Pesi presenti, in ordine crescente e senza ripetizioni.
    pub pesi: Vec<u16>,
    /// Vero se viene dalla cartella dei caratteri e non dal sistema: sono
    /// quelle che si possono mostrare per prime, perche' ci sono di sicuro
    /// su ogni installazione.
    pub di_serie: bool,
}

impl Famiglia {
    /// Il peso disponibile piu' vicino a quello chiesto.
    ///
    /// A parita' di distanza vince il piu' pesante: fra 400 e 800, per un 600
    /// richiesto, un sottotitolo sta meglio in 800.
    pub fn peso_piu_vicino(&self, chiesto: u16) -> Option<u16> {
        self.pesi
            .iter()
            .copied()
            .min_by_key(|&p| (p.abs_diff(chiesto), u16::MAX - p))
    }
}

/// Cosa e' successo alla richiesta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Esito {
    /// Trovato esattamente cio' che era stato chiesto.
    Esatto { famiglia: String, peso: u16 },
    /// La famiglia c'era ma non in quel peso.
    PesoSostituito { famiglia: String, chiesto: u16, usato: u16 },
    /// La famiglia non c'era.
    FamigliaSostituita { chiesto: String, famiglia: String, peso: u16 },
}

impl Esito {
    pub fn famiglia(&self) -> &str {
        match self {
            Esito::Esatto { famiglia, .. }
            | Esito::PesoSostituito { famiglia, .. }
            | Esito::FamigliaSostituita { famiglia, .. } => famiglia,
        }
    }

    pub fn peso(&self) -> u16 {
        match self {
            Esito::Esatto { peso, .. }
            | Esito::FamigliaSostituita { peso, .. } => *peso,
            Esito::PesoSostituito { usato, .. } => *usato,
        }
    }

    /// Vero se e' stato usato qualcosa di diverso da quanto chiesto: e' il caso
    /// in cui l'interfaccia deve dirlo, invece di lasciarlo scoprire guardando
    /// il risultato.
    pub fn e_una_ricaduta(&self) -> bool {
        !matches!(self, Esito::Esatto { .. })
    }

    /// Il messaggio da mostrare, se c'e' qualcosa da dire.
    pub fn avviso(&self) -> Option<String> {
        match self {
            Esito::Esatto { .. } => None,
            Esito::PesoSostituito { famiglia, chiesto, usato } => Some(format!(
                "Il peso {chiesto} non e' disponibile per {famiglia}. E' stato usato il {usato}."
            )),
            Esito::FamigliaSostituita { chiesto, famiglia, .. } => Some(format!(
                "Il carattere «{chiesto}» non e' disponibile. Ne e' stato usato un altro: {famiglia}."
            )),
        }
    }
}

/// Il catalogo dei caratteri utilizzabili.
pub struct Catalogo {
    db: fontdb::Database,
    famiglie: Vec<Famiglia>,
}

impl Catalogo {
    /// Costruisce il catalogo.
    ///
    /// L'ordine conta: prima l'Inter incorporato, poi i caratteri di serie
    /// della cartella, poi — se richiesto — quelli installati sul sistema. Chi
    /// arriva dopo non toglie il posto a chi c'era gia'.
    pub fn nuovo(cartelle: &[PathBuf], con_font_di_sistema: bool) -> Self {
        let mut db = fontdb::Database::new();
        db.load_font_data(crate::FONT_INTER_BOLD.to_vec());

        for cartella in cartelle {
            if !cartella.is_dir() {
                continue;
            }
            db.load_fonts_dir(cartella);
        }
        // Tutto cio' che c'e' fin qui e' "di serie": e' presente su ogni
        // installazione, e sono le sole famiglie che si possono suggerire
        // senza sapere cosa c'e' sulla macchina.
        let di_serie: Vec<String> = nomi_famiglie(&db);
        debug!(caratteri = db.len(), famiglie = di_serie.len(), "caratteri di serie caricati");

        if con_font_di_sistema {
            db.load_system_fonts();
            debug!(caratteri = db.len(), "aggiunti i caratteri di sistema");
        }

        let famiglie = elenca(&db, &di_serie);
        Self { db, famiglie }
    }

    /// Solo l'Inter incorporato: il catalogo minimo che esiste sempre.
    pub fn incorporato() -> Self {
        Self::nuovo(&[], false)
    }

    /// Le famiglie disponibili, quelle di serie per prime e poi in ordine
    /// alfabetico.
    pub fn famiglie(&self) -> &[Famiglia] {
        &self.famiglie
    }

    pub fn famiglia(&self, nome: &str) -> Option<&Famiglia> {
        self.famiglie.iter().find(|f| f.nome.eq_ignore_ascii_case(nome))
    }

    /// Aggiunge un carattere da un file. Ritorna il nome della famiglia.
    ///
    /// E' la via per chi ha scaricato un `.ttf` e non vuole installarlo:
    /// il file viene letto e basta, non copiato ne' registrato altrove.
    pub fn aggiungi_file(&mut self, percorso: &Path) -> Result<String> {
        let dati = std::fs::read(percorso)
            .with_context(|| format!("lettura del carattere {}", percorso.display()))?;
        let prima = self.db.len();
        self.db.load_font_data(dati);
        if self.db.len() == prima {
            anyhow::bail!(
                "{}: il file non contiene alcun carattere utilizzabile",
                percorso.display()
            );
        }
        // La famiglia del volto appena aggiunto: e' l'ultimo del database.
        let nome = self
            .db
            .faces()
            .last()
            .and_then(|f| f.families.first().map(|(n, _)| n.clone()))
            .context("il carattere non dichiara un nome di famiglia")?;

        let di_serie: Vec<String> =
            self.famiglie.iter().filter(|f| f.di_serie).map(|f| f.nome.clone()).collect();
        self.famiglie = elenca(&self.db, &di_serie);
        Ok(nome)
    }

    /// Risolve una richiesta contro cio' che c'e' davvero.
    ///
    /// Non fallisce mai: al peggio ricade sulla famiglia predefinita, e lo
    /// dichiara nell'esito.
    pub fn risolvi(&mut self, richiesta: &Richiesta) -> Result<Esito> {
        let peso = richiesta.peso.clamp(100, 900);

        // Un file preciso ha la precedenza: chi lo indica sa cosa vuole.
        if let Some(percorso) = &richiesta.file {
            let famiglia = self.aggiungi_file(percorso)?;
            let usato = self
                .famiglia(&famiglia)
                .and_then(|f| f.peso_piu_vicino(peso))
                .unwrap_or(peso);
            return Ok(if usato == peso {
                Esito::Esatto { famiglia, peso }
            } else {
                Esito::PesoSostituito { famiglia, chiesto: peso, usato }
            });
        }

        match self.famiglia(&richiesta.famiglia) {
            Some(f) => {
                let nome = f.nome.clone();
                let usato = f.peso_piu_vicino(peso).unwrap_or(peso);
                Ok(if usato == peso {
                    Esito::Esatto { famiglia: nome, peso }
                } else {
                    Esito::PesoSostituito { famiglia: nome, chiesto: peso, usato }
                })
            }
            None => {
                // La ricaduta e' la famiglia predefinita, che c'e' sempre
                // perche' e' incorporata nel binario.
                let ripiego = self
                    .famiglia(FAMIGLIA_PREDEFINITA)
                    .or_else(|| self.famiglie.first())
                    .context("nessun carattere disponibile")?;
                Ok(Esito::FamigliaSostituita {
                    chiesto: richiesta.famiglia.clone(),
                    famiglia: ripiego.nome.clone(),
                    peso: ripiego.peso_piu_vicino(peso).unwrap_or(peso),
                })
            }
        }
    }

    /// Il database, per chi deve comporre il testo.
    pub fn database(&self) -> &fontdb::Database {
        &self.db
    }

    /// Consuma il catalogo e restituisce il database.
    pub fn in_database(self) -> fontdb::Database {
        self.db
    }
}

/// Le cartelle in cui cercare i caratteri di serie, in ordine di preferenza.
///
/// Prima accanto all'eseguibile (e' li' che stanno in un pacchetto installato),
/// poi nella cartella di lavoro (e' li' che stanno durante lo sviluppo).
pub fn cartelle_predefinite() -> Vec<PathBuf> {
    let mut fuori = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            fuori.push(dir.join("assets").join("fonts"));
            // In un pacchetto le risorse stanno accanto, non dentro `bin`.
            if let Some(su) = dir.parent() {
                fuori.push(su.join("share").join("verba").join("fonts"));
            }
        }
    }
    fuori.push(PathBuf::from("assets/fonts"));
    fuori
}

fn nomi_famiglie(db: &fontdb::Database) -> Vec<String> {
    let mut nomi: Vec<String> = db
        .faces()
        .filter_map(|f| f.families.first().map(|(n, _)| n.clone()))
        .collect();
    nomi.sort_unstable();
    nomi.dedup();
    nomi
}

/// Raccoglie le famiglie con i pesi che hanno davvero, scartando i corsivi:
/// per un sottotitolo il corsivo e' una scelta a parte, non un peso.
fn elenca(db: &fontdb::Database, di_serie: &[String]) -> Vec<Famiglia> {
    let mut mappa: std::collections::BTreeMap<String, Vec<u16>> = Default::default();
    for faccia in db.faces() {
        if faccia.style != fontdb::Style::Normal {
            continue;
        }
        let Some((nome, _)) = faccia.families.first() else { continue };
        mappa.entry(nome.clone()).or_default().push(faccia.weight.0);
    }

    let mut fuori: Vec<Famiglia> = mappa
        .into_iter()
        .map(|(nome, mut pesi)| {
            pesi.sort_unstable();
            pesi.dedup();
            let di_serie = di_serie.iter().any(|n| n == &nome);
            Famiglia { nome, pesi, di_serie }
        })
        .collect();

    // Quelle di serie per prime: ci sono su ogni installazione, e sono le sole
    // che si possono suggerire senza sapere cosa c'e' sulla macchina.
    fuori.sort_by(|a, b| b.di_serie.cmp(&a.di_serie).then_with(|| a.nome.cmp(&b.nome)));
    fuori
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogo_di_serie() -> Catalogo {
        Catalogo::nuovo(&[PathBuf::from("../../assets/fonts")], false)
    }

    #[test]
    fn il_catalogo_minimo_contiene_sempre_inter() {
        let c = Catalogo::incorporato();
        let inter = c.famiglia("Inter").expect("Inter e' incorporato nel binario");
        assert!(inter.pesi.contains(&700));
        assert!(inter.di_serie);
    }

    #[test]
    fn i_caratteri_di_serie_ci_sono_tutti() {
        let c = catalogo_di_serie();
        for atteso in ["Inter", "Montserrat", "Poppins", "Oswald", "Anton", "Bebas Neue"] {
            assert!(
                c.famiglia(atteso).is_some(),
                "manca {atteso}; presenti: {:?}",
                c.famiglie().iter().map(|f| &f.nome).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn i_pesi_dichiarati_sono_quelli_che_ci_sono() {
        let c = catalogo_di_serie();
        let m = c.famiglia("Montserrat").unwrap();
        assert_eq!(m.pesi, vec![400, 700, 900]);
        let anton = c.famiglia("Anton").unwrap();
        assert_eq!(anton.pesi, vec![400], "Anton ha un peso solo");
    }

    #[test]
    fn le_famiglie_di_serie_vengono_per_prime() {
        let c = catalogo_di_serie();
        let prime: Vec<bool> = c.famiglie().iter().map(|f| f.di_serie).collect();
        // Nessuna di serie dopo una che non lo e'.
        assert!(
            prime.windows(2).all(|w| w[0] || !w[1]),
            "l'ordine mescola le famiglie di serie con le altre"
        );
    }

    #[test]
    fn il_peso_chiesto_viene_trovato_quando_c_e() {
        let mut c = catalogo_di_serie();
        let esito = c
            .risolvi(&Richiesta { famiglia: "Poppins".into(), peso: 900, file: None })
            .unwrap();
        assert_eq!(esito, Esito::Esatto { famiglia: "Poppins".into(), peso: 900 });
        assert!(!esito.e_una_ricaduta());
        assert!(esito.avviso().is_none());
    }

    #[test]
    fn un_peso_assente_ricade_sul_piu_vicino_e_lo_dichiara() {
        let mut c = catalogo_di_serie();
        let esito = c
            .risolvi(&Richiesta { famiglia: "Anton".into(), peso: 700, file: None })
            .unwrap();
        assert_eq!(
            esito,
            Esito::PesoSostituito { famiglia: "Anton".into(), chiesto: 700, usato: 400 }
        );
        assert!(esito.avviso().unwrap().contains("Anton"));
    }

    #[test]
    fn a_parita_di_distanza_vince_il_peso_maggiore() {
        let f = Famiglia { nome: "prova".into(), pesi: vec![400, 800], di_serie: true };
        assert_eq!(f.peso_piu_vicino(600), Some(800));
    }

    #[test]
    fn una_famiglia_assente_ricade_su_quella_predefinita() {
        let mut c = catalogo_di_serie();
        let esito = c
            .risolvi(&Richiesta { famiglia: "Un Carattere Inventato".into(), peso: 700, file: None })
            .unwrap();
        assert!(matches!(esito, Esito::FamigliaSostituita { .. }));
        assert_eq!(esito.famiglia(), FAMIGLIA_PREDEFINITA);
        let avviso = esito.avviso().unwrap();
        assert!(avviso.contains("Un Carattere Inventato") && avviso.contains("Inter"));
    }

    #[test]
    fn un_file_aggiunto_a_mano_entra_nel_catalogo() {
        let mut c = Catalogo::incorporato();
        assert!(c.famiglia("Oswald").is_none(), "Oswald non e' incorporato");
        let nome = c.aggiungi_file(Path::new("../../assets/fonts/Oswald-Bold.ttf")).unwrap();
        assert_eq!(nome, "Oswald");
        assert!(c.famiglia("Oswald").unwrap().pesi.contains(&700));
    }

    #[test]
    fn un_file_che_non_e_un_carattere_da_errore_chiaro() {
        let mut c = Catalogo::incorporato();
        let errore = c.aggiungi_file(Path::new("Cargo.toml")).unwrap_err().to_string();
        assert!(errore.contains("non contiene alcun carattere"), "{errore}");
    }

    #[test]
    fn un_file_inesistente_da_errore_col_percorso() {
        let mut c = Catalogo::incorporato();
        let errore = c.aggiungi_file(Path::new("/non/esiste.ttf")).unwrap_err().to_string();
        assert!(errore.contains("/non/esiste.ttf"), "{errore}");
    }
}
