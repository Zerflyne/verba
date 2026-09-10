//! Disegno dei sottotitoli su fotogrammi RGBA con sfondo trasparente.
//!
//! Il disegno e' in due passate:
//!
//! 1. **rasterizzazione del blocco** — i glifi della riga vengono convertiti in
//!    una maschera di copertura (alfa per pixel), dalla quale si ricava il
//!    contorno con una trasformata di distanza. Dagli stessi glifi, gia'
//!    posizionati dallo shaping, si ricava il rettangolo di evidenziazione di
//!    ogni parola. Tutto cio' dipende solo dalla geometria del blocco, non dal
//!    tempo: si calcola **una volta per blocco**;
//! 2. **composizione** — a ogni cambio di parola indicata si ridisegna il
//!    rettangolo e si ricompone la maschera gia' pronta. E' un ciclo sui pixel
//!    del riquadro occupato dalla riga, quindi il costo per fotogramma resta
//!    trascurabile.
//!
//! L'ordine di sovrapposizione e' rettangolo, contorno, testo: il rettangolo
//! sta **dietro**, e il testo sopra resta del suo colore.
//!
//! L'alfa prodotta e' **dritta** (non premoltiplicata): e' quello che si aspetta
//! l'encoder ProRes 4444 e, a valle, qualsiasi montaggio video.

use cosmic_text::{Attrs, Buffer, Family, Shaping, SwashCache, SwashContent, Weight};

use crate::layout::{Allineamento, Blocco, LayoutConfig, Tipografo};

/// Colore RGBA a 8 bit per canale, alfa dritta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colore(pub [u8; 4]);

impl Colore {
    pub const BIANCO: Colore = Colore([255, 255, 255, 255]);
    pub const NERO: Colore = Colore([0, 0, 0, 255]);
    /// Viola dell'evidenziazione.
    pub const VIOLA: Colore = Colore([124, 58, 237, 255]);

    /// Legge `#RRGGBB` o `#RRGGBBAA` (il cancelletto e' facoltativo).
    pub fn da_esadecimale(s: &str) -> Result<Colore, String> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 && s.len() != 8 {
            return Err(format!("«{s}»: servono 6 o 8 cifre esadecimali"));
        }
        let mut c = [0u8, 0, 0, 255];
        for (i, coppia) in s.as_bytes().chunks(2).enumerate() {
            let testo = std::str::from_utf8(coppia).map_err(|_| "colore non valido".to_string())?;
            c[i] = u8::from_str_radix(testo, 16).map_err(|e| format!("«{testo}»: {e}"))?;
        }
        Ok(Colore(c))
    }
}

#[derive(Debug, Clone)]
pub struct Stile {
    /// Colore del testo. Resta lo stesso per tutte le parole: a indicare quella
    /// in corso e' il rettangolo dietro, non un cambio di colore.
    pub colore: Colore,
    /// Colore del rettangolo che sta dietro alla parola in corso.
    pub colore_evidenziazione: Colore,
    pub colore_bordo: Colore,
    /// Spessore del contorno del testo in pixel. 0 = nessun contorno.
    pub bordo: f32,
    /// Disegna il rettangolo di evidenziazione.
    pub evidenzia: bool,
    /// Margine orizzontale del rettangolo oltre la parola, in frazione del corpo.
    pub padding: f32,
    /// Altezza del rettangolo, in frazione del corpo.
    pub altezza: f32,
    /// Raggio degli angoli del rettangolo, in frazione del corpo.
    pub raggio: f32,
}

impl Default for Stile {
    fn default() -> Self {
        Self {
            colore: Colore::BIANCO,
            colore_evidenziazione: Colore::VIOLA,
            colore_bordo: Colore::NERO,
            bordo: 0.0,
            evidenzia: true,
            padding: 0.18,
            altezza: 1.12,
            raggio: 0.20,
        }
    }
}

/// Rettangolo smussato dell'evidenziazione, in coordinate del fotogramma.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rettangolo {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub raggio: f32,
}

