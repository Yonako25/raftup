//! Tests du pont avec le faux worker Rust (sans Python).
//!
//! Le faux worker répond normalement au premier appel (`version`, la poignée de main de
//! `PyBridge::start`) puis applique son mode : tous les tests passent donc par `start`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use py_bridge::{BridgeError, BridgeOptions, CancelToken, PyBridge};
use serde_json::{json, Value};

fn options(mode: &str) -> BridgeOptions {
    let mut o = BridgeOptions::new(env!("CARGO_BIN_EXE_fake-worker"));
    o.env.insert("RAFTUP_FAKE_WORKER".into(), mode.into());
    o
}

/// Options où le mode défaillant s'applique dès la poignée de main (échec dans `start`).
fn options_at_start(mode: &str) -> BridgeOptions {
    let mut o = options(mode);
    o.env
        .insert("RAFTUP_FAKE_WORKER_AFTER_HANDSHAKE".into(), "0".into());
    o
}

fn start(mode: &str) -> PyBridge {
    PyBridge::start(options(mode)).unwrap()
}

#[test]
fn start_ping_version() {
    let mut b = start("ok");
    b.ping().unwrap();
    let v = b.version().unwrap();
    assert_eq!(v.protocol, 1);
    assert_eq!(v.implementation, "Fake");
    assert_eq!(b.call("echo", json!({"a": 1})).unwrap(), json!({"a": 1}));
    assert!(b.is_alive());
}

#[test]
fn chemin_inexistant() {
    let o = BridgeOptions::new(PathBuf::from("Z:/nexiste/pas/python.exe"));
    assert!(matches!(
        PyBridge::start(o),
        Err(BridgeError::PythonNotFound { .. })
    ));
}

#[cfg(windows)]
#[test]
fn bat_et_cmd_refuses_sous_windows() {
    for nom in ["Z:/nexiste/pas/python.bat", "Z:/nexiste/pas/PYTHON.CMD"] {
        let o = BridgeOptions::new(PathBuf::from(nom));
        assert!(
            matches!(
                PyBridge::start(o),
                Err(BridgeError::UnsupportedPython { .. })
            ),
            "{nom} aurait dû être refusé"
        );
    }
}

#[test]
fn garbage() {
    assert!(matches!(
        PyBridge::start(options_at_start("garbage")),
        Err(BridgeError::InvalidJson { .. })
    ));
    let mut b = start("garbage");
    assert!(matches!(b.ping(), Err(BridgeError::InvalidJson { .. })));
    assert!(matches!(b.ping(), Err(BridgeError::Closed)));
    assert!(!b.is_alive());
}

#[test]
fn huge() {
    let mut b = start("huge");
    assert!(matches!(b.ping(), Err(BridgeError::LineTooLong { .. })));
    assert!(matches!(b.ping(), Err(BridgeError::Closed)));
}

#[test]
fn crash_avec_stderr() {
    let mut b = start("crash");
    match b.ping() {
        Err(BridgeError::ProcessExited { stderr_tail, .. }) => {
            assert!(
                stderr_tail.contains("boom"),
                "stderr_tail = {stderr_tail:?}"
            )
        }
        other => panic!("attendu ProcessExited, reçu {other:?}"),
    }
    assert!(matches!(b.ping(), Err(BridgeError::Closed)));
}

#[test]
fn crash_au_demarrage() {
    assert!(matches!(
        PyBridge::start(options("crash-on-start")),
        Err(BridgeError::ProcessExited { .. })
    ));
}

#[test]
fn silent_timeout() {
    let mut o = options("silent");
    o.call_timeout = Duration::from_millis(300);
    let mut b = PyBridge::start(o).unwrap();
    let t = Instant::now();
    assert!(matches!(b.ping(), Err(BridgeError::Timeout { .. })));
    assert!(t.elapsed() < Duration::from_secs(2));
    assert!(!b.is_alive());
    assert!(matches!(b.ping(), Err(BridgeError::Closed)));
}

#[test]
fn silent_timeout_au_demarrage() {
    let mut o = options_at_start("silent");
    o.startup_timeout = Duration::from_millis(300);
    assert!(matches!(
        PyBridge::start(o),
        Err(BridgeError::Timeout { .. })
    ));
}

