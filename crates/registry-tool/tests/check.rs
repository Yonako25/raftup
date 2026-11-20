//! Niveaux de confiance calculés avec un faux générateur (ADR 0042).
// Helpers de test : un test qui échoue doit simplement paniquer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::type_complexity)]

use std::cell::Cell;
use std::io::{Cursor, Write};
use std::path::Path;
use std::time::Duration;

use registry::publish::{keygen, publish, SECRET_KEY_FILE};
use registry::{sha256_hex, verify_index, Package, PackageKind, Trust, TrustLevel};
use registry_tool::check::{check_package, Candidate, CheckSettings, Generator, RunOutcome};
use registry_tool::options::{OptionKind, OptionSpec};
use registry_tool::report::{Report, StepStatus};
use registry_tool::run::{run_check, write_report, CheckRequest, PackageSource, REPORT_JSON};
use registry_tool::structure::check_apworld;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

/// Une apworld minimale : `monde/__init__.py` (+ fichiers en plus).
fn apworld(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn good_apworld() -> Vec<u8> {
    apworld(&[
        ("monde/__init__.py", "# monde"),
        ("monde/archipelago.json", r#"{"game": "Monde"}"#),
    ])
}

fn package(data: &[u8]) -> Package {
    Package {
        kind: PackageKind::Apworld,
        name: "monde".to_string(),
        version: "1.0".to_string(),
        game: Some("Monde".to_string()),
        url: "https://github.com/a/b/releases/download/v1/monde.apworld".to_string(),
        sha256: sha256_hex(data),
        size: data.len() as u64,
        license: "MIT".to_string(),
        authors: vec!["Auteur".to_string()],
        trust: Trust::default(),
    }
}

/// Faux générateur : rend `outcome(numéro d'appel, yaml)` et garde les YAML reçus.
struct Fake {
    calls: Vec<(String, u64)>,
    outcome: Box<dyn FnMut(usize, &str) -> RunOutcome>,
    options: Result<Vec<OptionSpec>, RunOutcome>,
}

impl Fake {
    fn new(outcome: impl FnMut(usize, &str) -> RunOutcome + 'static) -> Fake {
        Fake {
            calls: Vec::new(),
            outcome: Box::new(outcome),
            options: Ok(vec![
                OptionSpec {
                    name: "hard".into(),
                    kind: OptionKind::Toggle,
                },
                OptionSpec {
                    name: "count".into(),
                    kind: OptionKind::Range { start: 1, end: 9 },
                },
            ]),
        }
    }

    fn always_ok() -> Fake {
        Fake::new(|_, _| RunOutcome::Generated)
    }
}

impl Generator for Fake {
    fn name(&self) -> String {
        "Faux 1.0".to_string()
    }

    fn generate(&mut self, _: &Path, yaml: &str, seed: u64, _: Duration) -> RunOutcome {
        let n = self.calls.len();
        self.calls.push((yaml.to_string(), seed));
        (self.outcome)(n, yaml)
    }

    fn describe_options(
        &mut self,
        _: &Path,
        _: &str,
        _: Duration,
    ) -> Result<Vec<OptionSpec>, RunOutcome> {
        self.options.clone()
    }
}

fn settings() -> CheckSettings {
    CheckSettings {
        seeds: 3,
        fuzz_cases: 8,
        ..CheckSettings::default()
    }
}

/// Lance la vérification avec une horloge arrêtée.
fn run(
    data: &[u8],
    generator: Result<&mut dyn Generator, String>,
    settings: &CheckSettings,
) -> Result<registry_tool::report::PackageReport, String> {
    let package = package(data);
    let candidate = Candidate {
        package: &package,
        data,
        apworld_path: Path::new("monde.apworld"),
    };
    check_package(&candidate, generator, settings, &mut || Duration::ZERO)
}

#[test]
fn tout_passe_donne_teste_en_profondeur() {
    let mut fake = Fake::always_ok();
    let report = run(&good_apworld(), Ok(&mut fake), &settings()).unwrap();
    assert_eq!(report.level, TrustLevel::Fuzzed);
    // 3 graines par défaut, puis 8 cas.
    assert_eq!(fake.calls.len(), 3 + 8);
    assert_eq!(
        fake.calls[..3].iter().map(|c| c.1).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(report.detail.contains("Faux 1.0"));
    assert!(report.detail.contains("3 graine(s)"));
    assert!(report.detail.contains("8 cas de fuzzing"));
    assert!(report.steps.iter().all(|s| s.status == StepStatus::Passed));
}

#[test]
fn sans_generateur_structure_valide() {
    let report = run(
        &good_apworld(),
        Err("pas de Python ici".into()),
        &settings(),
    )
    .unwrap();
    assert_eq!(report.level, TrustLevel::Valid);
    assert_eq!(report.steps.len(), 3);
    assert_eq!(report.steps[1].status, StepStatus::Skipped);
    assert_eq!(report.steps[1].detail, "pas de Python ici");
}

#[test]
fn structure_invalide_non_verifie() {
    let mut fake = Fake::always_ok();
    let report = run(b"pas un zip", Ok(&mut fake), &settings()).unwrap();
    assert_eq!(report.level, TrustLevel::Unverified);
    assert_eq!(report.steps[0].status, StepStatus::Failed);
    assert!(
        fake.calls.is_empty(),
        "le générateur ne doit pas être appelé"
    );
}

#[test]
fn graine_refusee_reste_a_structure_valide() {
    let mut fake = Fake::new(|n, _| {
        if n == 1 {
            RunOutcome::Refused("FillError".into())
        } else {
            RunOutcome::Generated
        }
    });
    let report = run(&good_apworld(), Ok(&mut fake), &settings()).unwrap();
    assert_eq!(report.level, TrustLevel::Valid);
    assert!(report.steps[1].detail.contains("graine 2"));
    assert!(report.steps[1].detail.contains("FillError"));
    assert_eq!(report.steps[2].status, StepStatus::Skipped);
}

#[test]
fn plantage_ou_boucle_au_fuzzing_reste_generable() {
    for outcome in [
        RunOutcome::Crashed("process mort".into()),
        RunOutcome::TimedOut("300 s".into()),
    ] {
        let mut fake = Fake::new(move |n, _| {
            if n == 3 + 4 {
                outcome.clone()
            } else {
                RunOutcome::Generated
            }
        });
        let report = run(&good_apworld(), Ok(&mut fake), &settings()).unwrap();
        assert_eq!(report.level, TrustLevel::Generates);
        let fuzz = &report.steps[2];
        assert_eq!(fuzz.status, StepStatus::Failed);
        assert!(fuzz.detail.starts_with("cas 5 "), "{}", fuzz.detail);
        // On s'arrête au premier échec.
        assert_eq!(fake.calls.len(), 3 + 5);
        // Le détail de l'index ne parle pas de fuzzing réussi.
        assert!(!report.detail.contains("fuzzing"));
    }
}

#[test]
fn refus_propres_permis_et_notes() {
    let mut fake = Fake::new(|n, _| {
        if n >= 3 {
            RunOutcome::Refused("OptionError".into())
        } else {
            RunOutcome::Generated
        }
    });
    let report = run(&good_apworld(), Ok(&mut fake), &settings()).unwrap();
    assert_eq!(report.level, TrustLevel::Fuzzed);
    let fuzz = &report.steps[2];
    assert!(fuzz.detail.contains("8 refusé(s) proprement"));
    // Seuls les refus sur des options valides sont notés (cas 1, 4, 5, 8).
    assert_eq!(fuzz.notes.len(), 4);
}

#[test]
fn limite_de_temps_arrete_le_fuzzing() {
    let mut fake = Fake::always_ok();
    let package = package(&good_apworld());
    let data = good_apworld();
    let candidate = Candidate {
        package: &package,
        data: &data,
        apworld_path: Path::new("monde.apworld"),
    };
    let settings = CheckSettings {
        time_limit: Duration::from_secs(10),
        ..settings()
    };
    // Horloge : chaque lecture avance de 4 s ; la limite tombe avant le 4e cas.
    let ticks = Cell::new(0u64);
    let mut clock = || {
        ticks.set(ticks.get() + 4);
        Duration::from_secs(ticks.get() - 4)
    };
    let report = check_package(&candidate, Ok(&mut fake), &settings, &mut clock).unwrap();
    assert_eq!(report.level, TrustLevel::Generates);
    assert!(report.steps[2].detail.contains("après 3 cas sur 8"));
}

#[test]
fn erreur_d_environnement_sans_niveau() {
    let mut fake = Fake::new(|_, _| RunOutcome::Environment("disque plein".into()));
    assert_eq!(
        run(&good_apworld(), Ok(&mut fake), &settings()).unwrap_err(),
        "disque plein"
    );
}

#[test]
fn options_illisibles_restent_generable() {
    let mut fake = Fake::always_ok();
    fake.options = Err(RunOutcome::Refused("script".into()));
    let report = run(&good_apworld(), Ok(&mut fake), &settings()).unwrap();
    assert_eq!(report.level, TrustLevel::Generates);
    assert!(report.steps[2]
        .detail
        .contains("options du monde illisibles"));
}

#[test]
fn fuzzing_reproductible() {
    let mut a = Fake::always_ok();
    let mut b = Fake::always_ok();
    run(&good_apworld(), Ok(&mut a), &settings()).unwrap();
    run(&good_apworld(), Ok(&mut b), &settings()).unwrap();
    assert_eq!(a.calls, b.calls);
    // Une autre graine de fuzzing change les cas.
    let mut c = Fake::always_ok();
    let other = CheckSettings {
        fuzz_seed: 1,
        ..settings()
    };
    run(&good_apworld(), Ok(&mut c), &other).unwrap();
    assert_ne!(a.calls, c.calls);
}

#[test]
fn structure_des_apworlds() {
    assert!(check_apworld("monde", Some("Monde"), &good_apworld()).is_ok());
    let cas = [
        (apworld(&[("monde/rules.py", "")]), "__init__.py"),
        (
            apworld(&[("monde/__init__.py", ""), ("../evil.py", "")]),
            "..",
        ),
        (
            apworld(&[("monde/__init__.py", ""), ("autre/x.py", "")]),
            "hors du dossier",
        ),
        (
            apworld(&[("monde/__init__.py", ""), ("C:/x.py", "")]),
            "absolu",
        ),
        (
            apworld(&[
                ("monde/__init__.py", ""),
                ("monde/archipelago.json", r#"{"game": "Autre"}"#),
            ]),
            "Autre",
        ),
        (
            apworld(&[("monde/__init__.py", ""), ("archipelago.json", "[1]")]),
            "objet",
        ),
    ];
    for (data, attendu) in cas {
        let erreur = check_apworld("monde", Some("Monde"), &data).unwrap_err();
        assert!(erreur.contains(attendu), "{erreur}");
    }
}

/// Bout à bout : source → check (fichiers locaux puis téléchargement) → rapport → publish.
#[test]
fn rapport_puis_publication_signee() {
    let tmp = tempfile::tempdir().unwrap();
    let dist = tmp.path().join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("monde.apworld"), good_apworld()).unwrap();
    std::fs::write(
        dist.join("demo.toml"),
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles/slime-rancher.toml"),
        )
        .unwrap(),
    )
    .unwrap();
    let source = tmp.path().join("registre.toml");
    std::fs::write(
        &source,
        r#"
format_version = 1
registry = "raftup"

[[package]]
kind = "apworld"
name = "monde"
version = "1.0"
game = "Monde"
file = "dist/monde.apworld"
url = "https://github.com/a/b/releases/download/v1/monde.apworld"
license = "MIT"
authors = ["Auteur"]

[[package]]
kind = "profile"
name = "slime-rancher"
version = "1"
file = "dist/demo.toml"
url = "https://github.com/a/b/releases/download/v1/slime-rancher.toml"
license = "MIT"
authors = ["Auteur"]
"#,
    )
    .unwrap();

    let mut fake = Fake::always_ok();
    let request = CheckRequest {
        source: &source,
        only: None,
        packages: PackageSource::LocalFiles,
        work_dir: &tmp.path().join("travail"),
        settings: settings(),
    };
    let report = run_check(&request, Ok(&mut fake)).unwrap();
    let levels: Vec<TrustLevel> = report.packages.iter().map(|p| p.level).collect();
    assert_eq!(levels, [TrustLevel::Fuzzed, TrustLevel::Valid]);
    assert_eq!(report.generator.as_deref(), Some("Faux 1.0"));

    // Le même rapport, en téléchargeant chez l'auteur (faux réseau).
    let fetcher = launcher_core::DirFetcher::new()
        .with_file(
            "https://github.com/a/b/releases/download/v1/monde.apworld",
            dist.join("monde.apworld"),
        )
        .with_file(
            "https://github.com/a/b/releases/download/v1/slime-rancher.toml",
            dist.join("demo.toml"),
        );
    let mut fake2 = Fake::always_ok();
    let download = CheckRequest {
        packages: PackageSource::Download(&fetcher),
        ..request
    };
    assert_eq!(run_check(&download, Ok(&mut fake2)).unwrap(), report);
    // `--paquet` : un seul.
    let only = CheckRequest {
        only: Some("monde"),
        packages: PackageSource::LocalFiles,
        ..download
    };
    assert_eq!(run_check(&only, Err("x".into())).unwrap().packages.len(), 1);

    // Rapport écrit puis relu ; le Markdown nomme les niveaux.
    let out = tmp.path().join("rapport");
    let (json, markdown) = write_report(&report, &out).unwrap();
    assert_eq!(json, out.join(REPORT_JSON));
    let relu = Report::parse(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(relu, report);
    let md = std::fs::read_to_string(markdown).unwrap();
    assert!(md.contains("Testé en profondeur") && md.contains("Structure valide"));

    // Publication avec le rapport : les niveaux sont dans l'index signé.
    let cles = tmp.path().join("cles");
    let publique = keygen(&cles).unwrap();
    let sortie = tmp.path().join("sortie");
    let publie = publish(
        &source,
        &cles.join(SECRET_KEY_FILE),
        &sortie,
        &relu.claims(),
    )
    .unwrap();
    assert!(publie.warnings.is_empty(), "{:?}", publie.warnings);
    let index = verify_index(
        &std::fs::read(sortie.join("index.json")).unwrap(),
        &std::fs::read_to_string(sortie.join("index.json.minisig")).unwrap(),
        &publique,
    )
    .unwrap();
    let monde = index.get(PackageKind::Apworld, "monde").unwrap();
    assert_eq!(monde.trust.level, TrustLevel::Fuzzed);
    assert!(monde.trust.detail.as_deref().unwrap().contains("Faux 1.0"));

    // Le fichier change après le rapport : le niveau n'est pas repris.
    std::fs::write(
        dist.join("monde.apworld"),
        apworld(&[("monde/__init__.py", "# v2")]),
    )
    .unwrap();
    let publie = publish(
        &source,
        &cles.join(SECRET_KEY_FILE),
        &sortie,
        &relu.claims(),
    )
    .unwrap();
    assert_eq!(publie.levels[0].1, TrustLevel::Unverified);
    assert_eq!(publie.warnings.len(), 1);
}

#[test]
fn rapport_d_un_format_inconnu_refuse() {
    assert!(Report::parse(br#"{"format_version": 9}"#)
        .unwrap_err()
        .contains("format 9"));
    assert!(Report::parse(b"pas du json").is_err());
}
