//! I comandi che la finestra puo' chiamare.
//!
//! Ognuno e' sottile: prende quello che arriva, chiama il motore, restituisce
//! il risultato. Gli errori tornano come stringhe gia' scritte per essere
//! lette da una persona — nella barra di stato, non in una finestra modale.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, State};

use verba_core::caratteri::Famiglia;
use verba_core::cartelle;
use verba_core::encoder::FormatoVideo;
use verba_core::eventi::{Evento, Progresso};
use verba_core::impostazioni::Impostazioni;
use verba_core::layout::Blocco;
use verba_core::modelli::{self, Dimensione};
use verba_core::pipeline::{ConfigTrascrizione, PercorsiModelli};
use verba_core::progetto::{self, Preset};
use verba_core::prompt::{self, PromptConfig};
use verba_core::sessione::{Descrizione, ParolaVista, Riepilogo, Sessione};
use verba_core::srt;
use verba_core::{align, gpu, onnx};

use crate::stato::Stato;

/// L'evento su cui la finestra si mette in ascolto.
const CANALE: &str = "verba://avanzamento";

type Esito<T> = Result<T, String>;

/// Trasforma un errore del motore in una riga da mostrare.
fn riga(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Il canale di avanzamento che rimanda tutto alla finestra.
fn progresso(app: &AppHandle, stato: &Stato) -> Progresso {
    stato.interruttore.riprendi();
    let finestra = app.clone();
    Progresso::con_interruttore(
        move |e: Evento| {
            let _ = finestra.emit(CANALE, &e);
        },
        stato.interruttore.clone(),
    )
}

// ------------------------------------------------------------ informazioni

#[derive(Serialize)]
pub struct Info {
    versione: &'static str,
    repository: &'static str,
    licenza: &'static str,
    /// Il provider ONNX effettivamente attivo, per la barra di stato.
    provider: String,
    dispositivo: String,
    cartella_modelli: String,
    cartella_dati: String,
}

#[tauri::command]
pub fn informazioni(stato: State<'_, Stato>) -> Info {
    let i = stato.impostazioni.lock().unwrap();
    let device = gpu::select(gpu::DEFAULT_MIN_VRAM_MIB, i.solo_cpu(), None);
    Info {
        versione: env!("CARGO_PKG_VERSION"),
        repository: env!("CARGO_PKG_REPOSITORY"),
        licenza: "MIT",
        provider: onnx::provider_attivo().etichetta().to_string(),
        dispositivo: device.describe(),
        cartella_modelli: i.modelli().display().to_string(),
        cartella_dati: cartelle::dati().display().to_string(),
    }
}

#[tauri::command]
pub fn impostazioni(stato: State<'_, Stato>) -> Impostazioni {
    stato.impostazioni.lock().unwrap().clone()
}

#[tauri::command]
pub fn salva_impostazioni(nuove: Impostazioni, stato: State<'_, Stato>) -> Esito<()> {
    {
        let mut i = stato.impostazioni.lock().unwrap();
        *i = nuove;
        i.salva().map_err(riga)?;
    }
    // La soglia di segnalazione si vede subito nella striscia di parole.
    let soglia = stato.impostazioni.lock().unwrap().soglia;
    if let Some(s) = stato.sessione.lock().unwrap().as_mut() {
        s.imposta_soglia(soglia);
    }
    Ok(())
}

// ----------------------------------------------------------------- modelli

#[derive(Serialize)]
pub struct ModelloVisto {
    id: &'static str,
    nome: &'static str,
    file: &'static str,
    spiegazione: &'static str,
    byte: u64,
    leggibile: String,
    presente: bool,
    /// Byte gia' scaricati di un file interrotto.
    ripresa: u64,
    si_scarica: bool,
    /// Il comando che lo produce, per i file che non si scaricano.
    comando: Option<&'static str>,
    /// Fa parte dei quattro file che servono con la dimensione scelta.
    in_uso: bool,
}

#[derive(Serialize)]
pub struct StatoModelli {
    cartella: String,
    modelli: Vec<ModelloVisto>,
    /// Vero quando c'e' tutto quello che serve per trascrivere.
    pronto: bool,
    /// Byte ancora da scaricare.
    da_scaricare: u64,
    da_scaricare_leggibile: String,
    /// I file che vanno prodotti a mano, con le istruzioni.
    a_mano: Vec<String>,
}

fn leggi_stato_modelli(cartella: &std::path::Path, d: Dimensione) -> StatoModelli {
    let in_uso: Vec<&'static str> = modelli::necessari(d).iter().map(|m| m.id).collect();
    let mancanti = modelli::mancanti(cartella, d);
    let da_scaricare: u64 =
        mancanti.iter().filter(|m| m.provenienza.si_scarica()).map(|m| m.byte).sum();

    StatoModelli {
        cartella: cartella.display().to_string(),
        modelli: modelli::stato_completo(cartella)
            .into_iter()
            .map(|s| {
                let m = s.modello;
                ModelloVisto {
                    id: m.id,
                    nome: m.nome,
                    file: m.file,
                    spiegazione: m.spiegazione,
                    byte: if s.presente { s.byte_su_disco } else { m.byte },
                    leggibile: cartelle::dimensione_leggibile(if s.presente {
                        s.byte_su_disco
                    } else {
                        m.byte
                    }),
                    presente: s.presente,
                    ripresa: s.ripresa,
                    si_scarica: m.provenienza.si_scarica(),
                    comando: match m.provenienza {
                        verba_core::modelli::Provenienza::DaEsportare { comando } => Some(comando),
                        _ => None,
                    },
                    in_uso: in_uso.contains(&m.id),
                }
            })
            .collect(),
        pronto: mancanti.is_empty(),
        da_scaricare,
        da_scaricare_leggibile: cartelle::dimensione_leggibile(da_scaricare),
        a_mano: mancanti
            .iter()
            .filter(|m| !m.provenienza.si_scarica())
            .map(|m| modelli::istruzioni_a_mano(m))
            .collect(),
    }
}

#[tauri::command]
pub fn stato_modelli(stato: State<'_, Stato>) -> StatoModelli {
    let i = stato.impostazioni.lock().unwrap();
    leggi_stato_modelli(&i.modelli(), i.modello)
}

#[tauri::command]
pub async fn scarica_modelli(app: AppHandle, stato: State<'_, Stato>) -> Esito<StatoModelli> {
    let (cartella, dimensione) = {
        let i = stato.impostazioni.lock().unwrap();
        (i.modelli(), i.modello)
    };
    let p = progresso(&app, &stato);
    let dove = cartella.clone();
    // Tre gigabyte non passano dal filo dell'interfaccia.
    tauri::async_runtime::spawn_blocking(move || modelli::scarica_mancanti(&dove, dimensione, &p))
        .await
        .map_err(riga)?
        .map_err(riga)?;
    Ok(leggi_stato_modelli(&cartella, dimensione))
}

// ------------------------------------------------------------------ il file

#[tauri::command]
pub async fn apri(percorso: String, app: AppHandle, stato: State<'_, Stato>) -> Esito<Descrizione> {
    let p = progresso(&app, &stato);
    let soglia = stato.impostazioni.lock().unwrap().soglia;
    let sessione = tauri::async_runtime::spawn_blocking(move || {
        Sessione::apri(&PathBuf::from(percorso), &p)
    })
    .await
    .map_err(riga)?
    .map_err(riga)?;

    let descrizione = sessione.descrizione();
    let mut sessione = sessione;
    sessione.imposta_soglia(soglia);
    *stato.sessione.lock().unwrap() = Some(sessione);
    Ok(descrizione)
}

#[tauri::command]
pub fn chiudi(stato: State<'_, Stato>) {
    *stato.sessione.lock().unwrap() = None;
}

#[tauri::command]
pub fn descrizione(stato: State<'_, Stato>) -> Option<Descrizione> {
    stato.sessione.lock().unwrap().as_ref().map(|s| s.descrizione())
}

// ------------------------------------------------------------ trascrizione

#[tauri::command]
pub async fn trascrivi(app: AppHandle, stato: State<'_, Stato>) -> Esito<Riepilogo> {
    if stato.occupato.swap(true, Ordering::SeqCst) {
        return Err("c'e' gia' un'elaborazione in corso".into());
    }
    let esito = trascrivi_davvero(app, &stato).await;
    stato.occupato.store(false, Ordering::SeqCst);
    esito
}

async fn trascrivi_davvero(app: AppHandle, stato: &State<'_, Stato>) -> Esito<Riepilogo> {
    onnx::assicura_libreria().map_err(riga)?;

    let impostazioni = stato.impostazioni.lock().unwrap().clone();
    let percorsi = PercorsiModelli::nella_cartella_con(impostazioni.modelli(), impostazioni.modello);
    let mancanti = percorsi.mancanti();
    if !mancanti.is_empty() {
        return Err(format!(
            "mancano {} file dei modelli. Vai in Impostazioni e scaricali.",
            mancanti.len()
        ));
    }

    let initial_prompt = prompt::build(&PromptConfig {
        csv: impostazioni.termini.clone(),
        max_chars: prompt::DEFAULT_MAX_CHARS,
        ..Default::default()
    })
    .map_err(riga)?;

    let thread = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 32);
    let device = gpu::select(gpu::DEFAULT_MIN_VRAM_MIB, impostazioni.solo_cpu(), None);

    let cfg = ConfigTrascrizione {
        modelli: percorsi,
        whisper: verba_core::transcribe::WhisperConfig {
            language: impostazioni.lingua.clone(),
            threads: thread as i32,
            initial_prompt,
            ..Default::default()
        },
        allineamento: align::AlignConfig::default(),
        thread,
        ..Default::default()
    };

    let p = progresso(&app, stato);
    let catalogo = stato.catalogo();

    // La sessione viene tolta dallo stato per la durata dell'operazione e
    // rimessa alla fine: cosi' il lock non resta preso mentre si lavora, e i
    // comandi che leggono (l'anteprima, la striscia) non si bloccano.
    let mut sessione = stato
        .sessione
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| "non c'e' nessun file aperto".to_string())?;

    let (sessione, esito) = tauri::async_runtime::spawn_blocking(move || {
        let esito = sessione.trascrivi(&cfg, &device, catalogo, &p);
        (sessione, esito)
    })
    .await
    .map_err(riga)?;

    let riepilogo = sessione.riepilogo().cloned();
    *stato.sessione.lock().unwrap() = Some(sessione);
    esito.map_err(riga)?;
    riepilogo.ok_or_else(|| "trascrizione senza riepilogo".to_string())
}

