//! I comandi che si limitano a dire cosa c'e': caratteri, preset, formati.

use std::path::PathBuf;

use verba_core::caratteri::{self, Catalogo};
use verba_core::encoder::FormatoVideo;
use verba_core::progetto::{self, FormatoPreset, Preset};
use verba_core::render::Evidenziazione;

/// I caratteri disponibili con i loro pesi.
pub fn caratteri(cartelle_extra: &[PathBuf], di_sistema: bool) {
    let mut cartelle = caratteri::cartelle_predefinite();
    cartelle.extend(cartelle_extra.iter().cloned());
    let c = Catalogo::nuovo(&cartelle, di_sistema);

    println!("Caratteri disponibili ({}):\n", c.famiglie().len());
    for f in c.famiglie() {
        let pesi: Vec<String> = f.pesi.iter().map(|p| p.to_string()).collect();
        println!(
            "  {:<28} {:<24} {}",
            f.nome,
            pesi.join(" "),
            if f.di_serie { "di serie" } else { "" }
        );
    }
    if !di_sistema {
        println!("\nCon --caratteri-di-sistema si aggiungono quelli installati sulla macchina.");
    }
    println!(
        "Per usarne un altro: scaricare il .ttf e passarlo con --font FILE, oppure metterlo in\n\
         una cartella e passarla con --cartella-caratteri CARTELLA."
    );
}

/// I preset di serie.
pub fn preset() {
    println!("Preset di serie:\n");
    for p in progetto::di_serie() {
        println!("  {:<14} {}", p.nome.to_lowercase(), descrivi(&p));
    }
    println!("\nSi usano con --preset-di-serie NOME, oppure si salva il proprio con");
    println!("--salva-preset FILE e lo si ricarica con --preset FILE.");
}

/// I formati video di uscita, con il comando che li produce.
pub fn formati() {
    println!("Formati video:\n");
    for f in FormatoVideo::TUTTI {
        let comando = if f.ha_alfa() { "verba overlay" } else { "verba rendi" };
        println!("  {:<14} .{:<5} {}", comando, f.estensione(), f.etichetta());
        println!("  {:<21} {}\n", "", f.descrizione());
    }
    println!("Il codec si sceglie con l'estensione di --out.\n");
    println!("Formati di testo (verba trascrivi):\n");
    for (ext, cosa) in [
        ("srt", "un blocco per riga mostrata"),
        ("vtt", "come l'SRT, per il web"),
        ("json", "parola per parola: testo, inizio, fine, confidenza"),
        ("txt", "solo il testo, una battuta per riga"),
    ] {
        println!("  .{ext:<5} {cosa}");
    }
    println!("\n`verba rendi` imprime i sottotitoli sul filmato e richiede un file video di");
    println!("partenza; da un file audio si puo' produrre solo un overlay.");
}

/// Una riga che descrive il preset, per l'elenco.
fn descrivi(p: &Preset) -> String {
    let formato = match p.posizione.formato {
        FormatoPreset::Verticale => "9:16",
        FormatoPreset::Orizzontale => "16:9",
        FormatoPreset::DalSorgente => "dal sorgente",
    };
    let forma = match p.evidenziazione.forma {
        Evidenziazione::Rettangolo => "rettangolo",
        Evidenziazione::Sottolineatura => "sottolineatura",
        Evidenziazione::SoloColore => "solo colore",
        Evidenziazione::Nessuna => "nessuna evidenziazione",
    };
    format!("{formato}, {} riga/e, {}, {forma}", p.posizione.righe_max, p.testo.carattere)
}
