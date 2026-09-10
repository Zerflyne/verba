//! Compila l'encoder video C++ (`cpp/encoder.cpp`) e lo lega a libavcodec /
//! libavformat / libavutil.
//!
//! Le librerie vengono cercate con `pkg-config`. Se gli header non sono
//! installati a livello di sistema si possono indicare a mano:
//!
//! ```sh
//! FFMPEG_INCLUDE_DIR=/percorso/include FFMPEG_LIB_DIR=/percorso/lib cargo build
//! ```

use std::env;
use std::path::PathBuf;

const LIBRERIE: [&str; 3] = ["libavcodec", "libavformat", "libavutil"];

fn main() {
    println!("cargo:rerun-if-changed=cpp/encoder.cpp");
    println!("cargo:rerun-if-changed=cpp/encoder.h");
    println!("cargo:rerun-if-changed=cpp/media.cpp");
    println!("cargo:rerun-if-changed=cpp/media.h");
    println!("cargo:rerun-if-env-changed=FFMPEG_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=FFMPEG_LIB_DIR");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .file("cpp/encoder.cpp")
        .file("cpp/media.cpp")
        .warnings(true);

    let include_manuale = env::var_os("FFMPEG_INCLUDE_DIR").map(PathBuf::from);
    let lib_manuale = env::var_os("FFMPEG_LIB_DIR").map(PathBuf::from);

    if let Some(dir) = &include_manuale {
        build.include(dir);
    }
    if let Some(dir) = &lib_manuale {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }

    // Con i percorsi forniti a mano si salta pkg-config: serve proprio quando
    // il .pc non c'e'.
    if include_manuale.is_none() || lib_manuale.is_none() {
        for lib in LIBRERIE {
            match pkg_config::Config::new().cargo_metadata(true).probe(lib) {
                Ok(info) => {
                    for dir in &info.include_paths {
                        build.include(dir);
                    }
                }
                Err(e) => {
                    if include_manuale.is_none() && lib_manuale.is_none() {
                        panic!(
                            "{lib} non trovato ({e}).\n\
                             Su Debian/Ubuntu: sudo apt install libavcodec-dev libavformat-dev libavutil-dev\n\
                             In alternativa indica i percorsi con FFMPEG_INCLUDE_DIR e FFMPEG_LIB_DIR."
                        );
                    }
                }
            }
        }
    }

    if lib_manuale.is_some() {
        for lib in LIBRERIE {
            println!("cargo:rustc-link-lib=dylib={}", lib.trim_start_matches("lib"));
        }
    }

    build.compile("subencoder");
}
