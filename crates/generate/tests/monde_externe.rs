//! Génération d'un monde natif qui vit hors du dépôt (ADR 0048 : The Planet Crafter).
//!
//! Test ignoré : il lit le `world.toml` désigné par `RAFTUP_MONDE_NATIF`, puis génère
//! plusieurs graines à un et à deux joueurs avec les options par défaut. Si
//! `RAFTUP_MONDE_SORTIE` est donnée, la première seed y est écrite (`.archipelago`), pour
//! l'héberger ensuite avec `raftup-server`.
//!
//! ```text
//! $env:RAFTUP_MONDE_NATIF = "D:\planet-crafter\monde\world.toml"
//! cargo test -p generate --test monde_externe -- --ignored --nocapture
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use generate::{generate, GameOptions, GenerateRequest, NativeOptions, PlayerRequest};

#[test]
#[ignore = "monde natif hors du dépôt : RAFTUP_MONDE_NATIF"]
fn monde_natif_externe_se_genere() {
    let chemin = std::env::var("RAFTUP_MONDE_NATIF").expect("RAFTUP_MONDE_NATIF manquante");
    let texte = std::fs::read_to_string(&chemin).unwrap();
    // `Arc` : les options de chaque joueur gardent une référence partagée au même monde.
    let world = Arc::new(worlds_native::parse_world(&texte).unwrap());
    let defaut = || GameOptions::Native(NativeOptions::new(world.clone()));

    for seed in 1..=5u64 {
        for joueurs in [1usize, 2] {
            let players = (0..joueurs)
                .map(|i| PlayerRequest::new(format!("Joueur{}", i + 1), defaut()))
                .collect();
            let sortie = generate(&GenerateRequest::new(seed, players))
                .unwrap_or_else(|e| panic!("graine {seed}, {joueurs} joueur(s) : {e}"));
            println!(
                "graine {seed}, {joueurs} joueur(s) : seed de {} octets",
                sortie.archipelago.len()
            );
            if seed == 1 && joueurs == 1 {
                if let Ok(dossier) = std::env::var("RAFTUP_MONDE_SORTIE") {
                    let fichier =
                        std::path::Path::new(&dossier).join("AP_monde_externe.archipelago");
                    std::fs::write(&fichier, &sortie.archipelago).unwrap();
                    println!("écrit : {}", fichier.display());
                }
            }
        }
    }
}
