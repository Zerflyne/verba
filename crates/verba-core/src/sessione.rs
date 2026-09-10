//! Lo stato di un lavoro aperto: il file, l'audio, le parole, l'aspetto.
//!
//! Serve all'applicazione, ma non sa che l'applicazione esiste. Tiene insieme
//! le cose che nell'interfaccia stanno in stanze diverse — il file caricato,
//! la trascrizione, il preset in corso, la scena da disegnare — e le espone
//! come operazioni che una finestra puo' chiamare.
//!
//! Il punto fermo e' [`Sessione::fotogramma`]: **l'anteprima esce dallo stesso
//! codice dell'export**. Non c'e' un'impaginazione per lo schermo e una per il
//! file; c'e' una scena sola, e sia il disegno a schermo sia la codifica le
//! chiedono il fotogramma al tempo `t`. Se cosi' non fosse, anteprima ed
//! export divergerebbero su qualche dettaglio, e trovare il perche' costerebbe
//! giorni.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::audio::{self, AudioInput, Pcm, PreprocessConfig};
use crate::caratteri::{Catalogo, Esito};
use crate::encoder::FormatoVideo;
use crate::eventi::{Fase, Progresso};
use crate::gpu::Device;
use crate::layout::{self, LayoutConfig, Tipografo};
use crate::media::{Informazioni, Media, Modalita};
use crate::pipeline::{self, ConfigTrascrizione};
use crate::progetto::Preset;
use crate::render::Rasterizzatore;
use crate::scena::Scena;
use crate::trascrizione::{IdParola, Parola, Trascrizione};
use crate::video::{self, Sfondo, VideoConfig};

/// Quante colonne ha la forma d'onda mostrata sotto il trasporto.
///
/// E' una figura larga meno di 1600 px: piu' colonne di cosi' non si vedono, e
/// mandarle alla finestra costerebbe soltanto memoria.
pub const COLONNE_ONDA: usize = 1600;

/// Le caratteristiche del file aperto, come le mostra la barra di stato.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Descrizione {
    pub percorso: String,
    pub nome: String,
    /// `audio` oppure `video`: e' il badge accanto al nome del file.
    pub modalita: String,
    pub larghezza: u32,
    pub altezza: u32,
    pub fps: f64,
    pub durata: f64,
    pub codec_video: Option<String>,
    pub codec_audio: Option<String>,
    /// La riga sola che riassume tutto: «1280x720 · 25.00 fps · 00:09».
    pub riassunto: String,
}

/// Una parola come la vede la striscia in basso.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParolaVista {
    pub id: u64,
    pub testo: String,
    pub inizio: f64,
    pub fine: f64,
    pub confidenza: f32,
    /// Sotto la soglia di segnalazione: e' li' che conviene guardare.
    pub incerta: bool,
}

/// Cosa e' venuto fuori dalla trascrizione, per la barra di stato.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Riepilogo {
    pub parole: usize,
    pub incerte: usize,
    pub blocchi: usize,
    pub secondi: f64,
    pub dispositivo: String,
    pub modello: String,
}

/// Un lavoro aperto.
pub struct Sessione {
    percorso: PathBuf,
    info: Informazioni,
    pcm: Pcm,
    onda: Vec<f32>,
    trascrizione: Option<Trascrizione>,
    preset: Preset,
    /// La soglia sotto la quale una parola viene segnalata.
    soglia: f32,
    scena: Option<Scena>,
    /// Il filmato di fondo per l'anteprima, aperto solo se serve.
    filmato: Option<Media>,
    /// Il fotogramma composto: sfondo piu' sottotitoli.
    composto: Vec<u8>,
    avviso_carattere: Option<String>,
    riepilogo: Option<Riepilogo>,
}

impl Sessione {
    /// Apre un file e ne prepara l'audio. Non carica alcun modello.
    ///
    /// E' la parte che deve essere veloce: dal trascinamento del file alla
    /// scheda che ne mostra le caratteristiche non deve passare un'attesa.
    pub fn apri(percorso: &Path, progresso: &Progresso) -> Result<Self> {
        let media = Media::apri(percorso)
            .with_context(|| format!("apertura di {}", percorso.display()))?;
        let info = media.informazioni().clone();

        let pcm = {
            let _c = progresso.inizia(Fase::Preparazione);
            let ingresso = AudioInput::from_cli_arg(&percorso.to_string_lossy());
            audio::load_and_preprocess(&[ingresso], &PreprocessConfig::default())?
        };
        let onda = forma_onda(&pcm, COLONNE_ONDA);

        info!(
            file = %percorso.display(),
            modalita = ?info.modalita,
            descrizione = %info.descrizione(),
            "sessione aperta"
        );

        Ok(Self {
            percorso: percorso.to_path_buf(),
            filmato: info.e_video().then_some(media),
            info,
            pcm,
            onda,
            trascrizione: None,
            preset: crate::progetto::di_serie()
                .into_iter()
                .next()
                .unwrap_or_else(|| Preset::da(
                    "Predefinito",
                    &LayoutConfig::default(),
                    &Default::default(),
                    &Default::default(),
                    Default::default(),
                )),
            soglia: 0.5,
            scena: None,
            composto: Vec::new(),
            avviso_carattere: None,
            riepilogo: None,
        })
    }

