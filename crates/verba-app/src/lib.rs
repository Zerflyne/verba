//! Il guscio dell'applicazione: una finestra sopra `verba-core`.
//!
//! Qui non c'e' logica. Ogni comando prende quello che arriva dalla finestra,
//! lo passa al motore e restituisce il risultato; lo stato di un lavoro aperto
//! sta in [`verba_core::sessione::Sessione`], che non sa che Tauri esiste.
//!
//! Due regole che questo file rispetta e che conviene non violare:
//!
//! 1. **L'anteprima esce dallo stesso codice dell'export.** Il comando
//!    [`fotogramma`] chiede alla sessione il fotogramma al tempo `t` e lo
//!    manda alla finestra come byte RGBA; la finestra lo mostra e basta. Non
//!    esiste un'impaginazione in JavaScript.
//! 2. **Le operazioni lunghe non stanno sul filo dell'interfaccia.** Vanno in
//!    `spawn_blocking`, e l'avanzamento torna indietro come eventi.

mod comandi;
mod stato;

pub use stato::Stato;

/// Costruisce e avvia la finestra.
pub fn avvia() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("verba_app=info,verba_core=info,warn")),
        )
        .with_target(false)
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Stato::nuovo())
        .invoke_handler(tauri::generate_handler![
            comandi::informazioni,
            comandi::impostazioni,
            comandi::salva_impostazioni,
            comandi::stato_modelli,
            comandi::scarica_modelli,
            comandi::gpu_disponibili,
            comandi::termini,
            comandi::termini_salva,
            comandi::problema,
            comandi::file_da_aprire,
            comandi::apri,
            comandi::chiudi,
            comandi::descrizione,
            comandi::trascrivi,
            comandi::annulla,
            comandi::preset_corrente,
            comandi::preset_di_serie,
            comandi::preset_carica,
            comandi::preset_salva,
            comandi::applica_aspetto,
            comandi::dimensioni,
            comandi::fotogramma,
            comandi::onda,
            comandi::traccia_audio,
            comandi::parole,
            comandi::finestra,
            comandi::caratteri,
            comandi::aggiungi_carattere,
            comandi::formati,
            comandi::nome_proposto,
            comandi::esporta,
            comandi::esporta_testo,
            comandi::mostra_nella_cartella,
        ])
        .run(tauri::generate_context!())
        .expect("avvio della finestra di Verba");
}
