//! Exemple MANUEL : installe le vrai runtime de génération (Python, Archipelago, wheels)
//! avec le vrai réseau, dans le dossier donné en argument. Jamais lancé par les tests.
//!
//! `cargo run -p launcher-core --example setup_generator -- <dossier>`

use std::path::PathBuf;

use launcher_core::generation::{ensure_runtime, pins, runtime_status, RuntimeStatus};
use launcher_core::{Downloader, HttpFetcher, Paths};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage : setup_generator <dossier>")?,
    );
    let paths = Paths::under(&root);
    let pins = pins()?;
    if let RuntimeStatus::Missing { download_bytes } = runtime_status(&paths, &pins) {
        println!("À télécharger (plafond) : {download_bytes} octets");
    }
    let downloader = Downloader::new(paths.downloads_dir(), Box::new(HttpFetcher::new()));
    let runtime = ensure_runtime(&paths, &pins, &downloader, &mut |step, bytes| {
        println!("{step:?} : {bytes} octets");
    })?;
    println!("Prêt : {runtime:?}");
    Ok(())
}