    pub fn percorso(&self) -> &Path {
        &self.percorso
    }

    pub fn informazioni(&self) -> &Informazioni {
        &self.info
    }

    pub fn e_video(&self) -> bool {
        self.info.e_video()
    }

    pub fn durata(&self) -> f64 {
        // La durata dell'audio e' quella che conta: e' su quella che sono
        // costruiti i tempi delle parole.
        self.pcm.duration_secs()
    }

    /// La forma d'onda gia' ridotta a colonne, ognuna fra 0 e 1.
    pub fn onda(&self) -> &[f32] {
        &self.onda
    }

    pub fn descrizione(&self) -> Descrizione {
        Descrizione {
            percorso: self.percorso.display().to_string(),
            nome: self
                .percorso
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            modalita: match self.info.modalita {
                Modalita::Audio => "audio".into(),
                Modalita::Video => "video".into(),
            },
            larghezza: self.info.larghezza,
            altezza: self.info.altezza,
            fps: self.info.fps(),
            durata: self.durata(),
            codec_video: self.info.codec_video.clone(),
            codec_audio: self.info.codec_audio.clone(),
            riassunto: self.info.descrizione(),
        }
    }

    /// La soglia di segnalazione delle parole incerte.
    pub fn soglia(&self) -> f32 {
        self.soglia
    }

    pub fn imposta_soglia(&mut self, soglia: f32) {
        self.soglia = soglia.clamp(0.0, 1.0);
    }

    pub fn preset(&self) -> &Preset {
        &self.preset
    }

    pub fn trascritta(&self) -> bool {
        self.trascrizione.is_some()
    }

    pub fn riepilogo(&self) -> Option<&Riepilogo> {
        self.riepilogo.as_ref()
    }

    /// L'avviso sul carattere, se ne e' stato sostituito uno.
    pub fn avviso_carattere(&self) -> Option<&str> {
        self.avviso_carattere.as_deref()
    }

    /// Trascrive, e prepara la scena con l'aspetto corrente.
    ///
    /// L'ordine dei modelli sta in [`crate::pipeline`], lo stesso della riga di
    /// comando: qui non si riorchestra niente.
    pub fn trascrivi(
        &mut self,
        cfg: &ConfigTrascrizione,
        device: &Device,
        catalogo: Catalogo,
        progresso: &Progresso,
    ) -> Result<()> {
        let avvio = std::time::Instant::now();
        let trascrizione = pipeline::trascrivi(&self.pcm, device, cfg, progresso)?;
        let parole = trascrizione.len();
        let incerte = trascrizione.incerte(self.soglia).count();
        self.trascrizione = Some(trascrizione);
        self.impagina(catalogo, progresso)?;

        self.riepilogo = Some(Riepilogo {
            parole,
            incerte,
            blocchi: self.scena.as_ref().map(|s| s.blocchi().len()).unwrap_or(0),
            secondi: avvio.elapsed().as_secs_f64(),
            dispositivo: device.describe(),
            modello: cfg
                .modelli
                .whisper
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        });
        Ok(())
    }

    /// Cambia l'aspetto e ricompone. Non rilancia alcun modello: e' quello che
    /// permette al pannello di destra di applicarsi entro un fotogramma.
    pub fn applica(&mut self, preset: Preset, catalogo: Catalogo, progresso: &Progresso) -> Result<()> {
        self.preset = preset;
        if self.trascrizione.is_some() {
            self.impagina(catalogo, progresso)?;
        }
        Ok(())
    }

    /// La configurazione di impaginazione che nasce dal preset corrente.
    pub fn layout(&self) -> LayoutConfig {
        self.preset.layout(self.info.risoluzione())
    }

