//! Chiffrement `wss` du serveur (J1, étape 8 ; NF06, ADR 0009).
//!
//! Chaque test crée un certificat auto-signé avec `rcgen` : aucune clé privée
//! n'est rangée dans le dépôt. Le client de test fait confiance à ce seul
//! certificat.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use multidata::{load_archipelago, ArchiveLimits};
use server::{Room, Server, TlsConfig, TlsError};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Un certificat auto-signé pour `127.0.0.1` : (certificat PEM, clé PEM, certificat DER).
fn self_signed() -> (String, String, Vec<u8>) {
    let names = vec!["127.0.0.1".to_string(), "localhost".to_string()];
    let certified = rcgen::generate_simple_self_signed(names).unwrap();
    (
        certified.cert.pem(),
        certified.key_pair.serialize_pem(),
        certified.cert.der().to_vec(),
    )
}

fn seed_mit() -> Room {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago");
    let data = load_archipelago(&std::fs::read(path).unwrap(), &ArchiveLimits::default()).unwrap();
    Room::new(data)
}

/// Lance un serveur `wss` ; renvoie son port et le certificat (DER) à croire.
async fn start_tls() -> (u16, Vec<u8>) {
    let (cert, key, der) = self_signed();
    let tls = TlsConfig::from_pem(cert.as_bytes(), key.as_bytes()).unwrap();
    let server = Server::bind("127.0.0.1:0", seed_mit())
        .await
        .unwrap()
        .with_tls(tls);
    let port = server.local_addr().unwrap().port();
    tokio::spawn(server.run());
    (port, der)
}

/// Client `wss` qui ne croit que ce certificat.
async fn connect_wss(port: u16, trusted: &[u8]) -> Result<Client, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(trusted.to_vec().into()).unwrap();
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = Connector::Rustls(Arc::new(config));
    let url = format!("wss://127.0.0.1:{port}");
    tokio_tungstenite::connect_async_tls_with_config(url, None, false, Some(connector))
        .await
        .map(|(client, _)| client)
        .map_err(|e| e.to_string())
}

/// Prochaine trame texte ; `None` si la connexion est fermée.
async fn next_text(client: &mut Client) -> Option<String> {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .unwrap();
        match message {
            Some(Ok(Message::Text(text))) => return Some(text.to_string()),
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => continue,
        }
    }
}

/// En `wss`, le client reçoit le `RoomInfo` puis joue normalement.
#[tokio::test]
async fn accueil_en_wss() {
    let (port, der) = start_tls().await;
    let mut client = connect_wss(port, &der).await.unwrap();
    let first = next_text(&mut client).await.unwrap();
    assert!(first.contains("RoomInfo"), "{first}");

    let request = r#"[{"cmd":"GetDataPackage","games":["Archipelago"]}]"#;
    client.send(Message::text(request)).await.unwrap();
    let reply = next_text(&mut client).await.unwrap();
    assert!(reply.contains("DataPackage"), "{reply}");
}

/// Un client en clair (`ws://`) sur un serveur `wss` échoue, sans gêner les autres.
#[tokio::test]
async fn client_en_clair_refuse() {
    let (port, der) = start_tls().await;
    let plain = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}")).await;
    assert!(plain.is_err(), "le clair aurait dû échouer");

    let mut client = connect_wss(port, &der).await.unwrap();
    assert!(next_text(&mut client).await.unwrap().contains("RoomInfo"));
}

/// Un client qui ne connaît pas le certificat (ici : certificats racine publics)
/// refuse la connexion. C'est ce que ferait un mod de jeu avec un certificat
/// auto-signé.
#[tokio::test]
async fn certificat_inconnu_refuse() {
    let (port, _) = start_tls().await;
    let result = tokio_tungstenite::connect_async(format!("wss://127.0.0.1:{port}")).await;
    let Err(error) = result else {
        panic!("le certificat aurait dû être refusé");
    };
    assert!(
        error.to_string().to_lowercase().contains("certificate"),
        "{error}"
    );
}

/// Les limites restent en place en `wss` : un message trop gros ferme la connexion.
#[tokio::test]
async fn limites_en_wss() {
    let (port, der) = start_tls().await;
    let mut client = connect_wss(port, &der).await.unwrap();
    next_text(&mut client).await.unwrap();

    let big = format!("[\"{}\"]", "a".repeat(1024 * 1024));
    let _ = client.send(Message::text(big)).await;
    let code = loop {
        match tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .unwrap()
        {
            Some(Ok(Message::Close(frame))) => break frame.map(|f| f.code),
            None | Some(Err(_)) => break None,
            Some(Ok(_)) => continue,
        }
    };
    assert_eq!(code, Some(CloseCode::Size));
}

// ----- Erreurs de configuration -----

#[test]
fn certificat_et_cle_inverses() {
    let (cert, key, _) = self_signed();
    // La clé à la place du certificat : pas de certificat dans le fichier.
    let error = TlsConfig::from_pem(key.as_bytes(), key.as_bytes())
        .err()
        .unwrap();
    assert!(matches!(error, TlsError::NoCertificate { .. }), "{error}");
    // Le certificat à la place de la clé : pas de clé dans le fichier.
    let error = TlsConfig::from_pem(cert.as_bytes(), cert.as_bytes())
        .err()
        .unwrap();
    assert!(matches!(error, TlsError::NoKey { .. }), "{error}");
}

#[test]
fn cle_d_un_autre_certificat() {
    let (cert, _, _) = self_signed();
    let (_, other_key, _) = self_signed();
    let error = TlsConfig::from_pem(cert.as_bytes(), other_key.as_bytes())
        .err()
        .unwrap();
    assert!(matches!(error, TlsError::Rejected(_)), "{error}");
}

#[test]
fn fichier_absent() {
    let missing = Path::new("n-existe-pas/fullchain.pem");
    let error = TlsConfig::from_pem_files(missing, missing).err().unwrap();
    assert!(matches!(error, TlsError::Read { .. }), "{error}");
    assert!(error.to_string().contains("fullchain.pem"), "{error}");
}

#[test]
fn fichiers_pem_valides() {
    let (cert, key, _) = self_signed();
    // Dossier temporaire de cargo, dans `target/` : rien n'est écrit ailleurs.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("raftup-tls");
    std::fs::create_dir_all(&dir).unwrap();
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, cert).unwrap();
    std::fs::write(&key_path, key).unwrap();
    assert!(TlsConfig::from_pem_files(&cert_path, &key_path).is_ok());
}
