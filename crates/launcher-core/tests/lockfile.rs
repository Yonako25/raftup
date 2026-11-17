//! Tests du lockfile.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use launcher_core::hash::Sha256;
use launcher_core::lockfile::{
    path_for_seed, LockError, LockedMod, Lockfile, Mismatch, FORMAT_VERSION,
};
use launcher_core::profile::Profile;
use launcher_core::record::{InstallRecord, RecordState, StepRecord};

const PROFILE: &str = include_str!("data/profile/jeu-test.toml");

fn hash(byte: u8) -> Sha256 {
    Sha256::of_bytes(&[byte])
}

fn lock() -> Lockfile {
    let mut lock = Lockfile::new("jeu-test");
    lock.mods = vec![
        LockedMod {
            id: "melonloader".into(),
            version: "0.7.3".into(),
            sha256: hash(1),
        },
        LockedMod {
            id: "le-mod".into(),
            version: "1.2.0".into(),
            sha256: hash(2),
        },
    ];
    lock
}

fn step(id: &str, version: &str, sha: Sha256) -> StepRecord {
    StepRecord {
        id: id.into(),
        version: version.into(),
        root: "C:/jeu".into(),
        sha256: sha,
        files: vec![],
        dirs: vec![],
    }
}

/// Un reçu qui correspond exactement au lockfile.
fn matching_record() -> InstallRecord {
    let mut record = InstallRecord::new("jeu-test", "C:/jeu".into());
    record.state = RecordState::Complete;
    record.steps = vec![
        step("melonloader", "0.7.3", hash(1)),
        step("le-mod", "1.2.0", hash(2)),
    ];
    record
}

#[test]
fn aller_retour_sans_tmp_restant() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp
        .path()
        .join("sous")
        .join("partie.archipelago.raftup-lock.toml");
    lock().save(&path).unwrap();
    assert_eq!(Lockfile::load(&path).unwrap(), lock());
    let names: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["partie.archipelago.raftup-lock.toml"]);
    // Sortie stable, même en réécrivant par-dessus.
    let first = fs::read_to_string(&path).unwrap();
    lock().save(&path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), first);
}

#[test]
fn chemin_a_cote_de_la_seed() {
    assert_eq!(
        path_for_seed(Path::new("parties/AP_123.archipelago")),
        Path::new("parties/AP_123.archipelago.raftup-lock.toml")
    );
}

#[test]
fn aucune_url_ni_chemin_dans_le_fichier() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("x.raftup-lock.toml");
    // Fabriqué depuis un profil qui, lui, contient des URL et des chemins.
    let profile = Profile::parse("jeu-test.toml", PROFILE).unwrap();
    let from_profile = Lockfile::from_profile(&profile);
    assert_eq!(from_profile.mods.len(), 2);
    from_profile.save(&path).unwrap();
    let text = fs::read_to_string(&path).unwrap().to_lowercase();
    for forbidden in [
        "http",
        "github",
        "thunderstore",
        "url",
        "path",
        "game_dir",
        "/",
        "\\",
    ] {
        assert!(!text.contains(forbidden), "« {forbidden} » dans :\n{text}");
    }
    assert!(text.contains("melonloader"));
}

#[test]
fn refuse_un_fichier_avec_url_ou_chemin() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("x.toml");
    let sha = hash(1).to_string();
    for extra in ["url = \"https://evil.test/x.zip\"", "path = \"C:/x\""] {
        let text = format!(
            "format_version = 1\ngame_id = \"jeu-test\"\n[[mods]]\nid = \"a\"\nversion = \"1\"\nsha256 = \"{sha}\"\n{extra}\n"
        );
        fs::write(&path, text).unwrap();
        assert!(matches!(Lockfile::load(&path), Err(LockError::Toml(_))));
    }
}

#[test]
fn refuse_version_future_et_doublons() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("x.toml");
    fs::write(&path, "format_version = 9\nchamp_futur = 1\n").unwrap();
    assert!(matches!(
        Lockfile::load(&path),
        Err(LockError::UnsupportedVersion { found: 9, .. })
    ));

    let mut double = lock();
    double.mods.push(double.mods[0].clone());
    assert!(matches!(double.save(&path), Err(LockError::Invalid(_))));
    assert_eq!(lock().format_version, FORMAT_VERSION);
}

#[test]
fn fichier_absent_donne_une_erreur_io() {
    let tmp = tempfile::tempdir().unwrap();
    let err = Lockfile::load(&tmp.path().join("absent.toml")).unwrap_err();
    assert!(matches!(err, LockError::Io { .. }));
}

#[test]
fn installation_conforme() {
    assert!(lock().compare(Some(&matching_record())).is_match());
}

#[test]
fn jeu_non_installe_ou_autre_jeu() {
    assert_eq!(
        lock().compare(None).mismatches,
        vec![Mismatch::NotInstalled]
    );
    let mut record = matching_record();
    record.game_id = "autre".into();
    assert_eq!(
        lock().compare(Some(&record)).mismatches,
        vec![Mismatch::WrongGame {
            locked: "jeu-test".into(),
            installed: "autre".into()
        }]
    );
}

#[test]
fn installation_a_moitie_faite() {
    let mut record = matching_record();
    record.state = RecordState::InProgress;
    assert_eq!(
        lock().compare(Some(&record)).mismatches,
        vec![Mismatch::InstallIncomplete]
    );
}

#[test]
fn ecarts_de_mods_dans_un_ordre_stable() {
    let mut record = matching_record();
    // melonloader : autre version ; le-mod : retiré ; deux mods en trop (désordre voulu).
    record.steps = vec![
        step("zeta", "1", hash(9)),
        step("melonloader", "0.8.0", hash(1)),
        step("alpha", "1", hash(8)),
    ];
    let report = lock().compare(Some(&record));
    assert_eq!(
        report.mismatches,
        vec![
            Mismatch::VersionDiffers {
                id: "melonloader".into(),
                locked: "0.7.3".into(),
                installed: "0.8.0".into()
            },
            Mismatch::Missing {
                id: "le-mod".into()
            },
            Mismatch::Extra { id: "alpha".into() },
            Mismatch::Extra { id: "zeta".into() },
        ]
    );
    assert!(!report.is_match());
}

#[test]
fn meme_version_mais_hash_different() {
    let mut record = matching_record();
    record.steps[1].sha256 = hash(77);
    assert_eq!(
        lock().compare(Some(&record)).mismatches,
        vec![Mismatch::HashDiffers {
            id: "le-mod".into(),
            locked: hash(2),
            installed: hash(77)
        }]
    );
}
