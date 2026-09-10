//! Il fotogramma al tempo `t`.
//!
//! Questo modulo esiste per una ragione sola, ed e' la piu' importante di tutta
//! la parte grafica: **l'anteprima e l'export devono uscire da qui**. Se
//! l'anteprima avesse un percorso di codice suo — piu' veloce, magari scritto
//! nel linguaggio dell'interfaccia — prima o poi divergerebbe dall'export su
//! qualche dettaglio, e si passerebbero giorni a cercare il perche'.
//!
//! Chi disegna chiede un tempo e riceve dei pixel. Nient'altro.

use crate::layout::{Blocco, LayoutConfig};
use crate::render::{Rasterizzatore, Tela};

/// Cosa e' visibile in un dato istante: quale blocco, e quale parola vi e'
/// indicata (`None` quando non ce n'e' nessuna: il blocco resta, la forma no).
pub type Stato = Option<(usize, Option<usize>)>;

/// I blocchi impaginati piu' tutto cio' che serve a disegnarli.
pub struct Scena {
    blocchi: Vec<Blocco>,
    rasterizzatore: Rasterizzatore,
    tela: Tela,
    /// Il blocco su cui il rasterizzatore e' preparato.
    preparato: Option<usize>,
    /// Lo stato gia' disegnato sulla tela.
    disegnato: Option<Stato>,
    /// Cursore sui blocchi: la ricerca per tempo va quasi sempre avanti.
    cursore: usize,
    disegni: u64,
}

impl Scena {
    pub fn nuova(blocchi: Vec<Blocco>, rasterizzatore: Rasterizzatore) -> Self {
        let cfg = rasterizzatore.configurazione();
        let tela = Tela::nuova(cfg.larghezza, cfg.altezza);
        Self {
            blocchi,
            rasterizzatore,
            tela,
            preparato: None,
            disegnato: None,
            cursore: 0,
            disegni: 0,
        }
    }

    pub fn blocchi(&self) -> &[Blocco] {
        &self.blocchi
    }

    pub fn configurazione(&self) -> &LayoutConfig {
        self.rasterizzatore.configurazione()
    }

    /// Quante volte si e' davvero disegnato. Fra un cambio e l'altro il
    /// fotogramma non viene rifatto: a 30 fps una parola dura una dozzina di
    /// fotogrammi, e ridisegnarli tutti sarebbe lavoro inutile.
    pub fn disegni(&self) -> u64 {
        self.disegni
    }

    /// Cosa e' visibile al tempo `t`.
    ///
    /// La ricerca parte dal blocco dell'ultima richiesta: scorrere la linea
    /// temporale in avanti — cioe' quello che fanno sia l'export sia la
    /// riproduzione — costa un confronto per fotogramma.
    pub fn stato(&mut self, t: f64) -> Stato {
        if self.blocchi.is_empty() {
            return None;
        }
        // Un salto all'indietro (l'utente trascina il cursore) riparte da capo.
        if self.cursore > 0 && self.blocchi[self.cursore - 1].end > t {
            self.cursore = 0;
        }
        while self.cursore < self.blocchi.len() && self.blocchi[self.cursore].end <= t {
            self.cursore += 1;
        }
        match self.blocchi.get(self.cursore) {
            Some(b) if t >= b.start => Some((self.cursore, b.parola_attiva(t))),
            _ => None,
        }
    }

    /// Il fotogramma al tempo `t`, in RGBA a alfa dritta.
    pub fn fotogramma(&mut self, t: f64) -> &[u8] {
        let stato = self.stato(t);
        self.disegna(stato)
    }

    /// Come [`Scena::fotogramma`], ma partendo da uno stato gia' calcolato.
    ///
    /// L'export lo usa perche' calcola gli stati di tutti i fotogrammi in
    /// anticipo, per raggrupparli.
    pub fn disegna(&mut self, stato: Stato) -> &[u8] {
        if self.disegnato == Some(stato) {
            return self.tela.pixel();
        }
        match stato {
            None => self.tela.pulisci(),
            Some((b, parola)) => {
                if self.preparato != Some(b) {
                    self.rasterizzatore.prepara(&self.blocchi[b]);
                    self.preparato = Some(b);
                }
                self.rasterizzatore.componi(parola, &mut self.tela);
            }
        }
        self.disegnato = Some(stato);
        self.disegni += 1;
        self.tela.pixel()
    }

    /// La tela, per chi deve consegnarla all'encoder.
    pub fn tela(&self) -> &Tela {
        &self.tela
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{impagina, Formato, Tipografo};
    use crate::render::Stile;
    use crate::trascrizione::Parola;

    const FONT: &[u8] = include_bytes!("../assets/Inter-Bold.ttf");

    fn scena_di_prova() -> Scena {
        let (larghezza, altezza) = Formato::Verticale.risoluzione();
        let cfg = LayoutConfig {
            larghezza,
            altezza,
            dimensione_font: Some(72.0),
            ..Default::default()
        };
        let mut t = Tipografo::nuovo(FONT, cfg.corpo(), cfg.interlinea).unwrap();
        let parole: Vec<Parola> = "una prova di sottotitoli"
            .split(' ')
            .enumerate()
            .map(|(i, p)| Parola::nuova(p, 1.0 + i as f64 * 0.5, 1.0 + i as f64 * 0.5 + 0.45))
            .collect();
        let blocchi = impagina(&parole, &mut t, &cfg).unwrap();
        Scena::nuova(blocchi, Rasterizzatore::nuovo(t, cfg, Stile::default()))
    }

    #[test]
    fn prima_del_primo_blocco_non_c_e_nulla() {
        let mut s = scena_di_prova();
        assert_eq!(s.stato(0.1), None);
        assert!(s.fotogramma(0.1).iter().all(|&b| b == 0), "la tela doveva restare vuota");
    }

    #[test]
    fn durante_il_parlato_c_e_qualcosa_da_vedere() {
        let mut s = scena_di_prova();
        assert!(s.stato(1.2).is_some());
        let opachi = s.fotogramma(1.2).chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(opachi > 0, "nessun pixel disegnato durante il parlato");
    }

    #[test]
    fn lo_stesso_stato_non_viene_ridisegnato() {
        let mut s = scena_di_prova();
        s.fotogramma(1.2);
        let dopo_il_primo = s.disegni();
        // Un altro istante dentro la stessa parola: nulla e' cambiato.
        s.fotogramma(1.22);
        assert_eq!(s.disegni(), dopo_il_primo, "lo stesso fotogramma e' stato rifatto");
    }

    #[test]
    fn tornare_indietro_nel_tempo_da_lo_stesso_risultato() {
        let mut s = scena_di_prova();
        let avanti = s.fotogramma(1.2).to_vec();
        s.fotogramma(2.4);
        // L'utente trascina il cursore all'indietro: la ricerca deve ripartire.
        let indietro = s.fotogramma(1.2).to_vec();
        assert_eq!(avanti, indietro, "lo stesso tempo ha dato due fotogrammi diversi");
    }

    #[test]
    fn una_scena_senza_blocchi_e_sempre_trasparente() {
        let (larghezza, altezza) = Formato::Verticale.risoluzione();
        let cfg = LayoutConfig { larghezza, altezza, ..Default::default() };
        let t = Tipografo::nuovo(FONT, cfg.corpo(), cfg.interlinea).unwrap();
        let mut s = Scena::nuova(Vec::new(), Rasterizzatore::nuovo(t, cfg, Stile::default()));
        assert_eq!(s.stato(5.0), None);
        assert!(s.fotogramma(5.0).iter().all(|&b| b == 0));
    }
}