#[tauri::command]
pub fn annulla(stato: State<'_, Stato>) {
    stato.interruttore.annulla();
}

// ----------------------------------------------------------------- aspetto

#[tauri::command]
pub fn preset_corrente(stato: State<'_, Stato>) -> Option<Preset> {
    stato.sessione.lock().unwrap().as_ref().map(|s| s.preset().clone())
}

#[tauri::command]
pub fn preset_di_serie() -> Vec<Preset> {
    progetto::di_serie()
}

#[tauri::command]
pub fn preset_carica(percorso: String) -> Esito<Preset> {
    Preset::carica(&PathBuf::from(percorso)).map_err(riga)
}

#[tauri::command]
pub fn preset_salva(percorso: String, preset: Preset) -> Esito<()> {
    preset.salva(&PathBuf::from(percorso)).map_err(riga)
}

/// Applica un aspetto e ricompone. Non rilancia alcun modello.
#[tauri::command]
pub fn applica_aspetto(preset: Preset, stato: State<'_, Stato>) -> Esito<Option<String>> {
    let catalogo = stato.catalogo();
    let mut guardia = stato.sessione.lock().unwrap();
    let s = guardia.as_mut().ok_or_else(|| "non c'e' nessun file aperto".to_string())?;
    s.applica(preset, catalogo, &Progresso::silenzioso()).map_err(riga)?;
    Ok(s.avviso_carattere().map(str::to_string))
}

