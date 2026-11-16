//! Héberger une partie depuis le launcher (ADR 0010), sur la seed MIT.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use launcher_core::{Encryption, Host, HostOptions, LauncherError, SeedSummary};
use server::GameEvent;
use tokio_tungstenite::tungstenite::Message;

fn seed_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago")
}

fn plain(port: u16) -> HostOptions {
    HostOptions {
        seed: seed_path(),
        port,
        password: None,
        encryption: Encryption::Plain,
        public_host: "localhost".to_string(),
    }
}

#[test]
fn resume_de_la_seed() {
    let (summary, _) = SeedSummary::load(&seed_path()).unwrap();
    assert_eq!(summary.seed_name, "72844992796894532502");
    assert_eq!(summary.generator_version, "0.6.8");
    assert_eq!(summary.players, 3);
    let lien = &summary.slots[3];
    assert!(lien.is_group);
    assert_eq!(lien.members, ["Alice", "Bob"]);
    assert_eq!(summary.slots[0].locations, 25);
}

#[test]
fn seed_absente() {
    let error = SeedSummary::load(&PathBuf::from("n-existe-pas.archipelago")).unwrap_err();
    assert!(matches!(error, LauncherError::Read { .. }), "{error}");
}

#[tokio::test]
async fn heberger_suivre_arreter() {
    let mut host = Host::new();
    let status = host.start(plain(0)).await.unwrap();
    assert!(
        status.address.starts_with("ws://localhost:"),
        "{}",
        status.address
    );
    assert!(!status.encrypted);
    assert!(!status.has_password);
    let port: u16 = status.address.rsplit(':').next().unwrap().parse().unwrap();

    // Une deuxième partie en même temps est refusée.
    let error = host.start(plain(0)).await.unwrap_err();
    assert!(matches!(error, LauncherError::AlreadyRunning));

    // Un joueur rejoint : l'événement arrive dans le journal du launcher.
    let (mut alice, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}"))
        .await
        .unwrap();
    alice.next().await.unwrap().unwrap();
    let connect = r#"[{"cmd":"Connect","password":null,"game":"ChecksFinder","name":"Alice",
        "uuid":"u","version":{"major":0,"minor":6,"build":8,"class":"Version"},
        "items_handling":7,"tags":[]}]"#;
    alice.send(Message::text(connect)).await.unwrap();
    let mut joined = false;
    for _ in 0..100 {
        let events = host.events_since(0);
        if events
            .iter()
            .any(|e| matches!(&e.event, GameEvent::Joined { name, .. } if name == "Alice"))
        {
            joined = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(joined, "événement Joined non reçu");
    let status = host.status().await.unwrap();
    assert_eq!(status.server.slots[0].connections, 1);

    // Arrêt, puis on peut relancer sur le même port.
    host.stop().await.unwrap();
    assert!(!host.is_running());
    assert!(host.status().await.is_none());
    let again = host.start(plain(port)).await.unwrap();
    assert!(again.address.ends_with(&format!(":{port}")));
    host.stop().await.unwrap();
}

#[tokio::test]
async fn mot_de_passe_et_certificat_absent() {
    let mut host = Host::new();
    let mut options = plain(0);
    options.password = Some("secret".to_string());
    let status = host.start(options).await.unwrap();
    assert!(status.has_password);
    host.stop().await.unwrap();

    let mut options = plain(0);
    options.encryption = Encryption::Wss {
        cert: PathBuf::from("n-existe-pas/fullchain.pem"),
        key: PathBuf::from("n-existe-pas/privkey.pem"),
    };
    let error = host.start(options).await.unwrap_err();
    assert!(matches!(error, LauncherError::Tls(_)), "{error}");
    assert!(!host.is_running());
}
