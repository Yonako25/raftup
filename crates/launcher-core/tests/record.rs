//! Tests du reçu d'installation.

use std::path::PathBuf;

use launcher_core::hash::Sha256;
use launcher_core::paths::Paths;
use launcher_core::record::{
    Backup, FileRecord, InstallRecord, RecordError, RecordState, StepRecord,
};

fn sample() -> InstallRecord {
    let mut r = InstallRecord::new("slime-rancher", PathBuf::from("C:\\Jeux\\Slime Rancher é"));
    r.state = RecordState::Complete;
    r.steps.push(StepRecord {
        id: "melonloader".into(),
        version: "0.7.3".into(),
        root: PathBuf::from("C:/Jeux/Slime Rancher é"),
        sha256: Sha256::of_bytes(b"zip"),
        files: vec![
            FileRecord {
                path: PathBuf::from("version.dll"),
                sha256: Some(Sha256::of_bytes(b"dll")),
                backup: Some(Backup {
                    path: PathBuf::from("0/version.dll"),
                    original_sha256: Sha256::of_bytes(b"orig"),
                }),
            },
            FileRecord {
                path: PathBuf::from("Mods/a.dll"),
                sha256: None,
                backup: None,
            },
        ],
        dirs: vec![PathBuf::from("Mods")],
    });
    r
}

#[test]
fn aller_retour_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    let record = sample();
    record.save(&paths).unwrap();
    let loaded = InstallRecord::load(&paths, "slime-rancher").unwrap();
    assert_eq!(loaded, Some(record));
}

#[test]
fn absent_donne_none_et_remove_est_tolerant() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    assert_eq!(InstallRecord::load(&paths, "x").unwrap(), None);
    InstallRecord::remove(&paths, "x").unwrap();
    sample().save(&paths).unwrap();
    InstallRecord::remove(&paths, "slime-rancher").unwrap();
    assert_eq!(InstallRecord::load(&paths, "slime-rancher").unwrap(), None);
}

#[test]
fn aucun_tmp_ne_reste() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    sample().save(&paths).unwrap();
    sample().save(&paths).unwrap(); // réécriture par-dessus
    let names: Vec<String> = std::fs::read_dir(paths.installs_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["slime-rancher.toml".to_string()]);
}

#[test]
fn aucun_tmp_ne_reste_en_cas_d_erreur() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    // Un dossier à la place du fichier cible : le renommage échoue.
    std::fs::create_dir_all(paths.installs_dir().join("slime-rancher.toml")).unwrap();
    assert!(sample().save(&paths).is_err());
    assert!(!paths.installs_dir().join("slime-rancher.toml.tmp").exists());
}

#[test]
fn format_version_future_refuse() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    std::fs::create_dir_all(paths.installs_dir()).unwrap();
    std::fs::write(
        paths.installs_dir().join("g.toml"),
        "format_version = 2\nchamp_inconnu = true\n",
    )
    .unwrap();
    let err = InstallRecord::load(&paths, "g").unwrap_err();
    assert!(matches!(
        err,
        RecordError::UnsupportedVersion {
            found: 2,
            supported: 1
        }
    ));
}

#[test]
fn toml_invalide_refuse() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    std::fs::create_dir_all(paths.installs_dir()).unwrap();
    std::fs::write(paths.installs_dir().join("g.toml"), "pas du toml [[[").unwrap();
    assert!(matches!(
        InstallRecord::load(&paths, "g").unwrap_err(),
        RecordError::Toml(_)
    ));
}

#[test]
fn sortie_stable() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    let file = paths.installs_dir().join("slime-rancher.toml");
    sample().save(&paths).unwrap();
    let first = std::fs::read_to_string(&file).unwrap();
    sample().save(&paths).unwrap();
    let second = std::fs::read_to_string(&file).unwrap();
    assert_eq!(first, second);
    assert!(first.starts_with("format_version = 1\n"));
    assert!(first.contains("state = \"complete\""));
}

#[test]
fn identifiant_dangereux_refuse() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    for bad in ["", "../x", "a/b", "a\\b", "c:x"] {
        assert!(matches!(
            InstallRecord::load(&paths, bad).unwrap_err(),
            RecordError::InvalidGameId(_)
        ));
    }
}
