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

    match b.call("methode_inconnue", json!({})) {
        Err(BridgeError::Worker(e)) => assert_eq!(e.code, -32601),
        other => panic!("attendu Worker(-32601), reçu {other:?}"),
    }

    let texte = json!({"texte": "été ✓"});
    assert_eq!(b.call("echo", texte.clone()).unwrap(), texte);

    let status = b.shutdown().unwrap();
    assert_eq!(status.code(), Some(0));
}