// --------------------------------------------------------------- anteprima

#[tauri::command]
pub fn dimensioni(stato: State<'_, Stato>) -> Option<(u32, u32)> {
    stato.sessione.lock().unwrap().as_ref().map(|s| s.dimensioni())
}

/// Il fotogramma al tempo `t`, come byte RGBA.
///
/// **E' lo stesso codice che produce l'export.** La finestra lo mette in una
/// `ImageData` e lo disegna su un canvas; nessuna impaginazione avviene in
/// JavaScript, cosi' anteprima ed export non possono divergere.
#[tauri::command]
pub fn fotogramma(t: f64, stato: State<'_, Stato>) -> Esito<Response> {
    let mut guardia = stato.sessione.lock().unwrap();
    let s = guardia.as_mut().ok_or_else(|| "non c'e' nessun file aperto".to_string())?;
    let pixel = s.fotogramma(t).map_err(riga)?;
    Ok(Response::new(pixel.to_vec()))
}

#[tauri::command]
pub fn onda(stato: State<'_, Stato>) -> Vec<f32> {
    stato.sessione.lock().unwrap().as_ref().map(|s| s.onda().to_vec()).unwrap_or_default()
}

#[tauri::command]
pub fn parole(stato: State<'_, Stato>) -> Vec<ParolaVista> {
    stato.sessione.lock().unwrap().as_ref().map(|s| s.parole()).unwrap_or_default()
}