    /// Ricostruisce righe e scena a partire dal preset corrente.
    fn impagina(&mut self, catalogo: Catalogo, progresso: &Progresso) -> Result<()> {
        let Some(trascrizione) = &self.trascrizione else {
            bail!("non c'e' ancora una trascrizione da impaginare");
        };
        let cfg = self.layout();
        let mut catalogo = catalogo;
        let esito: Esito = catalogo.risolvi(&self.preset.carattere())?;
        self.avviso_carattere = esito.avviso();
        let mut tipografo = Tipografo::dal_catalogo(catalogo, &esito, cfg.corpo(), cfg.interlinea)
            .context("preparazione del carattere")?;

        let blocchi = {
            let _c = progresso.inizia(Fase::Impaginazione);
            layout::impagina(trascrizione.parole(), &mut tipografo, &cfg)?
        };
        let stile = self.preset.stile()?;
        self.scena = Some(Scena::nuova(blocchi, Rasterizzatore::nuovo(tipografo, cfg, stile)));
        Ok(())
    }

    /// Il fotogramma da mostrare al tempo `t`: sfondo e sottotitoli sopra.
    ///
    /// In modalita' video lo sfondo e' il fotogramma del filmato; in modalita'
    /// audio non c'e' sfondo, e restano i soli sottotitoli su trasparenza —
    /// esattamente come uscirebbero da un overlay.
    pub fn fotogramma(&mut self, t: f64) -> Result<&[u8]> {
        let Some(scena) = &mut self.scena else {
            bail!("non c'e' ancora niente da disegnare");
        };
        let (larghezza, altezza) = scena.tela().dimensioni();
        let byte = larghezza * altezza * 4;

        if self.composto.len() != byte {
            self.composto = vec![0; byte];
        }

        // Lo sfondo: il fotogramma del filmato se le dimensioni coincidono.
        let mut disegnato = false;
        if let Some(media) = &mut self.filmato {
            let info = media.informazioni();
            if info.larghezza as usize == larghezza && info.altezza as usize == altezza {
                disegnato = media.fotogramma(t, &mut self.composto)?;
            }
        }
        if !disegnato {
            self.composto.fill(0);
        }

        video::sovrapponi(&mut self.composto, scena.fotogramma(t));
        Ok(&self.composto)
    }

    /// Le dimensioni del fotogramma d'anteprima.
    pub fn dimensioni(&self) -> (u32, u32) {
        match &self.scena {
            Some(s) => {
                let (l, a) = s.tela().dimensioni();
                (l as u32, a as u32)
            }
            None => self.info.risoluzione().unwrap_or((1080, 1920)),
        }
    }

    /// Tutte le parole, per la striscia in basso.
    pub fn parole(&self) -> Vec<ParolaVista> {
        match &self.trascrizione {
            Some(t) => t.parole().iter().map(|p| self.vista(p)).collect(),
            None => Vec::new(),
        }
    }

    /// La finestra di `quante` parole intorno al tempo `t`.
    ///
    /// Nella 0.1 e' in sola lettura: dice dove guardare, e non ancora cosa
    /// correggere.
    pub fn finestra(&self, t: f64, quante: usize) -> Vec<ParolaVista> {
        let Some(tr) = &self.trascrizione else { return Vec::new() };
        let parole = tr.parole();
        if parole.is_empty() {
            return Vec::new();
        }
        let centro = parole
            .iter()
            .position(|p| t < p.fine)
            .unwrap_or(parole.len().saturating_sub(1));
        let meta = quante / 2;
        let da = centro.saturating_sub(meta);
        let a = (da + quante).min(parole.len());
        let da = a.saturating_sub(quante);
        parole[da..a].iter().map(|p| self.vista(p)).collect()
    }

    /// La parola pronunciata al tempo `t`, se ce n'e' una.
    pub fn parola_al_tempo(&self, t: f64) -> Option<IdParola> {
        self.trascrizione.as_ref()?.al_tempo(t).map(|p| p.id)
    }

    fn vista(&self, p: &Parola) -> ParolaVista {
        ParolaVista {
            id: u64::from(p.id),
            testo: p.testo.clone(),
            inizio: p.inizio,
            fine: p.fine,
            confidenza: p.confidenza,
            incerta: p.incerta(self.soglia),
        }
    }

    /// I formati che ha senso proporre per questo file.
    ///
    /// Da un file audio non si puo' produrre un video sottotitolato: quei
    /// formati non vanno mostrati disabilitati, vanno nascosti.
    pub fn formati_video(&self) -> Vec<FormatoVideo> {
        FormatoVideo::TUTTI
            .into_iter()
            .filter(|f| self.e_video() || f.ha_alfa())
            .collect()
    }

    /// Il nome proposto per un export.
    pub fn nome_proposto(&self, formato: FormatoVideo) -> PathBuf {
        let radice =
            self.percorso.file_stem().and_then(|s| s.to_str()).unwrap_or("uscita").to_string();
        let nome = format!("{radice}{}.{}", formato.suffisso(), formato.estensione());
        match self.percorso.parent() {
            Some(d) if !d.as_os_str().is_empty() => d.join(nome),
            _ => PathBuf::from(nome),
        }
    }