#[test]
fn mauvais_id() {
    assert!(matches!(
        PyBridge::start(options_at_start("wrong-id")),
        Err(BridgeError::UnexpectedId { .. })
    ));
    // Après la poignée de main (id 1), le premier ping porte l'id 2.
    let mut b = start("wrong-id");
    assert!(matches!(
        b.ping(),
        Err(BridgeError::UnexpectedId {
            expected: 2,
            got: Some(102)
        })
    ));
    assert!(matches!(b.ping(), Err(BridgeError::Closed)));
}

#[test]
fn erreur_du_worker_laisse_le_pont_utilisable() {
    let mut b = start("error");
    match b.ping() {
        Err(BridgeError::Worker(e)) => assert_eq!(e.code, -32000),
        other => panic!("attendu Worker, reçu {other:?}"),
    }
    assert!(b.is_alive());
    // `null` est envoyé comme `{}` (le worker exige un objet).
    assert_eq!(b.call("echo", Value::Null).unwrap(), json!({}));
}

#[test]
fn id_null_avec_erreur_garde_le_message_et_ferme() {
    let mut b = start("null-id-error");
    match b.ping() {
        Err(BridgeError::Protocol(e)) => {
            assert_eq!(e.code, -32700);
            assert_eq!(e.message, "JSON illisible");
        }
        other => panic!("attendu Protocol, reçu {other:?}"),
    }
    assert!(matches!(b.ping(), Err(BridgeError::Closed)));
}

#[test]
fn result_null_traverse_le_pont() {
    let mut b = start("null");
    assert_eq!(b.call("ping", Value::Null).unwrap(), Value::Null);
    assert!(b.is_alive());
}

#[test]
fn requete_trop_grande_est_refusee_sans_fermer() {
    let mut o = options("ok");
    o.max_line_bytes = 1024;
    let mut b = PyBridge::start(o).unwrap();
    let gros = json!({"x": "a".repeat(2000)});
    match b.call("echo", gros) {
        Err(BridgeError::RequestTooLarge { max: 1024, len }) => assert!(len > 2000),
        other => panic!("attendu RequestTooLarge, reçu {other:?}"),
    }
    // Rien n'a été écrit : le pont reste utilisable.
    assert!(b.is_alive());
    b.ping().unwrap();
}

#[test]
fn worker_qui_ne_lit_pas_stdin_donne_timeout_sans_bloquer() {
    let mut o = options("no-read");
    o.max_line_bytes = 4 * 1024 * 1024;
    o.call_timeout = Duration::from_millis(500);
    let mut b = PyBridge::start(o).unwrap();
    // 2 Mio : bien plus que la capacité du pipe, l'écriture ne peut pas finir.
    let gros = json!({"x": "a".repeat(2 * 1024 * 1024)});
    let t = Instant::now();
    assert!(matches!(
        b.call("echo", gros),
        Err(BridgeError::Timeout { .. })
    ));
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "a bloqué trop longtemps"
    );
    assert!(!b.is_alive());
    assert!(matches!(b.ping(), Err(BridgeError::Closed)));
}

#[test]
fn version_mal_formee_apres_demarrage_ferme_le_pont() {
    let mut b = start("bad-version");
    assert!(matches!(b.version(), Err(BridgeError::InvalidResponse(_))));
    assert!(!b.is_alive());
    assert!(matches!(b.ping(), Err(BridgeError::Closed)));
}

#[test]
fn ancien_protocole() {
    assert!(matches!(
        PyBridge::start(options("old-protocol")),
        Err(BridgeError::IncompatibleWorker {
            expected: 1,
            got: 0
        })
    ));
}

#[test]
fn stderr_bruyant_ne_bloque_pas() {
    let mut b = start("noisy-stderr");
    b.ping().unwrap();
    b.ping().unwrap();
}