#[tauri::command]
pub fn finestra(t: f64, quante: usize, stato: State<'_, Stato>) -> Vec<ParolaVista> {
    stato
        .sessione
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.finestra(t, quante))
        .unwrap_or_default()
}

// -------------------------------------------------------------- caratteri

#[tauri::command]
pub fn caratteri(stato: State<'_, Stato>) -> Vec<Famiglia> {
    stato.catalogo().famiglie().to_vec()
}

/// Aggiunge un `.ttf` o `.otf` che l'utente si e' scaricato.
///
/// Il file resta dov'e': viene ricordato il percorso, e ricaricato a ogni
/// avvio. Verba non copia i caratteri altrove e non li cancella.
#[tauri::command]
pub fn aggiungi_carattere(percorso: String, stato: State<'_, Stato>) -> Esito<Vec<Famiglia>> {
    let file = PathBuf::from(percorso);
    // Prima si prova a leggerlo: se non e' un carattere valido, e' meglio
    // dirlo adesso che ritrovarselo scartato in silenzio a ogni avvio.
    let mut prova = verba_core::caratteri::Catalogo::incorporato();
    let nome = prova.aggiungi_file(&file).map_err(riga)?;

    {
        let mut i = stato.impostazioni.lock().unwrap();
        if i.aggiungi_carattere(&file) {
            i.salva().map_err(riga)?;
        }
    }
    tracing::info!(file = %file.display(), famiglia = %nome, "carattere aggiunto");
    Ok(stato.catalogo().famiglie().to_vec())
}

// ---------------------------------------------------------------- esporta

#[derive(Serialize)]
pub struct FormatoVisto {
    id: String,
    etichetta: &'static str,
    descrizione: &'static str,
    estensione: &'static str,
    alfa: bool,
}

#[derive(Serialize, Deserialize)]
pub struct FormatiDisponibili {
    video: Vec<FormatoVisto>,
    testo: Vec<FormatoTesto>,
}

#[derive(Serialize, Deserialize)]
pub struct FormatoTesto {
    id: &'static str,
    etichetta: &'static str,
    descrizione: &'static str,
    estensione: &'static str,
}

const TESTO: [FormatoTesto; 4] = [
    FormatoTesto {
        id: "srt",
        etichetta: "Sottotitoli",
        descrizione: "Un blocco per riga mostrata. Il formato che legge chiunque.",
        estensione: "srt",
    },
    FormatoTesto {
        id: "vtt",
        etichetta: "Sottotitoli WebVTT",
        descrizione: "Come l'SRT, nel formato che vogliono i lettori video del web.",
        estensione: "vtt",
    },
    FormatoTesto {
        id: "json",
        etichetta: "Parola per parola",
        descrizione: "Testo, inizio, fine e confidenza di ogni singola parola.",
        estensione: "json",
    },
    FormatoTesto {
        id: "txt",
        etichetta: "Solo testo",
        descrizione: "La trascrizione senza tempi, una battuta per riga.",
        estensione: "txt",
    },
];

fn id_formato(f: FormatoVideo) -> String {
    match f {
        FormatoVideo::H264 => "h264",
        FormatoVideo::Prores422 => "prores422",
        FormatoVideo::Prores4444 => "prores4444",
        FormatoVideo::Vp9Alpha => "vp9",
    }
    .to_string()
}

fn formato_da_id(id: &str) -> Option<FormatoVideo> {
    FormatoVideo::TUTTI.into_iter().find(|f| id_formato(*f) == id)
}

/// I formati che ha senso proporre per il file aperto.
///
/// In modalita' audio quelli video non vengono mostrati disabilitati: non
/// vengono mostrati.
#[tauri::command]
pub fn formati(stato: State<'_, Stato>) -> FormatiDisponibili {
    let video = stato
        .sessione
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.formati_video())
        .unwrap_or_default()
        .into_iter()
        .map(|f| FormatoVisto {
            id: id_formato(f),
            etichetta: f.etichetta(),
            descrizione: f.descrizione(),
            estensione: f.estensione(),
            alfa: f.ha_alfa(),
        })
        .collect();
    FormatiDisponibili { video, testo: TESTO.into_iter().collect() }
}