impl Rettangolo {
    fn e_vuoto(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn larghezza(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn altezza(&self) -> f32 {
        self.y1 - self.y0
    }

    /// Copertura del pixel di centro `(px, py)`.
    ///
    /// E' la distanza con segno da un rettangolo con gli angoli arrotondati:
    /// dentro vale 1, fuori 0, e sul bordo sfuma su un pixel. Cosi' gli angoli
    /// restano lisci senza disegnare archi a mano.
    fn copertura(&self, px: f32, py: f32) -> f32 {
        let (cx, cy) = ((self.x0 + self.x1) * 0.5, (self.y0 + self.y1) * 0.5);
        let (mx, my) = ((self.x1 - self.x0) * 0.5, (self.y1 - self.y0) * 0.5);
        let r = self.raggio.clamp(0.0, mx.min(my));
        let dx = ((px - cx).abs() - (mx - r)).max(0.0);
        let dy = ((py - cy).abs() - (my - r)).max(0.0);
        let d = (dx * dx + dy * dy).sqrt() - r;
        (0.5 - d).clamp(0.0, 1.0)
    }
}

/// Riquadro di pixel, estremi in coordinate del fotogramma (`fine` esclusa).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Riquadro {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Riquadro {
    pub fn vuoto() -> Self {
        Self { x0: 0, y0: 0, x1: 0, y1: 0 }
    }
    pub fn e_vuoto(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
    pub fn larghezza(&self) -> usize {
        self.x1.saturating_sub(self.x0)
    }
    pub fn altezza(&self) -> usize {
        self.y1.saturating_sub(self.y0)
    }
}

/// Fotogramma RGBA.
///
/// La pulizia agisce solo sull'area effettivamente sporcata dal disegno
/// precedente: azzerare 8 MB a ogni fotogramma per ridisegnare una riga di
/// testo sarebbe lavoro sprecato.
pub struct Tela {
    larghezza: usize,
    altezza: usize,
    pixel: Vec<u8>,
    sporco: Riquadro,
}

impl Tela {
    pub fn nuova(larghezza: u32, altezza: u32) -> Self {
        let (larghezza, altezza) = (larghezza as usize, altezza as usize);
        Self { larghezza, altezza, pixel: vec![0; larghezza * altezza * 4], sporco: Riquadro::vuoto() }
    }

    pub fn pixel(&self) -> &[u8] {
        &self.pixel
    }

    pub fn dimensioni(&self) -> (usize, usize) {
        (self.larghezza, self.altezza)
    }

    pub fn pulisci(&mut self) {
        if self.sporco.e_vuoto() {
            return;
        }
        for y in self.sporco.y0..self.sporco.y1 {
            let inizio = (y * self.larghezza + self.sporco.x0) * 4;
            let fine = (y * self.larghezza + self.sporco.x1) * 4;
            self.pixel[inizio..fine].fill(0);
        }
        self.sporco = Riquadro::vuoto();
    }
}

/// Maschere e geometrie di un blocco, indipendenti dal tempo.
struct MascheraBlocco {
    regione: Riquadro,
    /// Copertura dei glifi, un byte per pixel della regione.
    copertura: Vec<u8>,
    /// Copertura del contorno, gia' unita a quella dei glifi.
    contorno: Vec<u8>,
    /// Rettangolo di evidenziazione di ogni parola del blocco.
    rettangoli: Vec<Rettangolo>,
}

impl MascheraBlocco {
    fn vuota() -> Self {
        Self {
            regione: Riquadro::vuoto(),
            copertura: Vec::new(),
            contorno: Vec::new(),
            rettangoli: Vec::new(),
        }
    }
}

/// Rasterizza i blocchi e li compone sui fotogrammi.
pub struct Rasterizzatore {
    tipografo: Tipografo,
    cache: SwashCache,
    buffer: Buffer,
    cfg: LayoutConfig,
    stile: Stile,
    maschera: MascheraBlocco,
}

impl Rasterizzatore {
    pub fn nuovo(mut tipografo: Tipografo, cfg: LayoutConfig, stile: Stile) -> Self {
        let metriche = tipografo.metriche();
        let mut buffer = Buffer::new(&mut tipografo.font_system, metriche);
        buffer.set_wrap(&mut tipografo.font_system, cosmic_text::Wrap::None);
        buffer.set_size(&mut tipografo.font_system, None, None);
        Self { tipografo, cache: SwashCache::new(), buffer, cfg, stile, maschera: MascheraBlocco::vuota() }
    }

    /// Rettangolo di evidenziazione della parola, nel blocco preparato.
    pub fn rettangolo(&self, parola: usize) -> Option<Rettangolo> {
        self.maschera.rettangoli.get(parola).copied()
    }

    /// Prepara maschere e rettangoli di un blocco. Va chiamata una volta per blocco.
    pub fn prepara(&mut self, blocco: &Blocco) {
        let larghezza = self.cfg.larghezza as usize;
        let altezza = self.cfg.altezza as usize;
        let corpo = self.cfg.corpo();
        let altezza_riga = self.cfg.altezza_riga();

        // Le righe stanno una sotto l'altra a partire dal bordo superiore del
        // blocco; la colonna e' quella della configurazione, e dentro la
        // colonna ogni riga si allinea come richiesto.
        let y_blocco = self.cfg.riga_y(blocco.righe.len());
        let colonna_x = self.cfg.colonna_x();
        let colonna_w = self.cfg.larghezza_utile();

        // Buffer di lavoro a dimensione fotogramma: l'indicizzazione resta
        // banale e il costo e' una manciata di megabyte riutilizzati sempre.
        let mut copertura = vec![0u8; larghezza * altezza];
        let mut min_x = usize::MAX;
        let mut min_y = usize::MAX;
        let mut max_x = 0usize;
        let mut max_y = 0usize;

        // Estremi orizzontali di ogni parola e centro verticale della sua
        // riga, letti dai glifi effettivamente posizionati: rimisurare la
        // parola isolata darebbe una larghezza diversa (crenatura con i vicini,
        // spazi) e il rettangolo si scosterebbe dal testo.
        let mut estremi =
            vec![(f32::INFINITY, f32::NEG_INFINITY, 0.0f32); blocco.parole.len()];

        let attrs = Attrs::new().family(Family::Name(&self.tipografo.famiglia)).weight(Weight::BOLD);

        for (k, riga) in blocco.righe.iter().enumerate() {
            let y_riga = y_blocco + k as f32 * altezza_riga;
            let centro_riga = y_riga + altezza_riga / 2.0;
            let x_riga = match self.cfg.allineamento {
                Allineamento::Sinistra => colonna_x,
                Allineamento::Centro => colonna_x + (colonna_w - riga.larghezza) / 2.0,
                Allineamento::Destra => colonna_x + colonna_w - riga.larghezza,
            };

            self.buffer.set_text(
                &mut self.tipografo.font_system,
                &riga.testo,
                &attrs,
                Shaping::Advanced,
            );
            self.buffer.shape_until_scroll(&mut self.tipografo.font_system, false);

            for run in self.buffer.layout_runs() {
                for glifo in run.glyphs.iter() {
                    if let Some(p) = riga
                        .parole
                        .iter()
                        .find(|p| p.byte.start <= glifo.start && glifo.start < p.byte.end)
                    {
                        let e = &mut estremi[p.indice];
                        // Il riquadro d'avanzamento, non l'inchiostro: e' quello
                        // che rende uniforme la spaziatura fra rettangolo e testo.
                        e.0 = e.0.min(x_riga + glifo.x);
                        e.1 = e.1.max(x_riga + glifo.x + glifo.w);
                        e.2 = centro_riga;
                    }

                    let fisico = glifo.physical((x_riga, y_riga), 1.0);
                    let Some(immagine) =
                        self.cache.get_image(&mut self.tipografo.font_system, fisico.cache_key)
                    else {
                        continue;
                    };
                    if immagine.content != SwashContent::Mask {
                        // Inter e' un font a contorni: niente bitmap a colori.
                        continue;
                    }

                    let base_x = fisico.x + immagine.placement.left;
                    let base_y = run.line_y as i32 + fisico.y - immagine.placement.top;
                    let gw = immagine.placement.width as i32;
                    let gh = immagine.placement.height as i32;

                    for oy in 0..gh {
                        let py = base_y + oy;
                        if py < 0 || py >= altezza as i32 {
                            continue;
                        }
                        for ox in 0..gw {
                            let px = base_x + ox;
                            if px < 0 || px >= larghezza as i32 {
                                continue;
                            }
                            let alfa = immagine.data[(oy * gw + ox) as usize];
                            if alfa == 0 {
                                continue;
                            }
                            let idx = py as usize * larghezza + px as usize;
                            copertura[idx] = copertura[idx].max(alfa);
                            min_x = min_x.min(px as usize);
                            min_y = min_y.min(py as usize);
                            max_x = max_x.max(px as usize);
                            max_y = max_y.max(py as usize);
                        }
                    }
                }
            }
        }

        if min_x == usize::MAX {
            self.maschera = MascheraBlocco::vuota();
            return;
        }

        // Il rettangolo prende l'altezza dal corpo e non dai limiti dei glifi:
        // altrimenti "pagina" (con discendente) e "come" (senza) avrebbero
        // rettangoli di forma diversa, e la riga sembrerebbe ballare. Il centro
        // e' quello della fascia di riga.
        let padding = self.stile.padding.max(0.0) * corpo;
        let mezza_altezza = self.stile.altezza.max(0.0) * corpo / 2.0;
        let raggio_angoli = self.stile.raggio.max(0.0) * corpo;
        let rettangoli: Vec<Rettangolo> = estremi
            .iter()
            .map(|&(a, b, centro_y)| {
                if !a.is_finite() || !b.is_finite() || b <= a {
                    // Parola senza glifi disegnabili: nessun rettangolo.
                    return Rettangolo { x0: 0.0, y0: 0.0, x1: 0.0, y1: 0.0, raggio: 0.0 };
                }
                Rettangolo {
                    x0: a - padding,
                    y0: centro_y - mezza_altezza,
                    x1: b + padding,
                    y1: centro_y + mezza_altezza,
                    raggio: raggio_angoli,
                }
            })
            .collect();

        // La regione da ridisegnare copre i glifi con il loro contorno e, se
        // l'evidenziazione e' attiva, la fascia di tutti i rettangoli: il
        // rettangolo cambia posizione a ogni parola, ma l'area sporcata no.
        let raggio_bordo = self.stile.bordo.max(0.0).ceil() as usize + 1;
        let mut regione = Riquadro {
            x0: min_x.saturating_sub(raggio_bordo),
            y0: min_y.saturating_sub(raggio_bordo),
            x1: (max_x + 1 + raggio_bordo).min(larghezza),
            y1: (max_y + 1 + raggio_bordo).min(altezza),
        };
        if self.stile.evidenzia {
            for r in rettangoli.iter().filter(|r| !r.e_vuoto()) {
                regione.x0 = regione.x0.min(r.x0.floor().max(0.0) as usize);
                regione.y0 = regione.y0.min(r.y0.floor().max(0.0) as usize);
                regione.x1 = regione.x1.max((r.x1.ceil().max(0.0) as usize + 1).min(larghezza));
                regione.y1 = regione.y1.max((r.y1.ceil().max(0.0) as usize + 1).min(altezza));
            }
        }

        let (rw, rh) = (regione.larghezza(), regione.altezza());
        let mut cop = vec![0u8; rw * rh];
        for y in 0..rh {
            let src = (regione.y0 + y) * larghezza + regione.x0;
            cop[y * rw..(y + 1) * rw].copy_from_slice(&copertura[src..src + rw]);
        }
        let contorno = dilata(&cop, rw, rh, self.stile.bordo);

        self.maschera = MascheraBlocco { regione, copertura: cop, contorno, rettangoli };
    }

    /// Compone il blocco preparato sulla tela.
    ///
    /// `parola_attiva` e' l'indice della parola indicata dal rettangolo, oppure
    /// `None` quando in quell'istante non ce n'e' nessuna: in tal caso il
    /// rettangolo semplicemente non viene disegnato, e resta la sola riga.
    pub fn componi(&self, parola_attiva: Option<usize>, tela: &mut Tela) {
        tela.pulisci();
        let m = &self.maschera;
        if m.regione.e_vuoto() {
            return;
        }
        let rw = m.regione.larghezza();
        let con_bordo = self.stile.bordo > 0.0 && self.stile.colore_bordo.0[3] > 0;

        // Il rettangolo si sposta a scatti da una parola all'altra: non c'e'
        // interpolazione, la geometria e' quella della parola indicata e basta.
        let rettangolo = if self.stile.evidenzia {
            parola_attiva
                .and_then(|i| m.rettangoli.get(i))
                .copied()
                .filter(|r| !r.e_vuoto() && self.stile.colore_evidenziazione.0[3] > 0)
        } else {
            None
        };
        // Limiti in coordinate della regione, per non valutare la distanza
        // firmata su tutti i pixel della riga.
        let limiti = rettangolo.map(|r| {
            (
                (r.x0.floor() as i64 - m.regione.x0 as i64).max(0) as usize,
                (r.y0.floor() as i64 - m.regione.y0 as i64).max(0) as usize,
                ((r.x1.ceil() as i64 - m.regione.x0 as i64).max(0) as usize + 1).min(rw),
                ((r.y1.ceil() as i64 - m.regione.y0 as i64).max(0) as usize + 1).min(m.regione.altezza()),
            )
        });

        for y in 0..m.regione.altezza() {
            let riga_tela = (m.regione.y0 + y) * tela.larghezza + m.regione.x0;
            for x in 0..rw {
                let i = y * rw + x;
                let cf = m.copertura[i];
                let cb = if con_bordo { m.contorno[i] } else { 0 };
                let cr = match (rettangolo, limiti) {
                    (Some(r), Some((lx0, ly0, lx1, ly1)))
                        if x >= lx0 && x < lx1 && y >= ly0 && y < ly1 =>
                    {
                        r.copertura(
                            (m.regione.x0 + x) as f32 + 0.5,
                            (m.regione.y0 + y) as f32 + 0.5,
                        )
                    }
                    _ => 0.0,
                };
                if cf == 0 && cb == 0 && cr <= 0.0 {
                    continue;
                }

                // Composizione premoltiplicata dal basso verso l'alto:
                // rettangolo, contorno, testo.
                let mut acc = [0.0f32; 4];
                sovrapponi(&mut acc, self.stile.colore_evidenziazione, cr);
                if con_bordo {
                    sovrapponi(&mut acc, self.stile.colore_bordo, cb as f32 / 255.0);
                }
                sovrapponi(&mut acc, self.stile.colore, cf as f32 / 255.0);

                let p = (riga_tela + x) * 4;
                if acc[3] <= 0.0 {
                    tela.pixel[p..p + 4].fill(0);
                    continue;
                }
                for c in 0..3 {
                    tela.pixel[p + c] = (acc[c] / acc[3] * 255.0).round().clamp(0.0, 255.0) as u8;
                }
                tela.pixel[p + 3] = (acc[3] * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        tela.sporco = m.regione;
    }
}

/// Sovrappone un colore all'accumulatore premoltiplicato (`source over`).
fn sovrapponi(acc: &mut [f32; 4], colore: Colore, copertura: f32) {
    let a = copertura.clamp(0.0, 1.0) * (colore.0[3] as f32 / 255.0);
    if a <= 0.0 {
        return;
    }
    let resto = 1.0 - a;
    for (canale, &sorgente) in acc.iter_mut().zip(colore.0.iter()).take(3) {
        *canale = (sorgente as f32 / 255.0) * a + *canale * resto;
    }
    acc[3] = a + acc[3] * resto;
}

/// Contorno della sagoma: alfa in funzione della distanza dal bordo dei glifi.
///
/// La distanza e' calcolata con una trasformata chamfer a due passate (pesi
/// 1 e √2): l'errore rispetto alla distanza euclidea resta sotto il 5 %, che
/// su un contorno di pochi pixel e' invisibile, e il costo e' lineare nell'area
/// invece che proporzionale al quadrato del raggio.
fn dilata(copertura: &[u8], w: usize, h: usize, raggio: f32) -> Vec<u8> {
    if raggio <= 0.0 || w == 0 || h == 0 {
        return copertura.to_vec();
    }
    const LONTANO: f32 = 1.0e9;
    const DIAG: f32 = std::f32::consts::SQRT_2;

    let mut d = vec![LONTANO; w * h];
    for i in 0..w * h {
        if copertura[i] >= 128 {
            d[i] = 0.0;
        }
    }

    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut v = d[i];
            if x > 0 {
                v = v.min(d[i - 1] + 1.0);
            }
            if y > 0 {
                v = v.min(d[i - w] + 1.0);
                if x > 0 {
                    v = v.min(d[i - w - 1] + DIAG);
                }
                if x + 1 < w {
                    v = v.min(d[i - w + 1] + DIAG);
                }
            }
            d[i] = v;
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            let mut v = d[i];
            if x + 1 < w {
                v = v.min(d[i + 1] + 1.0);
            }
            if y + 1 < h {
                v = v.min(d[i + w] + 1.0);
                if x + 1 < w {
                    v = v.min(d[i + w + 1] + DIAG);
                }
                if x > 0 {
                    v = v.min(d[i + w - 1] + DIAG);
                }
            }
            d[i] = v;
        }
    }

    let mut out = vec![0u8; w * h];
    for i in 0..w * h {
        let alfa = (raggio + 0.5 - d[i]).clamp(0.0, 1.0);
        out[i] = ((alfa * 255.0).round() as u8).max(copertura[i]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trascrizione::Parola;
    use crate::layout::{impagina, Formato};

    const FONT: &[u8] = include_bytes!("../assets/Inter-Bold.ttf");

    fn cfg() -> LayoutConfig {
        let (larghezza, altezza) = Formato::Verticale.risoluzione();
        LayoutConfig { larghezza, altezza, dimensione_font: Some(72.0), ..Default::default() }
    }

    /// Prepara il rasterizzatore sul primo blocco del testo dato.
    fn scena(testo: &str, cfg: &LayoutConfig, stile: Stile) -> (Rasterizzatore, Tela, Vec<Blocco>) {
        let mut t = Tipografo::nuovo(FONT, cfg.corpo(), cfg.interlinea).unwrap();
        let parole: Vec<Parola> = testo
            .split(' ')
            .enumerate()
            .map(|(i, p)| Parola::nuova(p, i as f64 * 0.4, i as f64 * 0.4 + 0.35))
            .collect();
        let blocchi = impagina(&parole, &mut t, cfg).unwrap();
        let mut r = Rasterizzatore::nuovo(t, cfg.clone(), stile);
        r.prepara(&blocchi[0]);
        let tela = Tela::nuova(cfg.larghezza, cfg.altezza);
        (r, tela, blocchi)
    }

    fn pixel(tela: &Tela, x: usize, y: usize) -> [u8; 4] {
        let (w, _) = tela.dimensioni();
        let p = (y * w + x) * 4;
        [tela.pixel()[p], tela.pixel()[p + 1], tela.pixel()[p + 2], tela.pixel()[p + 3]]
    }

    fn conta_colore(tela: &Tela, c: Colore) -> usize {
        tela.pixel()
            .chunks_exact(4)
            .filter(|p| p[3] > 200 && p[0] == c.0[0] && p[1] == c.0[1] && p[2] == c.0[2])
            .count()
    }

    #[test]
    fn il_colore_esadecimale_si_legge_con_e_senza_alfa() {
        assert_eq!(Colore::da_esadecimale("#FFFFFF").unwrap(), Colore::BIANCO);
        assert_eq!(Colore::da_esadecimale("000000ff").unwrap(), Colore::NERO);
        assert_eq!(Colore::da_esadecimale("#10203040").unwrap(), Colore([16, 32, 48, 64]));
        assert_eq!(Colore::da_esadecimale("#7C3AED").unwrap(), Colore::VIOLA);
        assert!(Colore::da_esadecimale("#12345").is_err());
        assert!(Colore::da_esadecimale("#gggggg").is_err());
    }

    /// Colonne estreme occupate da un pixel non trasparente.
    fn estremi_opachi(tela: &Tela) -> (usize, usize) {
        let (w, _) = tela.dimensioni();
        let mut min = usize::MAX;
        let mut max = 0usize;
        for (i, p) in tela.pixel().chunks_exact(4).enumerate() {
            if p[3] > 0 {
                let x = i % w;
                min = min.min(x);
                max = max.max(x);
            }
        }
        assert!(min != usize::MAX, "nessun pixel disegnato");
        (min, max)
    }

    #[test]
    fn il_testo_resta_nei_margini_e_il_rettangolo_nel_fotogramma() {
        let cfg = cfg();
        let stile = Stile::default();
        let (r, mut tela, _) =
            scena("sottotitoli nitidi su sfondo trasparente", &cfg, stile.clone());
        let margine = cfg.margine_x() as usize;

        // Senza rettangolo resta il solo testo, che deve stare nei margini.
        r.componi(None, &mut tela);
        let (min_testo, max_testo) = estremi_opachi(&tela);
        assert!(min_testo >= margine, "testo a x={min_testo}, margine {margine}");
        assert!(
            max_testo < cfg.larghezza as usize - margine,
            "testo a x={max_testo}, margine {margine}"
        );

        // Il rettangolo puo' sconfinare nel margine del solo padding — e' uno
        // sfondo, non testo — ma mai uscire dal fotogramma.
        r.componi(Some(0), &mut tela);
        let (min_rett, max_rett) = estremi_opachi(&tela);
        let padding = (stile.padding * cfg.corpo()).ceil() as usize + 1;
        assert!(
            min_rett + padding >= margine,
            "il rettangolo sconfina di {} px, oltre il padding di {padding}",
            margine.saturating_sub(min_rett)
        );
        assert!(max_rett < cfg.larghezza as usize, "il rettangolo esce dal fotogramma");
    }

    #[test]
    fn lo_sfondo_resta_completamente_trasparente() {
        let cfg = cfg();
        let (r, mut tela, _) = scena("prova", &cfg, Stile::default());
        r.componi(Some(0), &mut tela);
        let opachi = tela.pixel().chunks_exact(4).filter(|p| p[3] > 0).count();
        let totale = cfg.larghezza as usize * cfg.altezza as usize;
        assert!(opachi > 0, "niente disegnato");
        assert!(opachi < totale / 20, "troppo pieno: {opachi} pixel su {totale}");
    }

    #[test]
    fn la_pulizia_cancella_il_disegno_precedente() {
        let cfg = cfg();
        let (r, mut tela, _) = scena("prima parola", &cfg, Stile::default());
        r.componi(Some(0), &mut tela);
        assert!(tela.pixel().iter().any(|&v| v != 0));
        tela.pulisci();
        assert!(tela.pixel().iter().all(|&v| v == 0), "restano pixel accesi dopo la pulizia");
    }

    // ----------------------------------------------------- evidenziazione

    #[test]
    fn il_rettangolo_sta_dietro_e_il_testo_resta_bianco() {
        let cfg = cfg();
        let (r, mut tela, _) = scena("alfa beta", &cfg, Stile::default());
        r.componi(Some(0), &mut tela);

        let viola = conta_colore(&tela, Colore::VIOLA);
        let bianco = conta_colore(&tela, Colore::BIANCO);
        assert!(viola > 0, "nessun pixel del rettangolo");
        assert!(bianco > 0, "il testo non e' rimasto bianco");
        // Il rettangolo e' molto piu' grande dei glifi che copre.
        assert!(viola > bianco, "viola {viola} <= bianco {bianco}");
    }

    #[test]
    fn il_rettangolo_avvolge_i_glifi_della_parola_indicata() {
        let cfg = cfg();
        let (r, mut tela, blocchi) = scena("alfa beta gamma", &cfg, Stile::default());
        let rett = r.rettangolo(1).unwrap();

        // Tutti i pixel bianchi dentro la fascia verticale del rettangolo e
        // dentro i suoi estremi orizzontali appartengono a "beta"; nessun
        // pixel del testo di "alfa" o "gamma" ci finisce dentro.
        r.componi(Some(1), &mut tela);
        let mut visti = 0;
        for y in rett.y0.ceil() as usize..rett.y1.floor() as usize {
            for x in rett.x0.ceil() as usize..rett.x1.floor() as usize {
                if pixel(&tela, x, y) == Colore::BIANCO.0 {
                    visti += 1;
                }
            }
        }
        assert!(visti > 0, "nessun glifo dentro il rettangolo");

        // I rettangoli delle tre parole sono ordinati e non si sovrappongono
        // piu' del doppio del padding.
        let r0 = r.rettangolo(0).unwrap();
        let r2 = r.rettangolo(2).unwrap();
        assert!(r0.x1 <= rett.x1 && rett.x1 <= r2.x1, "rettangoli fuori ordine");
        assert!(r0.x0 < rett.x0 && rett.x0 < r2.x0);
        assert_eq!(blocchi[0].parole.len(), 3);
    }

    #[test]
    fn il_rettangolo_supera_la_parola_del_padding() {
        let cfg = cfg();
        let stile = Stile { padding: 0.25, ..Default::default() };
        let (r, _, _) = scena("alfa", &cfg, stile.clone());
        let stretto = {
            let (r2, _, _) = scena("alfa", &cfg, Stile { padding: 0.0, ..stile.clone() });
            r2.rettangolo(0).unwrap()
        };
        let largo = r.rettangolo(0).unwrap();
        let atteso = 2.0 * 0.25 * cfg.corpo();
        assert!(
            (largo.larghezza() - stretto.larghezza() - atteso).abs() < 0.01,
            "{} vs {} (+{atteso})",
            largo.larghezza(),
            stretto.larghezza()
        );
    }

    #[test]
    fn l_altezza_del_rettangolo_non_dipende_dai_discendenti() {
        let cfg = cfg();
        // "pappagalli" ha discendenti (p, g), "concorso" no: se l'altezza
        // venisse dai limiti dei glifi, i due rettangoli sarebbero diversi.
        let (con, _, _) = scena("pappagalli", &cfg, Stile::default());
        let (senza, _, _) = scena("concorso", &cfg, Stile::default());
        let a = con.rettangolo(0).unwrap();
        let b = senza.rettangolo(0).unwrap();
        assert!((a.altezza() - b.altezza()).abs() < 1e-4, "{} vs {}", a.altezza(), b.altezza());
        assert!((a.y0 - b.y0).abs() < 1e-4 && (a.y1 - b.y1).abs() < 1e-4, "fasce diverse");
        assert!(
            (a.altezza() - Stile::default().altezza * cfg.corpo()).abs() < 1e-3,
            "l'altezza non deriva dal corpo: {}",
            a.altezza()
        );
    }

    /// Estremi verticali dei pixel non trasparenti.
    fn estremi_opachi_y(tela: &Tela) -> (usize, usize) {
        let (w, _) = tela.dimensioni();
        let mut min = usize::MAX;
        let mut max = 0usize;
        for (i, p) in tela.pixel().chunks_exact(4).enumerate() {
            if p[3] > 0 {
                let y = i / w;
                min = min.min(y);
                max = max.max(y);
            }
        }
        assert!(min != usize::MAX, "nessun pixel disegnato");
        (min, max)
    }

    #[test]
    fn con_due_righe_il_rettangolo_segue_la_riga_della_parola() {
        let cfg = LayoutConfig { righe_max: 2, ..cfg() };
        let (r, _, blocchi) = scena(
            "il rapido allineamento delle parole permette sottotitoli precisi",
            &cfg,
            Stile::default(),
        );
        let b = &blocchi[0];
        assert_eq!(b.righe.len(), 2, "serviva un blocco su due righe: {}", b.testo());

        let prima: usize = b.righe[0].parole[0].indice;
        let seconda: usize = b.righe[1].parole[0].indice;
        let (ra, rb) = (r.rettangolo(prima).unwrap(), r.rettangolo(seconda).unwrap());

        let y_blocco = cfg.riga_y(2);
        let h = cfg.altezza_riga();
        for (rett, k) in [(ra, 0.0f32), (rb, 1.0f32)] {
            let centro_atteso = y_blocco + (k + 0.5) * h;
            assert!(
                ((rett.y0 + rett.y1) / 2.0 - centro_atteso).abs() < 1e-3,
                "rettangolo centrato a {} invece che a {centro_atteso}",
                (rett.y0 + rett.y1) / 2.0
            );
        }
        assert!(rb.y0 > ra.y0, "la seconda riga deve stare sotto la prima");
    }

    #[test]
    fn due_righe_occupano_piu_altezza_di_una() {
        let testo = "il rapido allineamento delle parole permette sottotitoli precisi";
        let cfg1 = LayoutConfig { righe_max: 1, ..cfg() };
        let cfg2 = LayoutConfig { righe_max: 2, ..cfg() };
        let (r1, mut t1, _) = scena(testo, &cfg1, Stile::default());
        let (r2, mut t2, _) = scena(testo, &cfg2, Stile::default());
        r1.componi(None, &mut t1);
        r2.componi(None, &mut t2);
        let (a0, a1) = estremi_opachi_y(&t1);
        let (b0, b1) = estremi_opachi_y(&t2);
        assert!(b1 - b0 > a1 - a0, "due righe non occupano piu' spazio verticale di una");
    }

    #[test]
    fn l_allineamento_sposta_il_testo_dentro_la_colonna() {
        let testo = "una prova breve";
        let mut estremi = Vec::new();
        for allineamento in [Allineamento::Sinistra, Allineamento::Centro, Allineamento::Destra] {
            let cfg = LayoutConfig { allineamento, ..cfg() };
            let (r, mut tela, _) = scena(testo, &cfg, Stile::default());
            r.componi(None, &mut tela);
            estremi.push(estremi_opachi(&tela));
        }
        let (sx, _) = estremi[0];
        let (cx, _) = estremi[1];
        let (dx, _) = estremi[2];
        assert!(sx < cx && cx < dx, "allineamenti indistinguibili: {sx}, {cx}, {dx}");

        // A sinistra il testo si appoggia al bordo della colonna. Lo scarto
        // ammesso e' il fianco sinistro del primo glifo, che dipende dalla
        // lettera: si misura in frazione del corpo, non in pixel fissi.
        let cfg = cfg();
        let tolleranza = 0.15 * cfg.corpo();
        assert!(
            (sx as f32 - cfg.colonna_x()).abs() < tolleranza,
            "a sinistra il testo parte da {sx}, la colonna da {}",
            cfg.colonna_x()
        );
    }

    #[test]
    fn la_posizione_verticale_sposta_il_blocco() {
        let testo = "una prova breve";
        let mut centri = Vec::new();
        for posizione in [0.2f32, 0.5, 0.9] {
            let cfg = LayoutConfig { posizione_verticale: posizione, ..cfg() };
            let (r, mut tela, _) = scena(testo, &cfg, Stile::default());
            r.componi(None, &mut tela);
            let (y0, y1) = estremi_opachi_y(&tela);
            centri.push((y0 + y1) / 2);
        }
        assert!(
            centri[0] < centri[1] && centri[1] < centri[2],
            "la posizione verticale non sposta il testo: {centri:?}"
        );
    }

    #[test]
    fn il_rettangolo_e_centrato_sulla_fascia_di_riga() {
        let cfg = cfg();
        let (r, _, _) = scena("prova", &cfg, Stile::default());
        let rett = r.rettangolo(0).unwrap();
        let centro_atteso = cfg.riga_y(1) + cfg.altezza_riga() / 2.0;
        assert!(
            ((rett.y0 + rett.y1) / 2.0 - centro_atteso).abs() < 1e-4,
            "centro {} invece di {centro_atteso}",
            (rett.y0 + rett.y1) / 2.0
        );
    }

    #[test]
    fn gli_angoli_sono_smussati() {
        let cfg = cfg();
        let (r, mut tela, _) = scena("prova", &cfg, Stile { raggio: 0.30, ..Default::default() });
        r.componi(Some(0), &mut tela);
        let rett = r.rettangolo(0).unwrap();
        // L'angolo geometrico e' fuori dalla forma arrotondata...
        let ax = rett.x0.ceil() as usize;
        let ay = rett.y0.ceil() as usize;
        assert_eq!(pixel(&tela, ax, ay)[3], 0, "l'angolo non e' smussato");
        // ...mentre il centro del bordo superiore e' pieno.
        let cx = ((rett.x0 + rett.x1) / 2.0) as usize;
        assert!(pixel(&tela, cx, ay + 2)[3] > 200, "il bordo superiore non e' pieno");
    }

    #[test]
    fn senza_parola_attiva_non_si_disegna_il_rettangolo() {
        let cfg = cfg();
        let (r, mut tela, _) = scena("alfa beta", &cfg, Stile::default());
        r.componi(None, &mut tela);
        assert_eq!(conta_colore(&tela, Colore::VIOLA), 0, "rettangolo disegnato senza parola attiva");
        assert!(conta_colore(&tela, Colore::BIANCO) > 0, "il testo doveva restare");
    }

    #[test]
    fn senza_evidenziazione_resta_solo_il_testo() {
        let cfg = cfg();
        let stile = Stile { evidenzia: false, ..Default::default() };
        let (r, mut tela, _) = scena("alfa beta", &cfg, stile);
        r.componi(Some(0), &mut tela);
        assert_eq!(conta_colore(&tela, Colore::VIOLA), 0);
        assert!(conta_colore(&tela, Colore::BIANCO) > 0);
    }

    #[test]
    fn il_rettangolo_si_sposta_a_scatti_fra_le_parole() {
        let cfg = cfg();
        let (r, mut tela, _) = scena("alfa beta", &cfg, Stile::default());
        let centro = |t: &Tela| -> f32 {
            let (w, _) = t.dimensioni();
            let mut somma = 0.0;
            let mut n = 0.0;
            for (i, p) in t.pixel().chunks_exact(4).enumerate() {
                if p[3] > 200 && p[..3] == Colore::VIOLA.0[..3] {
                    somma += (i % w) as f32;
                    n += 1.0;
                }
            }
            somma / n
        };
        r.componi(Some(0), &mut tela);
        let prima = centro(&tela);
        r.componi(Some(1), &mut tela);
        let dopo = centro(&tela);
        assert!(dopo > prima + cfg.corpo(), "il rettangolo non e' saltato: {prima} -> {dopo}");
    }

    #[test]
    fn il_contorno_allarga_la_sagoma() {
        let cfg = cfg();
        let senza = {
            let stile = Stile { bordo: 0.0, evidenzia: false, ..Default::default() };
            let (r, mut tela, _) = scena("bordo", &cfg, stile);
            r.componi(Some(0), &mut tela);
            tela.pixel().chunks_exact(4).filter(|p| p[3] > 0).count()
        };
        let stile = Stile { bordo: 6.0, evidenzia: false, ..Default::default() };
        let (r, mut tela, _) = scena("bordo", &cfg, stile);
        r.componi(Some(0), &mut tela);
        let con = tela.pixel().chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(con > senza, "con bordo {con} <= senza bordo {senza}");
    }

    #[test]
    fn la_dilatazione_a_raggio_nullo_non_cambia_nulla() {
        let m = vec![0u8, 255, 0, 255];
        assert_eq!(dilata(&m, 2, 2, 0.0), m);
    }
}
