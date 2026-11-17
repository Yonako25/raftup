//! Tests de la désinstallation, à partir de reçus écrits à la main.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use launcher_core::hash::Sha256;
use launcher_core::install::InstallError;
use launcher_core::paths::Paths;
use launcher_core::record::{Backup, FileRecord, InstallRecord, RecordState, StepRecord};
use launcher_core::uninstall::uninstall;

const GAME: &str = "jeu";

struct Setup {
    _tmp: tempfile::TempDir,
    paths: Paths,
    root: PathBuf,
}

fn setup() -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(&tmp.path().join("app"));
    let root = tmp.path().join("Jeu é");
    fs::create_dir_all(&root).unwrap();
    Setup {
        _tmp: tmp,
        paths,
        root,
    }
}

fn write(path: &Path, data: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

fn file(path: &str, data: &[u8]) -> FileRecord {
    FileRecord {
        path: PathBuf::from(path),
        sha256: Some(Sha256::of_bytes(data)),
        backup: None,
    }
}

fn save(s: &Setup, state: RecordState, files: Vec<FileRecord>, dirs: &[&str]) {
    let mut record = InstallRecord::new(GAME, s.root.clone());
    record.state = state;
    record.steps.push(StepRecord {
        id: "mod".into(),
        version: "1".into(),
        root: s.root.clone(),
        sha256: Sha256::of_bytes(b"zip"),
        files,
        dirs: dirs.iter().map(PathBuf::from).collect(),
    });
    record.save(&s.paths).unwrap();
}

fn record_exists(s: &Setup) -> bool {
    InstallRecord::load(&s.paths, GAME).unwrap().is_some()
}

#[test]
fn fichiers_crees_supprimes_et_recu_efface() {
    let s = setup();
    write(&s.root.join("Mods").join("a.dll"), b"A");
    write(&s.root.join("b.dll"), b"B");
    save(
        &s,
        RecordState::Complete,
        vec![file("Mods/a.dll", b"A"), file("b.dll", b"B")],
        &["Mods"],
    );
    let report = uninstall(&s.paths, GAME, false).unwrap();
    assert_eq!(report.removed, 2);
    assert_eq!(report.restored, 0);
    assert!(report.conflicts.is_empty() && report.kept_dirs.is_empty());
    assert!(!s.root.join("b.dll").exists());
    assert!(!s.root.join("Mods").exists(), "dossier vide supprimé");
    assert!(s.root.exists(), "la racine du jeu reste");
    assert!(!record_exists(&s));
}

#[test]
fn rien_d_installe() {
    let s = setup();
    let err = uninstall(&s.paths, GAME, false).unwrap_err();
    assert!(matches!(err, InstallError::NotInstalled { .. }));
}

#[test]
fn sauvegarde_restauree() {
    let s = setup();
    write(&s.root.join("version.dll"), b"MOD");
    let backup_file = s.paths.backups_dir(GAME).join("0").join("version.dll");
    write(&backup_file, b"ORIGINAL");
    let mut f = file("version.dll", b"MOD");
    f.backup = Some(Backup {
        path: PathBuf::from("0/version.dll"),
        original_sha256: Sha256::of_bytes(b"ORIGINAL"),
    });
    save(&s, RecordState::Complete, vec![f], &[]);

    let report = uninstall(&s.paths, GAME, false).unwrap();
    assert_eq!(report.restored, 1);
    assert_eq!(report.removed, 0);
    assert_eq!(fs::read(s.root.join("version.dll")).unwrap(), b"ORIGINAL");
    assert!(!backup_file.exists());
    assert!(!record_exists(&s));
}

#[test]
fn dossier_non_vide_garde() {
    let s = setup();
    write(&s.root.join("Mods").join("a.dll"), b"A");
    write(&s.root.join("Mods").join("config.cfg"), b"cree par le jeu");
    save(
        &s,
        RecordState::Complete,
        vec![file("Mods/a.dll", b"A")],
        &["Mods"],
    );
    let report = uninstall(&s.paths, GAME, false).unwrap();
    assert_eq!(report.kept_dirs, vec![s.root.join("Mods")]);
    assert!(s.root.join("Mods").join("config.cfg").exists());
    assert!(!record_exists(&s));
}

#[test]
fn fichier_modifie_conflit_puis_force() {
    let s = setup();
    write(&s.root.join("a.dll"), b"MODIFIE PAR LE JOUEUR");
    save(&s, RecordState::Complete, vec![file("a.dll", b"A")], &[]);

    let report = uninstall(&s.paths, GAME, false).unwrap();
    assert_eq!(report.conflicts, vec![s.root.join("a.dll")]);
    assert_eq!(report.removed, 0);
    assert!(s.root.join("a.dll").exists());
    assert!(
        record_exists(&s),
        "reçu gardé pour pouvoir relancer avec force"
    );

    let report = uninstall(&s.paths, GAME, true).unwrap();
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(report.removed, 1);
    assert!(!s.root.join("a.dll").exists());
    assert!(!record_exists(&s));
}

#[test]
fn recu_in_progress_desinstallable() {
    let s = setup();
    write(&s.root.join("a.dll"), b"partiel");
    // Installation interrompue : le hash n'est pas encore noté.
    let f = FileRecord {
        path: PathBuf::from("a.dll"),
        sha256: None,
        backup: None,
    };
    // Un fichier prévu mais jamais écrit ne gêne pas.
    let g = FileRecord {
        path: PathBuf::from("jamais.dll"),
        sha256: None,
        backup: None,
    };
    save(&s, RecordState::InProgress, vec![f, g], &[]);
    let report = uninstall(&s.paths, GAME, false).unwrap();
    assert_eq!(report.removed, 1);
    assert!(!s.root.join("a.dll").exists());
    assert!(!record_exists(&s));
}

#[test]
fn chemin_malveillant_refuse_sans_rien_supprimer() {
    let s = setup();
    let outside = s.root.parent().unwrap().join("x");
    write(&outside, b"important");
    write(&s.root.join("a.dll"), b"A");
    save(
        &s,
        RecordState::Complete,
        vec![file("a.dll", b"A"), file("../x", b"important")],
        &[],
    );
    let err = uninstall(&s.paths, GAME, true).unwrap_err();
    assert!(matches!(err, InstallError::UnsafePath(_)));
    assert!(outside.exists());
    assert!(s.root.join("a.dll").exists(), "rien n'a été supprimé");
    assert!(record_exists(&s));
}

#[test]
fn racine_relative_refusee() {
    let s = setup();
    let mut record = InstallRecord::new(GAME, s.root.clone());
    record.steps.push(StepRecord {
        id: "m".into(),
        version: "1".into(),
        root: PathBuf::from("relatif"),
        sha256: Sha256::of_bytes(b"z"),
        files: vec![],
        dirs: vec![],
    });
    record.save(&s.paths).unwrap();
    assert!(matches!(
        uninstall(&s.paths, GAME, false).unwrap_err(),
        InstallError::UnsafeRoot(_)
    ));
}

#[test]
fn relance_apres_interruption() {
    let s = setup();
    // Première moitié déjà faite : a.dll supprimé, l'original déjà restauré mais la
    // sauvegarde pas encore effacée.
    write(&s.root.join("b.dll"), b"B");
    write(&s.root.join("v.dll"), b"ORIGINAL");
    let backup_file = s.paths.backups_dir(GAME).join("v.dll");
    write(&backup_file, b"ORIGINAL");
    let mut v = file("v.dll", b"MOD");
    v.backup = Some(Backup {
        path: PathBuf::from("v.dll"),
        original_sha256: Sha256::of_bytes(b"ORIGINAL"),
    });
    save(
        &s,
        RecordState::Complete,
        vec![file("a.dll", b"A"), file("b.dll", b"B"), v],
        &["Mods"],
    );
    let report = uninstall(&s.paths, GAME, false).unwrap();
    assert!(report.conflicts.is_empty());
    assert_eq!(report.removed, 1); // seulement b.dll
    assert_eq!(fs::read(s.root.join("v.dll")).unwrap(), b"ORIGINAL");
    assert!(!backup_file.exists());
    assert!(!record_exists(&s));
}

#[test]
fn sauvegarde_manquante_est_une_erreur_et_garde_le_recu() {
    let s = setup();
    write(&s.root.join("v.dll"), b"MOD");
    let mut v = file("v.dll", b"MOD");
    v.backup = Some(Backup {
        path: PathBuf::from("v.dll"),
        original_sha256: Sha256::of_bytes(b"ORIGINAL"),
    });
    save(&s, RecordState::Complete, vec![v], &[]);
    assert!(matches!(
        uninstall(&s.paths, GAME, false).unwrap_err(),
        InstallError::BackupMissing(_)
    ));
    assert!(record_exists(&s));
}

#[test]
fn lien_dans_le_chemin_refuse() {
    let s = setup();
    let elsewhere = s.root.parent().unwrap().join("ailleurs");
    write(&elsewhere.join("a.dll"), b"A");
    // Un lien (jonction sous Windows) à la place d'un dossier du jeu. Si le système
    // refuse de le créer (droits), le test n'a rien à vérifier.
    let link = s.root.join("Mods");
    #[cfg(windows)]
    let created = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&elsewhere)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    #[cfg(not(windows))]
    let created = std::os::unix::fs::symlink(&elsewhere, &link).is_ok();
    if !created {
        return;
    }
    save(
        &s,
        RecordState::Complete,
        vec![file("Mods/a.dll", b"A")],
        &[],
    );
    let err = uninstall(&s.paths, GAME, true).unwrap_err();
    assert!(matches!(err, InstallError::Link(_)));
    assert!(elsewhere.join("a.dll").exists(), "le lien n'est pas suivi");
}
