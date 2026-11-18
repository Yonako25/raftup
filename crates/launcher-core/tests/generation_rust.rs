//! Génération par le moteur Rust (ADR 0027, étape 4), de bout en bout : fichiers YAML
//! → `generate` → `AP_<seed>.zip` → seed relue.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::sync::Mutex;

use launcher_core::generation::{generate, rust_support, GenerateRequest, RustBackend};
use launcher_core::{GenerationError, Paths};
use py_bridge::CancelToken;

/// Une seule génération à la fois dans le process : les tests passent l'un après l'autre.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn yaml(dir: &std::path::Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn seed_apquest_sans_python() {
    let _serial = serial();
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    let alice = yaml(
        tmp.path(),
        "alice.yaml",
        "name: Alice\ngame: APQuest\nAPQuest:\n  hammer: true\n",
    );
    let bob = yaml(tmp.path(), "bob.yaml", "name: Bob\ngame: APQuest\n");
    let request = GenerateRequest {
        player_files: vec![alice.clone(), bob.clone()],
        // Un apworld (même absent) est ignoré par le moteur Rust : aucune erreur.
        apworlds: vec![tmp.path().join("absent.apworld")],
        seed: Some(12345),
        ..GenerateRequest::default()
    };
    assert_eq!(rust_support(&request.player_files), Ok(()));

    let (log, lines) = channel();
    let output = generate(&paths, &mut RustBackend, &request, &CancelToken::new(), log).unwrap();
    assert_eq!(output.seed_name, "00000000000000012345");
    assert!(output.zip_file.is_file());
    assert!(output.archipelago_file.is_file());
    assert_eq!(output.summary.players, 2);
    let names: Vec<_> = output.players.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Alice", "Bob"]);
    assert!(lines.try_iter().any(|l| l.contains("moteur Rust")));

    // Même seed, mêmes fichiers ⇒ même seed écrite.
    let first = std::fs::read(&output.archipelago_file).unwrap();
    let (log, _) = channel();
    let again = generate(&paths, &mut RustBackend, &request, &CancelToken::new(), log).unwrap();
    assert_eq!(std::fs::read(&again.archipelago_file).unwrap(), first);
}

#[test]
fn jeu_non_porte_refuse_avec_la_raison() {
    let _serial = serial();
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    let file = yaml(tmp.path(), "a.yaml", "name: A\ngame: Slime Rancher\n");
    let reason = rust_support(std::slice::from_ref(&file)).unwrap_err();
    assert!(reason.contains("Slime Rancher"), "{reason}");

    let request = GenerateRequest {
        player_files: vec![file],
        ..GenerateRequest::default()
    };
    let (log, _) = channel();
    let err = generate(&paths, &mut RustBackend, &request, &CancelToken::new(), log).unwrap_err();
    assert!(
        matches!(&err, GenerationError::GeneratorFailed { kind, .. } if kind == "Unsupported"),
        "{err:?}"
    );
    // Le dossier de cette génération ratée a été retiré.
    let left = std::fs::read_dir(paths.generations_dir()).unwrap().count();
    assert_eq!(left, 0);
}
