//! Diagnostic des échecs de connexion, avec le vrai serveur lancé dans le même processus.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

use client_core::{Client, ClientError, ConnectOptions};
use launcher_core::diagnose::{
    check_connection, check_connection_with_timeout, explain, ConnectionProblem,
};
use launcher_core::{Encryption, Host, HostOptions};

fn seed_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago")
}

/// Démarre une partie sur la seed MIT ; renvoie l'hôte (à garder en vie) et son port.
async fn start_host(encryption: Encryption, password: Option<&str>) -> (Host, u16) {
    let mut host = Host::new();
    let status = host
        .start(HostOptions {
            seed: seed_path(),
            port: 0,
            password: password.map(str::to_string),
            encryption,
            public_host: "127.0.0.1".to_string(),
            disable_save: true,
        })
        .await
        .unwrap();
    let port = status.address.rsplit(':').next().unwrap().parse().unwrap();
    (host, port)
}

/// Chaque problème a une phrase en français, non vide, qui ne contient pas de jargon anglais brut.
fn assert_french(problem: &ConnectionProblem) {
    let text = problem.message();
    assert!(text.len() > 20, "{text}");
    assert!(
        text.contains(' ') && text.ends_with(['.', ')']),
        "pas une phrase : {text}"
    );
}

#[tokio::test]
async fn connexion_reussie() {
    let (_host, port) = start_host(Encryption::Plain, None).await;
    let result = check_connection(
        &format!("ws://127.0.0.1:{port}"),
        "Alice",
        None,
        "ChecksFinder",
    )
    .await;
    assert_eq!(result, Ok(()));
}

#[tokio::test]
async fn mauvais_slot() {
    let (_host, port) = start_host(Encryption::Plain, None).await;
    let problem = check_connection(
        &format!("ws://127.0.0.1:{port}"),
        "Personne",
        None,
        "ChecksFinder",
    )
    .await
    .unwrap_err();
    assert_eq!(problem, ConnectionProblem::InvalidSlot);
    assert_french(&problem);
}

#[tokio::test]
async fn mauvais_jeu() {
    let (_host, port) = start_host(Encryption::Plain, None).await;
    let problem = check_connection(
        &format!("ws://127.0.0.1:{port}"),
        "Alice",
        None,
        "Pokemon Red",
    )
    .await
    .unwrap_err();
    assert_eq!(problem, ConnectionProblem::InvalidGame);
    assert_french(&problem);
}

#[tokio::test]
async fn mauvais_mot_de_passe() {
    let (_host, port) = start_host(Encryption::Plain, Some("secret")).await;
    let address = format!("ws://127.0.0.1:{port}");
    for password in [None, Some("faux")] {
        let problem = check_connection(&address, "Alice", password, "ChecksFinder")
            .await
            .unwrap_err();
        assert_eq!(problem, ConnectionProblem::InvalidPassword);
        assert_french(&problem);
    }
    // Le bon mot de passe passe.
    assert_eq!(
        check_connection(&address, "Alice", Some("secret"), "ChecksFinder").await,
        Ok(())
    );
}

#[tokio::test]
async fn port_ferme() {
    // On réserve un port libre, puis on le libère : plus rien n'y écoute.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    for address in [
        format!("ws://127.0.0.1:{port}"),
        format!("127.0.0.1:{port}"),
    ] {
        let problem = check_connection(&address, "Alice", None, "ChecksFinder")
            .await
            .unwrap_err();
        assert_eq!(problem, ConnectionProblem::PortClosed, "{address}");
        assert_french(&problem);
    }
}

#[tokio::test]
async fn adresse_invalide() {
    let problem = check_connection("http://exemple.fr", "Alice", None, "ChecksFinder")
        .await
        .unwrap_err();
    assert_eq!(problem, ConnectionProblem::BadAddress);
    assert_french(&problem);
    assert_eq!(
        check_connection("", "Alice", None, "ChecksFinder").await,
        Err(ConnectionProblem::BadAddress)
    );
}

#[tokio::test]
async fn delai_depasse() {
    // Un écouteur qui n'accepte jamais : la connexion TCP s'ouvre (file d'attente du
    // système) mais personne ne répond au WebSocket.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let problem = check_connection_with_timeout(
        &format!("ws://127.0.0.1:{port}"),
        "Alice",
        None,
        "ChecksFinder",
        Duration::from_millis(500),
    )
    .await
    .unwrap_err();
    assert_eq!(problem, ConnectionProblem::Timeout);
    assert_french(&problem);
    drop(listener);
}

/// Écrit un certificat auto-signé pour 127.0.0.1 dans un dossier temporaire.
fn write_self_signed(dir: &std::path::Path) -> Encryption {
    let names = vec!["127.0.0.1".to_string(), "localhost".to_string()];
    let certified = rcgen::generate_simple_self_signed(names).unwrap();
    let cert = dir.join("cert.pem");
    let key = dir.join("key.pem");
    std::fs::write(&cert, certified.cert.pem()).unwrap();
    std::fs::write(&key, certified.key_pair.serialize_pem()).unwrap();
    Encryption::Wss { cert, key }
}

#[tokio::test]
async fn certificat_auto_signe_donne_tls() {
    let tmp = tempfile::tempdir().unwrap();
    let (_host, port) = start_host(write_self_signed(tmp.path()), None).await;

    // Adresse explicite en wss://.
    let problem = check_connection(
        &format!("wss://127.0.0.1:{port}"),
        "Alice",
        None,
        "ChecksFinder",
    )
    .await
    .unwrap_err();
    assert_eq!(problem, ConnectionProblem::Tls);
    assert_french(&problem);

    // Adresse sans schéma : wss:// puis ws://. Le repli ne doit pas cacher l'erreur TLS.
    let problem = check_connection(&format!("127.0.0.1:{port}"), "Alice", None, "ChecksFinder")
        .await
        .unwrap_err();
    assert_eq!(problem, ConnectionProblem::Tls);
}

/// Documente le comportement de `client-core` seul : avec le repli wss vers ws,
/// quelle erreur remonte quand le serveur est en wss avec un certificat inconnu ?
#[tokio::test]
async fn repli_de_client_core_sur_une_erreur_tls() {
    let tmp = tempfile::tempdir().unwrap();
    let (_host, port) = start_host(write_self_signed(tmp.path()), None).await;
    let options = ConnectOptions::new(&format!("127.0.0.1:{port}"), "Alice", "ChecksFinder");
    let error = match Client::connect(&options).await {
        Ok(_) => panic!("la connexion aurait dû échouer"),
        Err(error) => error,
    };
    let problem = explain(&error);
    // Ce que client-core renvoie vraiment est affiché pour le rapport.
    println!("client-core seul : {error:?} -> {problem:?}");
    assert!(
        !matches!(error, ClientError::Refused(_)),
        "le serveur n'a pas à refuser : {error:?}"
    );
}

#[test]
fn chaque_probleme_a_une_phrase() {
    for problem in [
        ConnectionProblem::BadAddress,
        ConnectionProblem::Unreachable,
        ConnectionProblem::PortClosed,
        ConnectionProblem::Tls,
        ConnectionProblem::Timeout,
        ConnectionProblem::InvalidSlot,
        ConnectionProblem::InvalidGame,
        ConnectionProblem::IncompatibleVersion,
        ConnectionProblem::InvalidPassword,
        ConnectionProblem::InvalidItemsHandling,
        ConnectionProblem::Other("détail".into()),
    ] {
        assert_french(&problem);
    }
}
