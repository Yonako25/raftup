//! Statut « prêt ? » d'un slot, avec le vrai serveur lancé dans le même processus (F08).

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

use launcher_core::readiness::{check_slot, check_slot_with_timeout};
use launcher_core::{Encryption, Host, HostOptions, Readiness};

fn seed_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago")
}

async fn start_host(encryption: Encryption, password: Option<&str>) -> (Host, String) {
    let mut host = Host::new();
    let status = host
        .start(HostOptions {
            seed: seed_path(),
            port: 0,
            password: password.map(str::to_string),
            encryption,
            public_host: "127.0.0.1".to_string(),
            disable_save: true,
            webhook: None,
        })
        .await
        .unwrap();
    (host, status.address)
}

#[tokio::test]
async fn slot_pret_puis_toujours_pret() {
    let (host, address) = start_host(Encryption::Plain, None).await;
    // Deux tests de suite : la connexion de test est bien refermée.
    for _ in 0..2 {
        let result = check_slot(&address, "Alice", "ChecksFinder", None).await;
        assert_eq!(result, Readiness::Ready);
        assert!(result.is_ready());
    }
    // Le serveur voit la fermeture avec un peu de retard : on attend au plus 3 s.
    let mut connections = usize::MAX;
    for _ in 0..30 {
        connections = host.status().await.unwrap().server.slots[0].connections;
        if connections == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(connections, 0);
}

#[tokio::test]
async fn mauvais_slot_et_mauvais_jeu() {
    let (_host, address) = start_host(Encryption::Plain, None).await;
    let r = check_slot(&address, "Personne", "ChecksFinder", None).await;
    assert!(matches!(r, Readiness::WrongSlot(_)), "{r:?}");
    assert_eq!(r.label(), "Mauvais slot");
    let r = check_slot(&address, "Alice", "Pokemon Red", None).await;
    assert!(matches!(r, Readiness::WrongSlot(_)), "{r:?}");
    assert!(r.message().contains("slot"));
}

#[tokio::test]
async fn mot_de_passe_refuse_puis_accepte() {
    let (_host, address) = start_host(Encryption::Plain, Some("secret")).await;
    let r = check_slot(&address, "Alice", "ChecksFinder", None).await;
    assert!(matches!(r, Readiness::Refused(_)), "{r:?}");
    assert!(r.message().contains("Mot de passe"));
    assert_eq!(
        check_slot(&address, "Alice", "ChecksFinder", Some("secret")).await,
        Readiness::Ready
    );
}

#[tokio::test]
async fn certificat_auto_signe_donne_tls() {
    let tmp = tempfile::tempdir().unwrap();
    let certified = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
    let cert = tmp.path().join("cert.pem");
    let key = tmp.path().join("key.pem");
    std::fs::write(&cert, certified.cert.pem()).unwrap();
    std::fs::write(&key, certified.key_pair.serialize_pem()).unwrap();
    let (_host, address) = start_host(Encryption::Wss { cert, key }, None).await;
    let r = check_slot(&address, "Alice", "ChecksFinder", None).await;
    assert_eq!(r, Readiness::Tls);
}

#[tokio::test]
async fn delai_depasse_et_port_ferme() {
    // Un port qui accepte la connexion TCP mais ne répond jamais : délai dépassé.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let r = check_slot_with_timeout(
        &format!("ws://127.0.0.1:{port}"),
        "Alice",
        "ChecksFinder",
        None,
        Duration::from_millis(500),
    )
    .await;
    assert_eq!(r, Readiness::TimedOut);
    drop(listener);

    // Plus rien n'écoute : serveur injoignable, avec une phrase en français.
    let r = check_slot(
        &format!("ws://127.0.0.1:{port}"),
        "Alice",
        "ChecksFinder",
        None,
    )
    .await;
    assert!(matches!(r, Readiness::Unreachable(_)), "{r:?}");
    assert!(r.message().contains("port"));
}
