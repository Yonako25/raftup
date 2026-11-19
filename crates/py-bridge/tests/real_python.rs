//! Tests avec un vrai Python. Lancer avec :
//! `RAFTUP_PYTHON=C:\...\python.exe cargo test -p py-bridge -- --ignored`
//! (par exemple C:\Python312\python.exe).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use py_bridge::{BridgeError, BridgeOptions, PyBridge};
use serde_json::json;

fn start() -> PyBridge {
    let python = std::env::var("RAFTUP_PYTHON").expect("RAFTUP_PYTHON non défini");
    PyBridge::start(BridgeOptions::new(python)).unwrap()
}

#[test]
#[ignore = "demande RAFTUP_PYTHON"]
fn version_echo_shutdown() {
    let mut b = start();
    let v = b.version().unwrap();
    assert!(v.python.starts_with("3."), "python = {}", v.python);
    assert_eq!(v.protocol, 1);
    // PYTHONHASHSEED=0 posé par le pont : hachage des chaînes reproductible (ADR 0028).
    assert_eq!(v.hash_randomization, Some(0));

    match b.call("methode_inconnue", json!({})) {
        Err(BridgeError::Worker(e)) => assert_eq!(e.code, -32601),
        other => panic!("attendu Worker(-32601), reçu {other:?}"),
    }

    let texte = json!({"texte": "été ✓"});
    assert_eq!(b.call("echo", texte.clone()).unwrap(), texte);

    let status = b.shutdown().unwrap();
    assert_eq!(status.code(), Some(0));
}

#[test]
#[ignore = "demande RAFTUP_PYTHON"]
fn generate_avec_faux_archipelago() {
    use py_bridge::{GenerateParams, ERR_ALREADY_USED};
    use std::time::Duration;

    let racine = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = std::env::temp_dir().join(format!("raftup-gen-test-{}", std::process::id()));
    let joueurs = tmp.join("joueurs");
    std::fs::create_dir_all(&joueurs).unwrap();
    std::fs::write(joueurs.join("p1.yaml"), "name: Alice\ngame: FakeGame\n").unwrap();

    let params = GenerateParams {
        archipelago_dir: racine.join("tests/fixtures/fake_archipelago"),
        site_packages: None,
        player_files_dir: joueurs,
        output_dir: tmp.join("out"),
        seed: Some(42),
        spoiler: 3,
        race: false,
    };
    let mut b = start();
    let r = b.generate(&params, Duration::from_secs(60)).unwrap();
    assert_eq!(r.seed_name, "42");
    assert_eq!(r.players.len(), 1);
    assert_eq!(r.players[0].name, "Alice");
    assert!(r.zip.is_file(), "zip absent : {}", r.zip.display());

    // Un seul generate par process : le second est refusé, le worker reste en vie.
    match b.generate(&params, Duration::from_secs(60)) {
        Err(BridgeError::Worker(e)) => assert_eq!(e.code, ERR_ALREADY_USED),
        other => panic!("attendu Worker(-32012), reçu {other:?}"),
    }
    b.ping().unwrap();
    b.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&tmp);
}
