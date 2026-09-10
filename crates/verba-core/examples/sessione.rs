//! Ripercorre, fuori dalla finestra, la sequenza esatta che l'applicazione
//! esegue su un file: apri, trascrivi, dimensioni, fotogramma, traccia audio.
//!
//! Serve a riprodurre un difetto dell'anteprima senza dover cliccare in una
//! finestra GTK. Ogni passo stampa cosa ha ottenuto, cosi' quello che rompe si
//! riconosce dal punto in cui la stampa si ferma.
//!
//! ```text
//! cargo run -p verba-core --example sessione -- prova.mp3
//! ```

use std::path::PathBuf;

use verba_core::caratteri::{self, Catalogo};
use verba_core::eventi::Progresso;
use verba_core::gpu;
use verba_core::impostazioni::Impostazioni;
use verba_core::modelli::Dimensione;
use verba_core::pipeline::{ConfigTrascrizione, PercorsiModelli};
use verba_core::sessione::Sessione;

fn main() -> anyhow::Result<()> {
    let percorso = PathBuf::from(
        std::env::args().nth(1).unwrap_or_else(|| "prova.mp3".to_string()),
    );
    let impostazioni = Impostazioni::carica();
    let progresso = Progresso::silenzioso();

    println!("--- apri {}", percorso.display());
    let mut s = Sessione::apri(&percorso, &progresso)?;
    let d = s.descrizione();
    println!("    modalita = {}, {}", d.modalita, d.riassunto);
    println!("    dimensioni prima della scena = {:?}", s.dimensioni());

    println!("--- traccia audio");
    let wav = s.traccia_audio();
    println!(
        "    WAV di {} byte, intestazione {:?}",
        wav.len(),
        std::str::from_utf8(&wav[0..4]).unwrap_or("?")
    );

    println!("--- trascrivi");
    let device = gpu::select(
        impostazioni.soglia_vram_mib(),
        impostazioni.solo_cpu(),
        impostazioni.gpu_preferita(),
    );
    let cartella = impostazioni.modelli();
    let cfg = ConfigTrascrizione {
        modelli: if cartella.is_dir() {
            PercorsiModelli::nella_cartella_con(&cartella, impostazioni.modello)
        } else {
            PercorsiModelli::predefiniti(Dimensione::default())
        },
        ..Default::default()
    };
    let catalogo = Catalogo::nuovo(&caratteri::cartelle_predefinite(), false);
    s.trascrivi(&cfg, &device, catalogo, &progresso)?;
    println!("    parole = {}", s.parole().len());

    println!("--- dimensioni");
    let (l, a) = s.dimensioni();
    println!("    {l} x {a}");

    println!("--- fotogramma");
    for t in [0.0f64, 0.5, 1.0, 2.5, 5.0] {
        match s.fotogramma(t) {
            Ok(byte) => {
                // I pixel opachi sono la sola prova che qualcosa sia stato
                // disegnato: un fotogramma tutto trasparente e uno rotto
                // pesano gli stessi byte.
                let opachi = byte.chunks_exact(4).filter(|p| p[3] > 0).count();
                println!(
                    "    t={t}: {} byte (attesi {}), {opachi} pixel opachi",
                    byte.len(),
                    l as usize * a as usize * 4
                );
            }
            Err(e) => println!("    t={t}: ERRORE: {e:#}"),
        }
    }

    println!("--- fatto");
    Ok(())
}
