//! Test RÉEL de la génération : vrai Python embarqué, vrai générateur Archipelago 0.6.8.
//!
//! Ignoré par défaut. Il ne télécharge JAMAIS rien : il lui faut un runtime déjà installé
//! par `cargo run -p launcher-core --example setup_generator -- <dossier>`, dont le dossier
//! racine est donné par la variable `RAFTUP_REAL_RUNTIME`.
//!
//! `cargo test -p launcher-core --test generation_real -- --ignored --nocapture --test-threads=1`
//!
//! Un seul fil : le runtime n'accepte qu'une génération à la fois (sinon `Busy`).
//!
//! Remarque : Archipelago écrit `host.yaml` et `logs/` dans SON dossier, donc dans le
//! runtime installé (jamais dans le dépôt). Les seeds et les sorties vont dans un dossier
//! temporaire.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use launcher_core::generation::{
    generate, pins, runtime_status, GenerateRequest, PythonBackend, RuntimeStatus,
};
use launcher_core::{GenerationError, Paths};
use py_bridge::CancelToken;

/// Le jeu intégré le plus simple d'Archipelago 0.6.8 : sans ROM, sans fichier du jeu.
/// (Le monde « Clique » n'existe pas dans cette version.)
const YAML: &str = "name: Test\ngame: APQuest\nAPQuest:\n  hard_mode: false\n";

/// Dossier racine du runtime, ou `None` (avec un message) si la variable manque.
fn runtime_root() -> Option<PathBuf> {
    match std::env::var_os("RAFTUP_REAL_RUNTIME") {
        Some(v) => Some(PathBuf::from(v)),
        None => {
            eprintln!(
                "RAFTUP_REAL_RUNTIME absente : test réel sauté (installer avec setup_generator)"
            );
            None
        }
    }
}

fn backend(root: &Path) -> PythonBackend {
    let status = runtime_status(&Paths::under(root), &pins().expect("pins"));
    match status {
        RuntimeStatus::Ready(runtime) => PythonBackend::new(runtime),
        other => panic!("runtime non prêt dans {} : {other:?}", root.display()),
    }
}

fn request(dir: &Path, yaml: &str, seed: u64) -> GenerateRequest {
    let file = dir.join("test.yaml");
    std::fs::write(&file, yaml).unwrap();
    GenerateRequest {
        player_files: vec![file],
        seed: Some(seed),
        timeout: Duration::from_secs(300),
        ..GenerateRequest::default()
    }
}

#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn generation_reelle_apquest() {
    let Some(root) = runtime_root() else { return };
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = backend(&root);

    // Première génération, seed fixe.
    let (tx, rx) = channel();
    let t0 = Instant::now();
    let out = generate(
        &paths,
        &mut backend,
        &request(work.path(), YAML, 123456),
        &CancelToken::new(),
        tx,
    )
    .expect("génération");
    eprintln!("génération 1 : {:?}", t0.elapsed());
    let logs: Vec<String> = rx.try_iter().collect();
    eprintln!("--- {} lignes de log ---", logs.len());
    for l in logs.iter() {
        eprintln!("{l}");
    }

    assert!(out.archipelago_file.is_file());
    assert_eq!(out.summary.seed_name, out.seed_name);
    let slot = out
        .summary
        .slots
        .iter()
        .find(|s| s.name == "Test")
        .expect("slot Test");
    assert_eq!(slot.game, "APQuest");
    assert_eq!(out.summary.players, 1);
    assert_eq!(out.summary.generator_version, "0.6.8");
    assert_eq!(out.players.len(), 1);
    assert_eq!(out.players[0].name, "Test");

    // Même seed, même nom de seed.
    let (tx, _rx) = channel();
    let out2 = generate(
        &paths,
        &mut backend,
        &request(work.path(), YAML, 123456),
        &CancelToken::new(),
        tx,
    )
    .expect("génération 2");
    assert_eq!(out.seed_name, out2.seed_name);
}

#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn yaml_invalide_rend_generator_failed() {
    let Some(root) = runtime_root() else { return };
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = backend(&root);
    let (tx, _rx) = channel();
    // Jeu inconnu : le générateur refuse.
    let err = generate(
        &paths,
        &mut backend,
        &request(work.path(), "name: Test\ngame: JeuQuiNExistePas\n", 1),
        &CancelToken::new(),
        tx,
    )
    .expect_err("doit échouer");
    match err {
        GenerationError::GeneratorFailed {
            kind, traceback, ..
        } => {
            eprintln!("kind = {kind}\n{traceback}");
            assert!(!traceback.is_empty(), "traceback attendu");
        }
        other => panic!("autre erreur : {other:?}"),
    }
    // Le dossier de génération est nettoyé après l'échec.
    let left = std::fs::read_dir(paths.generations_dir())
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(left, 0, "generations/ doit être vide après un échec");
}

#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn annulation_precoce_rend_cancelled() {
    let Some(root) = runtime_root() else { return };
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = backend(&root);
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    // Annulation 700 ms après le début : le worker est lancé, Archipelago s'importe.
    let handle = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        trigger.cancel();
    });
    let (tx, _rx) = channel();
    let t0 = Instant::now();
    let err = generate(
        &paths,
        &mut backend,
        &request(work.path(), YAML, 42),
        &cancel,
        tx,
    )
    .expect_err("doit être annulée");
    handle.join().unwrap();
    eprintln!("annulation rendue après {:?}", t0.elapsed());
    assert!(matches!(err, GenerationError::Cancelled), "{err:?}");
    assert!(t0.elapsed() < Duration::from_secs(20));
}

/// Un `meta.yaml` valide (format d'Archipelago 0.6.8) est lu par le vrai générateur :
/// il le dit dans son journal (`Meta: ... >> <meta_description>`) et la génération réussit.
#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn generation_reelle_avec_meta_yaml() {
    let Some(root) = runtime_root() else { return };
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = backend(&root);
    let mut req = request(work.path(), YAML, 777);
    req.meta_yaml = Some(
        "meta_description: Raftup test meta\nnull:\n  progression_balancing: 80\n  accessibility: minimal\n"
            .to_string(),
    );
    let (tx, rx) = channel();
    let out = generate(&paths, &mut backend, &req, &CancelToken::new(), tx).expect("génération");
    assert_eq!(out.players.len(), 1);
    let logs: Vec<String> = rx.try_iter().collect();
    assert!(
        logs.iter().any(|l| l.contains("Raftup test meta")),
        "le générateur n'a pas lu le meta.yaml : {logs:?}"
    );
}

/// Un monde qui demande la ROM du joueur (Secret of Evermore) : sans ROM, le générateur
/// lève un `FileNotFoundError`, que l'interface traduit en message clair.
#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn monde_avec_rom_absente() {
    let Some(root) = runtime_root() else { return };
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = backend(&root);
    let yaml = "name: Test\ngame: Secret of Evermore\nSecret of Evermore: {}\n";
    let (tx, _rx) = channel();
    let error = generate(
        &paths,
        &mut backend,
        &request(work.path(), yaml, 1),
        &CancelToken::new(),
        tx,
    )
    .expect_err("sans ROM, la génération doit échouer");
    eprintln!("{error:?}");
    match error {
        GenerationError::GeneratorFailed { kind, message, .. } => {
            assert_eq!(kind, "FileNotFoundError");
            assert!(message.contains(".sfc"), "{message}");
        }
        other => panic!("attendu GeneratorFailed, reçu {other:?}"),
    }
}