#[test]
fn stderr_borne_apres_beaucoup_de_bruit() {
    // 2 Mio de bruit puis un crash : la fin de stderr reste bornée (8 Kio, 50 lignes).
    let mut b = start("noisy-crash");
    match b.ping() {
        Err(BridgeError::ProcessExited { stderr_tail, .. }) => {
            assert!(stderr_tail.len() <= 8 * 1024, "len = {}", stderr_tail.len());
            assert!(stderr_tail.lines().count() <= 50);
            assert!(stderr_tail.contains("boom"));
        }
        other => panic!("attendu ProcessExited, reçu {other:?}"),
    }
}

#[test]
fn shutdown_normal() {
    let b = start("ok");
    let status = b.shutdown().unwrap();
    assert!(status.success());
}

#[test]
fn ignore_eof_shutdown_et_drop_rapides() {
    let t = Instant::now();
    let mut o = options("ignore-eof");
    o.shutdown_grace = Duration::from_millis(300);
    let b = PyBridge::start(o).unwrap();
    let _ = b.shutdown().unwrap();
    assert!(t.elapsed() < Duration::from_secs(2));

    let t = Instant::now();
    let b = start("ignore-eof");
    drop(b);
    assert!(t.elapsed() < Duration::from_secs(2));
}

/// Lance `cancel()` depuis un autre thread après `delay`.
fn cancel_later(token: &CancelToken, delay: Duration) -> std::thread::JoinHandle<()> {
    let token = token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        token.cancel();
    })
}

#[test]
fn annulation_rapide_puis_pont_ferme() {
    let mut b = start("slow");
    let token = CancelToken::default();
    let handle = cancel_later(&token, Duration::from_millis(300));
    let t0 = Instant::now();
    let r = b.call_cancellable("ping", Value::Null, Duration::from_secs(30), &token);
    let elapsed = t0.elapsed();
    handle.join().unwrap();
    assert!(matches!(r, Err(BridgeError::Cancelled)), "{r:?}");
    assert!(elapsed < Duration::from_secs(1), "trop lent : {elapsed:?}");
    assert!(matches!(b.ping(), Err(BridgeError::Closed)));
    assert!(!b.is_alive());
}

#[test]
fn call_cancellable_timeout_comme_call_with_timeout() {
    let mut b = start("slow");
    let token = CancelToken::default();
    let r = b.call_cancellable("ping", Value::Null, Duration::from_millis(250), &token);
    assert!(matches!(r, Err(BridgeError::Timeout { .. })), "{r:?}");
}

#[test]
fn stderr_lines_recoit_les_lignes_dans_l_ordre() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut o = options("stderr-progress");
    o.stderr_lines = Some(tx);
    let mut b = PyBridge::start(o).unwrap();
    b.ping().unwrap();
    let mut got = Vec::new();
    for _ in 0..3 {
        got.push(rx.recv_timeout(Duration::from_secs(2)).unwrap());
    }
    assert_eq!(got, ["étape 1", "étape 2", "étape 3"]);
}

#[test]
fn stderr_lines_recepteur_disparu_ne_bloque_pas() {
    let (tx, rx) = std::sync::mpsc::channel();
    drop(rx);
    let mut o = options("stderr-progress");
    o.stderr_lines = Some(tx);
    let mut b = PyBridge::start(o).unwrap();
    b.ping().unwrap();
}

#[cfg(windows)]
#[test]
fn annulation_tue_le_petit_fils() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut o = options("spawn-grandchild");
    o.stderr_lines = Some(tx);
    let mut b = PyBridge::start(o).unwrap();
    let token = CancelToken::default();
    let handle = cancel_later(&token, Duration::from_millis(500));
    let r = b.call_cancellable("ping", Value::Null, Duration::from_secs(30), &token);
    handle.join().unwrap();
    assert!(matches!(r, Err(BridgeError::Cancelled)), "{r:?}");
    // Le canal se ferme quand stderr atteint EOF, donc quand plus aucun process (worker
    // comme petit-fils) ne tient le pipe ouvert.
    let t0 = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("le petit-fils vit encore (pas d'EOF sur stderr)")
            }
        }
        assert!(t0.elapsed() < Duration::from_secs(2), "EOF trop tardif");
    }
}

#[test]
fn drop_ne_bloque_pas() {
    let b = start("spawn-grandchild");
    let t0 = Instant::now();
    drop(b);
    assert!(t0.elapsed() < Duration::from_secs(2));
}
