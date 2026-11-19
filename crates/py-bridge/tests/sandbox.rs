//! Tests du bac à sable du worker (ADR 0034), avec le faux worker Rust.
//!
//! Sous Windows, le faux worker est VRAIMENT lancé en intégrité basse (copie étiquetée) :
//! on vérifie où il peut écrire. Il « devine » son niveau en essayant d'écrire dans un
//! dossier non étiqueté (`RAFTUP_FAKE_PROBE_DIR`), sans `unsafe`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(windows)]
use py_bridge::BridgeError;
use py_bridge::{BridgeOptions, PyBridge, Sandbox};
#[cfg(windows)]
use serde_json::json;

static NEXT: AtomicU32 = AtomicU32::new(0);

/// Dossier neuf sous le `target` des tests, avec `travail/` et `dehors/`.
fn fresh_dirs() -> (PathBuf, PathBuf, PathBuf) {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("bac-a-sable-{}-{n}", std::process::id()));
    let work = root.join("travail");
    let outside = root.join("dehors");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    (root, work, outside)
}

fn options(work: &PathBuf, probe: Option<&PathBuf>) -> BridgeOptions {
    let mut o = BridgeOptions::new(env!("CARGO_BIN_EXE_fake-worker"));
    o.sandbox = Some(Sandbox::new(work));
    if let Some(dir) = probe {
        o.env
            .insert("RAFTUP_FAKE_PROBE_DIR".into(), dir.display().to_string());
    }
    o
}

#[cfg(windows)]
#[test]
fn le_worker_n_ecrit_que_dans_son_dossier() {
    let (root, work, outside) = fresh_dirs();
    let mut b = PyBridge::start(options(&work, Some(&outside))).unwrap();
    assert!(b.sandbox_applied());
    assert_eq!(b.version().unwrap().integrity.as_deref(), Some("low"));
    assert!(work.join("tmp").is_dir(), "dossier temporaire créé");

    let inside = work.join("dedans.txt");
    b.call("write", json!({"path": inside.display().to_string()}))
        .unwrap();
    assert!(inside.is_file());
    // Même un sous-dossier créé AVANT l'étiquetage est inscriptible.
    b.call(
        "write",
        json!({"path": work.join("tmp").join("t.txt").display().to_string()}),
    )
    .unwrap();

    let err = b
        .call(
            "write",
            json!({"path": outside.join("non.txt").display().to_string()}),
        )
        .unwrap_err();
    assert!(matches!(err, BridgeError::Worker(_)), "{err:?}");
    assert!(!outside.join("non.txt").exists());
    b.shutdown().unwrap();

    // Après l'arrêt, le dossier n'est plus inscriptible par un AUTRE worker en bac à sable.
    let (root2, work2, outside2) = fresh_dirs();
    let mut b2 = PyBridge::start(options(&work2, Some(&outside2))).unwrap();
    assert!(b2
        .call(
            "write",
            json!({"path": work.join("plus-tard.txt").display().to_string()})
        )
        .is_err());
    b2.shutdown().unwrap();

    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(root2).unwrap();
}

#[cfg(windows)]
#[test]
fn un_worker_qui_n_est_pas_en_integrite_basse_est_refuse() {
    let (root, work, _outside) = fresh_dirs();
    // Sonde dans le dossier de travail (étiqueté bas) : l'écriture passe, le faux worker
    // annonce « medium », comme un worker que le bac à sable n'aurait pas atteint.
    let err = PyBridge::start(options(&work, Some(&work))).err().unwrap();
    assert!(
        matches!(&err, BridgeError::SandboxNotApplied { integrity: Some(l) } if l == "medium"),
        "{err:?}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn un_worker_muet_sur_son_integrite_est_refuse() {
    let (root, work, _outside) = fresh_dirs();
    let err = PyBridge::start(options(&work, None)).err().unwrap();
    assert!(
        matches!(err, BridgeError::SandboxNotApplied { integrity: None }),
        "{err:?}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn python_absent_avec_bac_a_sable() {
    let (root, work, _outside) = fresh_dirs();
    let mut o = BridgeOptions::new(PathBuf::from("Z:/nexiste/pas/python.exe"));
    o.sandbox = Some(Sandbox::new(&work));
    assert!(matches!(
        PyBridge::start(o),
        Err(BridgeError::PythonNotFound { .. })
    ));
    std::fs::remove_dir_all(root).unwrap();
}

/// Hors Windows : repli documenté, le worker démarre sans bac à sable.
#[cfg(not(windows))]
#[test]
fn repli_hors_windows() {
    let (root, work, _outside) = fresh_dirs();
    let b = PyBridge::start(options(&work, None)).unwrap();
    assert!(!b.sandbox_applied());
    assert!(work.join("tmp").is_dir());
    b.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
