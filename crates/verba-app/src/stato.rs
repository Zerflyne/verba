//! Lo stato condiviso della finestra.
//!
//! Un solo lavoro alla volta, protetto da un mutex, piu' le impostazioni e
//! l'interruttore che ferma l'operazione in corso. L'interruttore sta fuori
//! dal mutex apposta: `Annulla` deve poter arrivare **mentre** la
//! trascrizione tiene il lock, altrimenti il pulsante non farebbe niente
//! finche' l'operazione non e' finita da sola.

use std::sync::{Arc, Mutex};

use verba_core::caratteri::{self, Catalogo};
use verba_core::eventi::Interruttore;
use verba_core::impostazioni::Impostazioni;
use verba_core::sessione::Sessione;

pub struct Stato {
    pub sessione: Mutex<Option<Sessione>>,
    pub impostazioni: Mutex<Impostazioni>,
    /// Vero mentre una trascrizione o un export sono in corso.
    pub occupato: Arc<std::sync::atomic::AtomicBool>,
    pub interruttore: Interruttore,
}

impl Stato {
    pub fn nuovo() -> Self {
        Self {
            sessione: Mutex::new(None),
            impostazioni: Mutex::new(Impostazioni::carica()),
            occupato: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            interruttore: Interruttore::nuovo(),
        }
    }

    /// Il catalogo dei caratteri: quelli di serie, quelli aggiunti a mano e —
    /// se lo si e' chiesto — quelli installati sul sistema.
    pub fn catalogo(&self) -> Catalogo {
        let i = self.impostazioni.lock().unwrap();
        let mut cat = Catalogo::nuovo(&caratteri::cartelle_predefinite(), i.caratteri_di_sistema);
        for file in &i.caratteri_aggiunti {
            if let Err(e) = cat.aggiungi_file(file) {
                tracing::warn!(
                    file = %file.display(),
                    errore = %e,
                    "carattere aggiunto non piu' leggibile: lo salto"
                );
            }
        }
        cat
    }
}