#[tauri::command]
pub fn nome_proposto(formato: String, stato: State<'_, Stato>) -> Esito<String> {
    let guardia = stato.sessione.lock().unwrap();
    let s = guardia.as_ref().ok_or_else(|| "non c'e' nessun file aperto".to_string())?;

    let proposto = match formato_da_id(&formato) {
        Some(f) => s.nome_proposto(f),
        None => {
            let ext = TESTO
                .iter()
                .find(|t| t.id == formato)
                .map(|t| t.estensione)
                .ok_or_else(|| format!("formato «{formato}» sconosciuto"))?;
            let radice = s
                .percorso()
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or("sottotitoli")
                .to_string();
            let nome = format!("{radice}.{ext}");
            match s.percorso().parent() {
                Some(d) if !d.as_os_str().is_empty() => d.join(nome),
                _ => PathBuf::from(nome),
            }
        }
    };

    // La cartella scelta nelle impostazioni, se ce n'e' una.
    let i = stato.impostazioni.lock().unwrap();
    let proposto = match (&i.cartella_export, proposto.file_name()) {
        (Some(dir), Some(nome)) => dir.join(nome),
        _ => proposto,
    };
    Ok(proposto.display().to_string())
}

#[derive(Serialize)]
pub struct EsitoExport {
    percorso: String,
    fotogrammi: u64,
    secondi: f64,
}

#[tauri::command]
pub async fn esporta(
    formato: String,
    percorso: String,
    qualita: u32,
    app: AppHandle,
    stato: State<'_, Stato>,
) -> Esito<EsitoExport> {
    let f = formato_da_id(&formato).ok_or_else(|| format!("formato «{formato}» sconosciuto"))?;
    if stato.occupato.swap(true, Ordering::SeqCst) {
        return Err("c'e' gia' un'elaborazione in corso".into());
    }
    let esito = esporta_davvero(f, percorso, qualita, app, &stato).await;
    stato.occupato.store(false, Ordering::SeqCst);
    esito
}

async fn esporta_davvero(
    formato: FormatoVideo,
    percorso: String,
    qualita: u32,
    app: AppHandle,
    stato: &State<'_, Stato>,
) -> Esito<EsitoExport> {
    let p = progresso(&app, stato);
    let thread = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 32);
    let destinazione = PathBuf::from(&percorso);

    let mut sessione = stato
        .sessione
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| "non c'e' nessun file aperto".to_string())?;

    let (sessione, esito) = tauri::async_runtime::spawn_blocking(move || {
        let esito = sessione.esporta(formato, &destinazione, qualita, thread, &p);
        (sessione, esito)
    })
    .await
    .map_err(riga)?;

    *stato.sessione.lock().unwrap() = Some(sessione);
    let stat = esito.map_err(riga)?;

    // L'ultima scelta si ricorda, come dice la spec.
    {
        let mut i = stato.impostazioni.lock().unwrap();
        i.ultimo_formato = Some(id_formato(formato));
        let _ = i.salva();
    }
    Ok(EsitoExport { percorso, fotogrammi: stat.fotogrammi, secondi: stat.secondi })
}

/// Scrive un file di sottotitoli. Il formato lo dice l'estensione.
#[tauri::command]
pub fn esporta_testo(percorso: String, stato: State<'_, Stato>) -> Esito<String> {
    let destinazione = PathBuf::from(&percorso);
    let guardia = stato.sessione.lock().unwrap();
    let s = guardia.as_ref().ok_or_else(|| "non c'e' nessun file aperto".to_string())?;
    let blocchi: &[Blocco] = s.blocchi();

    let estensione = destinazione
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let contenuto = match estensione.as_str() {
        "srt" => srt::render(&srt::cues_da_blocchi(blocchi)),
        "vtt" => srt::render_vtt(&srt::cues_da_blocchi(blocchi)),
        "txt" => srt::render_testo(&srt::cues_da_blocchi(blocchi)),
        "json" => srt::render_json(s.parole_grezze()).map_err(riga)?,
        altro => return Err(format!("«.{altro}» non e' un formato di sottotitoli")),
    };
    std::fs::write(&destinazione, contenuto).map_err(riga)?;
    Ok(percorso)
}

#[tauri::command]
pub fn mostra_nella_cartella(percorso: String, app: AppHandle) -> Esito<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().reveal_item_in_dir(PathBuf::from(percorso)).map_err(riga)
}
