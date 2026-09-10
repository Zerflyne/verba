//! Dove sono i modelli, e come farli arrivare.
//!
//! Due responsabilita': risolvere i quattro percorsi prima di una
//! trascrizione, e il comando `verba modelli` che li elenca, li scarica e li
//! rimuove.

use std::path::PathBuf;

use anyhow::{bail, Result};
use verba_core::cartelle;
use verba_core::eventi::Progresso;
use verba_core::modelli::{self, Dimensione, Modello, Provenienza};
use verba_core::pipeline::PercorsiModelli;

use crate::opzioni::Comuni;

/// La cartella dei modelli secondo le opzioni date.
pub fn cartella(comuni: &Comuni) -> PathBuf {
    comuni.cartella_modelli.clone().unwrap_or_else(cartelle::modelli)
}

/// I quattro percorsi, scaricando quello che manca se e' stato chiesto.
///
/// L'ordine e' quello che ci si aspetta: un percorso scritto a mano vince su
/// tutto, poi la cartella indicata, poi la cartella dati, poi `./models` se si
/// sta lavorando dentro il repository.
pub fn percorsi(comuni: &Comuni, progresso: &Progresso) -> Result<PercorsiModelli> {
    let dimensione: Dimensione = comuni.modello.into();
    let mut p = match &comuni.cartella_modelli {
        Some(c) => PercorsiModelli::nella_cartella_con(c, dimensione),
        None => PercorsiModelli::predefiniti(dimensione),
    };

    // Se manca qualcosa e lo si e' chiesto, si scarica prima di sostituire i
    // percorsi indicati a mano: quelli non li tocca nessuno.
    if comuni.scarica_modelli {
        let dove = cartella(comuni);
        let a_mano = modelli::scarica_mancanti(&dove, dimensione, progresso)?;
        for m in a_mano {
            tracing::warn!("{} non si scarica: {}", m.nome, modelli::istruzioni_a_mano(m));
        }
        p = PercorsiModelli::nella_cartella_con(&dove, dimensione);
    }

    for (campo, scelto) in [
        (&mut p.whisper, &comuni.modello_whisper),
        (&mut p.segmentazione, &comuni.modello_segmentazione),
        (&mut p.allineamento, &comuni.modello_allineamento),
        (&mut p.vocabolario, &comuni.vocabolario_allineamento),
    ] {
        if let Some(s) = scelto {
            *campo = s.clone();
        }
    }

    let mancanti = p.mancanti();
    if !mancanti.is_empty() {
        let elenco: Vec<String> =
            mancanti.iter().map(|x| format!("  {}", x.display())).collect();
        bail!(
            "mancano {} file dei modelli:\n{}\n\n\
             Per averli:  verba modelli --scarica --modello {}\n\
             Oppure aggiungi --scarica-modelli a questo comando.",
            mancanti.len(),
            elenco.join("\n"),
            dimensione.nome()
        );
    }
    Ok(p)
}

/// `verba modelli`: cosa c'e', cosa manca, quanto occupa.
pub fn elenca(cartella: &std::path::Path, dimensione: Dimensione) {
    println!("Cartella dei modelli: {}\n", cartella.display());

    let necessari: Vec<&'static str> =
        modelli::necessari(dimensione).iter().map(|m| m.id).collect();
    let stato = modelli::stato_completo(cartella);
    let mut occupato = 0u64;

    for s in &stato {
        let m = s.modello;
        let serve = necessari.contains(&m.id);
        let segno = match (s.presente, serve) {
            (true, _) => "✓",
            (false, true) => "·",
            (false, false) => " ",
        };
        let peso = if s.presente {
            cartelle::dimensione_leggibile(s.byte_su_disco)
        } else if s.ripresa > 0 {
            format!("{} scaricati", cartelle::dimensione_leggibile(s.ripresa))
        } else {
            cartelle::dimensione_leggibile(m.byte)
        };
        if s.presente {
            occupato += s.byte_su_disco;
        }
        println!(
            "{segno} {:<14} {:<32} {:>9}{}",
            m.id,
            m.nome,
            peso,
            if serve { "   (in uso)" } else { "" }
        );
        println!("  {:<14} {}", "", m.spiegazione);
        if let Provenienza::DaEsportare { comando } = m.provenienza {
            if !s.presente {
                println!("  {:<14} non si scarica:  {comando}", "");
            }
        }
        println!();
    }

    println!("Occupati: {}", cartelle::dimensione_leggibile(occupato));
    let mancanti = modelli::mancanti(cartella, dimensione);
    if mancanti.is_empty() {
        println!("Tutto quello che serve per «{}» e' a posto.", dimensione.nome());
    } else {
        let (scaricabili, a_mano): (Vec<&&'static Modello>, Vec<&&'static Modello>) =
            mancanti.iter().partition(|m| m.provenienza.si_scarica());
        let quanti = |n: usize| if n == 1 { "Manca 1 file" } else { "Mancano file" };
        if !scaricabili.is_empty() {
            let byte: u64 = scaricabili.iter().map(|m| m.byte).sum();
            println!(
                "{} da scaricare per «{}»: {}.",
                quanti(scaricabili.len()),
                dimensione.nome(),
                cartelle::dimensione_leggibile(byte)
            );
            println!("  verba modelli --scarica --modello {}", dimensione.nome());
        }
        for m in a_mano {
            println!("{} non si scarica.\n{}", m.nome, modelli::istruzioni_a_mano(m));
        }
    }
    println!("\nDimensioni disponibili:");
    for d in Dimensione::TUTTE {
        println!("  {:<10} {}", d.nome(), d.compromesso());
    }
}

/// `verba modelli --verifica`: ricalcola l'impronta di quello che c'e'.
pub fn verifica(cartella: &std::path::Path) -> Result<()> {
    let mut guasti = 0;
    for s in modelli::stato_completo(cartella) {
        if !s.presente {
            continue;
        }
        let m = s.modello;
        match modelli::verifica(m, cartella)? {
            true => match m.provenienza {
                Provenienza::Scaricabile { sha256, .. } if !sha256.is_empty() => {
                    println!("✓ {:<14} impronta corretta", m.id)
                }
                _ => println!("· {:<14} nessuna impronta da confrontare", m.id),
            },
            false => {
                guasti += 1;
                println!("✕ {:<14} NON corrisponde: il file e' danneggiato", m.id);
            }
        }
    }
    if guasti > 0 {
        bail!(
            "{guasti} file danneggiati. Rimuovili con `verba modelli --rimuovi ID` e \
             riscaricali."
        );
    }
    Ok(())
}

/// `verba modelli --rimuovi ID`.
pub fn rimuovi(cartella: &std::path::Path, id: &[String]) -> Result<()> {
    for nome in id {
        let Some(m): Option<&'static Modello> = modelli::per_id(nome) else {
            bail!(
                "«{nome}» non e' un modello del catalogo. Sono: {}.",
                elenco_id().join(", ")
            );
        };
        if modelli::rimuovi(m, cartella)? {
            println!("rimosso {} ({})", m.nome, m.file);
        } else {
            println!("{} non c'era", m.nome);
        }
    }
    Ok(())
}

fn elenco_id() -> Vec<&'static str> {
    modelli::CATALOGO
        .iter()
        .map(|m| m.id)
        .chain(std::iter::once(modelli::ALLINEATORE.id))
        .collect()
}
