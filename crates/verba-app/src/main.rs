// Su Windows la finestra della console non deve comparire dietro
// l'applicazione. In debug resta, perche' li' i log servono.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    verba_app::avvia();
}