    /// Le parole come le vede il motore, per le uscite testuali.
    pub fn parole_grezze(&self) -> &[Parola] {
        self.trascrizione.as_ref().map(|t| t.parole()).unwrap_or(&[])
    }

    /// I blocchi impaginati, per le uscite testuali.
    pub fn blocchi(&self) -> &[crate::layout::Blocco] {
        self.scena.as_ref().map(|s| s.blocchi()).unwrap_or(&[])
    }

    /// Esporta un file video.
    pub fn esporta(
        &mut self,
        formato: FormatoVideo,
        destinazione: &Path,
        qualita: u32,
        thread: usize,
        progresso: &Progresso,
    ) -> Result<video::Statistiche> {
        let Some(scena) = &mut self.scena else {
            bail!("non c'e' ancora niente da esportare");
        };
        if !formato.ha_alfa() && !self.info.e_video() {
            bail!(
                "«{}» imprime i sottotitoli su un filmato, e questo e' un file audio. \
                 Da un file audio si puo' produrre solo un overlay.",
                formato.etichetta()
            );
        }
        let (fps_num, fps_den) = if self.info.e_video() && self.info.fps_num > 0 {
            (self.info.fps_num, self.info.fps_den.max(1))
        } else {
            (30, 1)
        };
        let vcfg = VideoConfig {
            formato,
            fps_num,
            fps_den,
            qualita,
            thread,
            durata: self.pcm.duration_secs(),
        };

        // Il filmato di fondo viene riaperto: quello dell'anteprima ha il
        // cursore dove l'ha lasciato l'utente, e un export deve partire da
        // zero comunque sia stata usata l'anteprima.
        let mut fondo = if formato.ha_alfa() { None } else { Some(Media::apri(&self.percorso)?) };
        let percorso = self.percorso.clone();
        let sfondo = match &mut fondo {
            Some(m) => Sfondo::Filmato { media: m, percorso: &percorso },
            None => Sfondo::Trasparente,
        };
        let _c = progresso.inizia(Fase::Codifica);
        video::esporta(scena, sfondo, &vcfg, destinazione, progresso)
    }
}

/// Riduce l'onda a `colonne` picchi fra 0 e 1.
///
/// Si prende il picco e non la media: una forma d'onda fatta di medie e'
/// piatta e non dice piu' dove si parla, che e' l'unica cosa per cui la si
/// guarda.
pub fn forma_onda(pcm: &Pcm, colonne: usize) -> Vec<f32> {
    if pcm.samples.is_empty() || colonne == 0 {
        return Vec::new();
    }
    let per_colonna = (pcm.samples.len() as f64 / colonne as f64).max(1.0);
    let mut onda = Vec::with_capacity(colonne);
    for c in 0..colonne {
        let da = (c as f64 * per_colonna) as usize;
        let a = (((c + 1) as f64 * per_colonna) as usize).min(pcm.samples.len());
        if da >= a {
            onda.push(0.0);
            continue;
        }
        let picco = pcm.samples[da..a].iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        onda.push(picco.min(1.0));
    }
    onda
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm_finto(secondi: f64) -> Pcm {
        let n = (16_000.0 * secondi) as usize;
        let samples = (0..n)
            .map(|i| {
                // Un impulso ogni secondo: la forma d'onda deve mostrarlo.
                if i % 16_000 < 800 {
                    0.9
                } else {
                    0.01
                }
            })
            .collect();
        Pcm { samples, sample_rate: 16_000 }
    }

    #[test]
    fn la_forma_d_onda_mostra_dove_si_parla() {
        let onda = forma_onda(&pcm_finto(4.0), 40);
        assert_eq!(onda.len(), 40);
        assert!(onda.iter().all(|v| (0.0..=1.0).contains(v)));
        // Le colonne con l'impulso sono alte, le altre no.
        let alte = onda.iter().filter(|v| **v > 0.5).count();
        assert!((1..=8).contains(&alte), "colonne alte: {alte}");
    }

    #[test]
    fn una_forma_d_onda_senza_campioni_non_va_in_panico() {
        assert!(forma_onda(&Pcm { samples: Vec::new(), sample_rate: 16_000 }, 100).is_empty());
        assert!(forma_onda(&pcm_finto(1.0), 0).is_empty());
    }

    #[test]
    fn piu_colonne_che_campioni_non_escono_dal_buffer() {
        let onda = forma_onda(&Pcm { samples: vec![0.5; 10], sample_rate: 16_000 }, 100);
        assert_eq!(onda.len(), 100);
    }
}
