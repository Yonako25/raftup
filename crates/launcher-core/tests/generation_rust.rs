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
    assert_eq!(rust_support(&request.player_files, None), Ok(()));

    let (log, lines) = channel();
    let output = generate(
        &paths,
        &mut RustBackend::default(),
        &request,
        &CancelToken::new(),
        log,
    )
    .unwrap();
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
    let again = generate(
        &paths,
        &mut RustBackend::default(),
        &request,
        &CancelToken::new(),
        log,
    )
    .unwrap();
    assert_eq!(std::fs::read(&again.archipelago_file).unwrap(), first);
}

/// Noms des fichiers du zip produit.
fn zip_entries(path: &std::path::Path) -> Vec<String> {
    let archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    archive
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect()
}

fn zip_text(path: &std::path::Path, name: &str) -> String {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut text = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

#[test]
fn spoiler_dans_le_zip_selon_le_niveau() {
    let _serial = serial();
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    let alice = yaml(tmp.path(), "alice.yaml", "name: Alice\ngame: APQuest\n");
    let bob = yaml(tmp.path(), "bob.yaml", "name: Bob\ngame: APQuest\n");
    let request = GenerateRequest {
        player_files: vec![alice, bob],
        seed: Some(99),
        ..GenerateRequest::default()
    };
    // Niveau par défaut (3) : spoiler complet, parcours compris.
    let (log, _) = channel();
    let output = generate(
        &paths,
        &mut RustBackend::default(),
        &request,
        &CancelToken::new(),
        log,
    )
    .unwrap();
    let spoiler = "AP_00000000000000000099_Spoiler.txt";
    let mut names = zip_entries(&output.zip_file);
    names.sort();
    assert_eq!(names, ["AP_00000000000000000099.archipelago", spoiler]);
    let text = zip_text(&output.zip_file, spoiler);
    assert!(text.starts_with("Raftup "), "{text}");
    assert!(text.contains("Seed: 00000000000000000099"));
    assert!(text.contains("\nPlayer 2: Bob\n"));
    assert!(text.contains("\n\nLocations:\n\n"));
    assert!(text.contains("\n\nPlaythrough:\n\n"));

    // Niveau 1 : placement sans parcours.
    let request1 = GenerateRequest {
        spoiler: 1,
        ..request.clone()
    };
    let (log, _) = channel();
    let output = generate(
        &paths,
        &mut RustBackend::default(),
        &request1,
        &CancelToken::new(),
        log,
    )
    .unwrap();
    let text = zip_text(&output.zip_file, spoiler);
    assert!(text.contains("Locations:") && !text.contains("Playthrough"));

    // Niveau 0 : pas de spoiler.
    let request0 = GenerateRequest {
        spoiler: 0,
        ..request
    };
    let (log, _) = channel();
    let output = generate(
        &paths,
        &mut RustBackend::default(),
        &request0,
        &CancelToken::new(),
        log,
    )
    .unwrap();
    assert_eq!(
        zip_entries(&output.zip_file),
        ["AP_00000000000000000099.archipelago"]
    );
}

#[test]
fn rapport_du_moteur_rust() {
    use launcher_core::generation::{EngineUsed, GeneratorBackend};
    let report = RustBackend::default().report();
    assert_eq!(report.engine, EngineUsed::Rust);
    assert_eq!(report.fallback, None);
}

#[test]
fn jeu_non_porte_refuse_avec_la_raison() {
    let _serial = serial();
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    let file = yaml(tmp.path(), "a.yaml", "name: A\ngame: Slime Rancher\n");
    let reason = rust_support(std::slice::from_ref(&file), None).unwrap_err();
    assert!(reason.contains("Slime Rancher"), "{reason}");

    let request = GenerateRequest {
        player_files: vec![file],
        ..GenerateRequest::default()
    };
    let (log, _) = channel();
    let err = generate(
        &paths,
        &mut RustBackend::default(),
        &request,
        &CancelToken::new(),
        log,
    )
    .unwrap_err();
    assert!(
        matches!(&err, GenerationError::GeneratorFailed { kind, .. } if kind == "Unsupported"),
        "{err:?}"
    );
    // Le dossier de cette génération ratée a été retiré.
    let left = std::fs::read_dir(paths.generations_dir()).unwrap().count();
    assert_eq!(left, 0);
}

#[test]
fn meta_yaml_du_prereglage() {
    let _serial = serial();
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    let file = yaml(tmp.path(), "a.yaml", "name: A\ngame: APQuest\n");
    // « Long » : compris par le moteur Rust, la génération réussit.
    let long = "meta_description: Raftup, partie longue\nnull:\n  progression_balancing: 20\n  accessibility: full\n";
    assert_eq!(
        rust_support(std::slice::from_ref(&file), Some(long)),
        Ok(())
    );
    let request = GenerateRequest {
        player_files: vec![file.clone()],
        meta_yaml: Some(long.to_string()),
        seed: Some(7),
        ..GenerateRequest::default()
    };
    let (log, _) = channel();
    generate(
        &paths,
        &mut RustBackend::default(),
        &request,
        &CancelToken::new(),
        log,
    )
    .unwrap();

    // « Rapide » (accès minimal) : refusé avec la raison, jamais ignoré en silence.
    let rapide =
        "meta_description: x\nnull:\n  progression_balancing: 80\n  accessibility: minimal\n";
    assert!(rust_support(std::slice::from_ref(&file), Some(rapide))
        .unwrap_err()
        .contains("accessibility"));
    let request = GenerateRequest {
        meta_yaml: Some(rapide.to_string()),
        ..request
    };
    let (log, _) = channel();
    let err = generate(
        &paths,
        &mut RustBackend::default(),
        &request,
        &CancelToken::new(),
        log,
    )
    .unwrap_err();
    assert!(
        matches!(&err, GenerationError::GeneratorFailed { kind, .. } if kind == "Unsupported"),
        "{err:?}"
    );
}

/// Moteur Rust avec l'équilibrage par temps (ADR 0030) : le journal le dit, la seed est
/// reproductible ; sans l'option, pas de ligne de journal.
#[test]
fn equilibrage_par_temps_sur_option() {
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
        player_files: vec![alice, bob],
        seed: Some(777),
        ..GenerateRequest::default()
    };

    let (log, lines) = channel();
    let mut off = RustBackend::default();
    generate(&paths, &mut off, &request, &CancelToken::new(), log).unwrap();
    assert!(!lines
        .try_iter()
        .any(|l| l.contains("Équilibrage par temps")));

    let mut on = RustBackend { time_balance: true };
    let (log, lines) = channel();
    let first = generate(&paths, &mut on, &request, &CancelToken::new(), log).unwrap();
    assert!(lines
        .try_iter()
        .any(|l| l.contains("Équilibrage par temps")));
    let first = std::fs::read(&first.archipelago_file).unwrap();
    let (log, _) = channel();
    let again = generate(&paths, &mut on, &request, &CancelToken::new(), log).unwrap();
    assert_eq!(std::fs::read(&again.archipelago_file).unwrap(), first);
}
