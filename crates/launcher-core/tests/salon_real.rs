//! Test RÉEL du salon de préparation (ADR 0044) : les YAML écrits par
//! `launcher_core::crew` (section de jeu vide, options par défaut de l'apworld) sont
//! acceptés par le vrai `Generate.py` d'Archipelago 0.6.8 et donnent une seed.
//!
//! Ignoré par défaut ; il ne télécharge rien. Il lui faut un runtime installé
//! (`cargo run -p launcher-core --example setup_generator -- <dossier>`, racine dans
//! `RAFTUP_REAL_RUNTIME`) et, pour Slime Rancher, l'apworld téléchargée chez l'auteur
//! (chemin du fichier dans `RAFTUP_SLIME_APWORLD`, sha256 du profil vérifié avant) :
//!
//! `cargo test -p launcher-core --test salon_real -- --ignored --nocapture --test-threads=1`

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::Duration;

use launcher_core::crew::{player_yaml, write_yamls, CrewMember};
use launcher_core::generation::{
    generate, pins, runtime_status, GenerateOutput, GenerateRequest, PythonBackend, RuntimeStatus,
};
use launcher_core::Paths;
use py_bridge::CancelToken;

/// Le générateur Python du runtime installé et ses chemins, ou `None` si la variable manque.
///
/// Les chemins sont ceux du runtime : `generate` ne pose des apworlds que dans un
/// Archipelago rangé sous ce dossier de données (sinon `UnmanagedArchipelago`).
fn backend() -> Option<(Paths, PythonBackend)> {
    let root = PathBuf::from(std::env::var_os("RAFTUP_REAL_RUNTIME").or_else(|| {
        eprintln!("RAFTUP_REAL_RUNTIME absente : test réel sauté");
        None
    })?);
    let paths = Paths::under(&root);
    match runtime_status(&paths, &pins().unwrap()) {
        RuntimeStatus::Ready(runtime) => Some((paths, PythonBackend::new(runtime))),
        other => panic!("runtime non prêt dans {} : {other:?}", root.display()),
    }
}

fn member(name: &str, ap_game: &str) -> CrewMember {
    CrewMember {
        name: name.into(),
        ap_game: ap_game.into(),
    }
}

/// Écrit les YAML de l'équipage comme le salon, puis génère avec le vrai Archipelago.
fn generate_crew(
    paths: &Paths,
    backend: &mut PythonBackend,
    work: &Path,
    crew: &[CrewMember],
    apworlds: Vec<PathBuf>,
) -> GenerateOutput {
    let player_files = write_yamls(&work.join("players"), crew).unwrap();
    let req = GenerateRequest {
        player_files,
        apworlds,
        seed: Some(4242),
        timeout: Duration::from_secs(300),
        ..GenerateRequest::default()
    };
    let (tx, rx) = channel();
    let result = generate(paths, backend, &req, &CancelToken::new(), tx);
    let logs: Vec<String> = rx.try_iter().collect();
    match result {
        Ok(out) => out,
        Err(e) => panic!(
            "génération refusée : {e:?}\n--- journal ---\n{}",
            logs.join("\n")
        ),
    }
}

/// Un joueur APQuest (monde de base d'Archipelago 0.6.8), section `"APQuest": {}`.
#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn salon_apquest_section_vide() {
    let Some((paths, mut backend)) = backend() else {
        return;
    };
    let crew = [member("Zoé", "APQuest")];
    // Le YAML est bien celui que le salon écrit : section de jeu vide.
    assert!(player_yaml(&crew[0]).contains("\"APQuest\": {}"));

    let work = tempfile::tempdir().unwrap();
    let out = generate_crew(&paths, &mut backend, work.path(), &crew, Vec::new());
    assert!(out.archipelago_file.is_file());
    assert_eq!(out.summary.players, 1);
    assert_eq!(out.summary.slots[0].name, "Zoé");
    assert_eq!(out.summary.slots[0].game, "APQuest");
}

/// Deux joueurs Slime Rancher (apworld de l'auteur), sections `"Slime Rancher": {}`.
/// Le second pseudo contient des guillemets : l'échappement YAML de `crew` est relu tel quel.
#[test]
#[ignore = "demande RAFTUP_REAL_RUNTIME et RAFTUP_SLIME_APWORLD"]
fn salon_deux_joueurs_slime_rancher() {
    let Some((paths, mut backend)) = backend() else {
        return;
    };
    let Some(apworld) = std::env::var_os("RAFTUP_SLIME_APWORLD").map(PathBuf::from) else {
        eprintln!("RAFTUP_SLIME_APWORLD absente : test Slime Rancher sauté");
        return;
    };
    let crew = [
        member("Léa", "Slime Rancher"),
        member("Max \"M\"", "Slime Rancher"),
    ];
    let work = tempfile::tempdir().unwrap();
    let out = generate_crew(&paths, &mut backend, work.path(), &crew, vec![apworld]);
    assert!(out.archipelago_file.is_file());
    assert_eq!(out.summary.players, 2);
    let names: Vec<&str> = out.summary.slots.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Léa", "Max \"M\""]);
    assert!(out.summary.slots.iter().all(|s| s.game == "Slime Rancher"));
}
