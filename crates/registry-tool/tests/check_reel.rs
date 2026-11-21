//! Test RÉEL du calcul du niveau (ADR 0045) : vrai générateur Archipelago 0.6.8, dans son
//! bac à sable, sur la vraie apworld de Slime Rancher.
//!
//! Ignoré par défaut. Il ne télécharge rien : il lui faut un runtime déjà installé
//! (`cargo run -p launcher-core --example setup_generator -- <dossier>`, racine dans
//! `RAFTUP_REAL_RUNTIME`) et l'apworld (`RAFTUP_SLIME_APWORLD`, chemin du fichier).
//!
//! `cargo test -p registry-tool --test check_reel -- --ignored --nocapture --test-threads=1`

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use launcher_core::generation::{pins, runtime_status, RuntimeStatus};
use launcher_core::Paths;
use registry::{sha256_hex, Package, PackageKind, Trust, TrustLevel};
use registry_tool::check::{check_package, Candidate, CheckSettings};
use registry_tool::official::OfficialGenerator;

#[test]
#[ignore = "demande RAFTUP_REAL_RUNTIME et RAFTUP_SLIME_APWORLD"]
fn slime_rancher_teste_en_profondeur() {
    let (Some(root), Some(apworld)) = (
        std::env::var_os("RAFTUP_REAL_RUNTIME"),
        std::env::var_os("RAFTUP_SLIME_APWORLD"),
    ) else {
        eprintln!("RAFTUP_REAL_RUNTIME ou RAFTUP_SLIME_APWORLD absente : test réel sauté");
        return;
    };
    let paths = Paths::under(&PathBuf::from(root));
    let RuntimeStatus::Ready(runtime) = runtime_status(&paths, &pins().unwrap()) else {
        panic!("runtime non prêt");
    };
    let data = std::fs::read(&apworld).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let copy = tmp.path().join("slime_rancher.apworld");
    std::fs::write(&copy, &data).unwrap();
    let package = Package {
        kind: PackageKind::Apworld,
        name: "slime_rancher".to_string(),
        version: "0.3.2.1".to_string(),
        game: Some("Slime Rancher".to_string()),
        url: "https://github.com/SWCreeperKing/Slimipelago/releases/download/v0.3.2.1/slime_rancher.apworld".to_string(),
        sha256: sha256_hex(&data),
        size: data.len() as u64,
        license: "unlicensed".to_string(),
        authors: vec!["SWCreeperKing".to_string()],
        trust: Trust::default(),
    };
    let mut generator = OfficialGenerator::new(paths, runtime, tmp.path().join("yaml"));
    let settings = CheckSettings {
        seeds: 1,
        fuzz_cases: 4,
        case_timeout: Duration::from_secs(240),
        ..CheckSettings::default()
    };
    let start = Instant::now();
    let candidate = Candidate {
        package: &package,
        data: &data,
        apworld_path: &copy,
    };
    let report = check_package(&candidate, Ok(&mut generator), &settings, &mut || {
        start.elapsed()
    })
    .unwrap();
    for step in &report.steps {
        eprintln!("{:?} {} : {}", step.status, step.name, step.detail);
    }
    assert_eq!(report.level, TrustLevel::Fuzzed);
    assert!(report.detail.contains("Archipelago 0.6.8"));
}
