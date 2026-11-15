//! Socle du serveur (J1, étape 3) : accueil des clients et lecture des trames.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::time::Duration;

use ap_protocol::{decode_server_frame, encode_server_frame, ServerPacket};
use futures_util::{SinkExt, StreamExt};
use multidata::{load_archipelago, ArchiveLimits};
use serde_json::Value;
use server::{Room, Server};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn seed_mit() -> Room {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago");
    let data = load_archipelago(&std::fs::read(path).unwrap(), &ArchiveLimits::default()).unwrap();
    Room::new(data)
}

/// Lance un serveur sur un port libre et renvoie son adresse `ws://`.
async fn start(room: Room) -> String {
    let server = Server::bind("127.0.0.1:0", room).await.unwrap();
    let address = server.local_addr().unwrap();
    tokio::spawn(server.run());
    format!("ws://{address}")
}

/// Se connecte et lit le `RoomInfo` d'accueil.
async fn connect(url: &str) -> (Client, Vec<ServerPacket>) {
    let (mut client, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let first = next_packets(&mut client).await.unwrap();
    (client, first)
}

/// Prochaine trame du serveur ; `None` si la connexion est fermée.
async fn next_packets(client: &mut Client) -> Option<Vec<ServerPacket>> {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .unwrap();
        match message {
            Some(Ok(Message::Text(text))) => return Some(decode_server_frame(&text).unwrap()),
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => continue,
        }
    }
}

/// Vérifie que le serveur n'envoie rien pendant un court instant.
async fn assert_silent(client: &mut Client) {
    let waited = tokio::time::timeout(Duration::from_millis(300), client.next()).await;
    assert!(waited.is_err(), "réponse inattendue : {waited:?}");
}

/// Le `RoomInfo` a les mêmes valeurs que celui du serveur officiel
/// 0.6.8 sur la même seed (capturé dans `tests/data/roominfo-seed-mit.json`).
/// Seul l'ordre des clés peut différer : il ne compte pas en JSON.
#[test]
fn room_info_comme_python() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/roominfo-seed-mit.json");
    let expected = std::fs::read_to_string(path).unwrap();
    let expected = expected.trim_end();
    // On reprend l'heure de la capture, la seule valeur qui change.
    let parsed: Value = serde_json::from_str(expected).unwrap();
    let time = parsed[0]["time"].as_f64().unwrap();

    let ours = seed_mit().room_info(time);
    let frame = encode_server_frame(&[ServerPacket::RoomInfo(ours)]).unwrap();
    let ours: Value = serde_json::from_str(&frame).unwrap();
    assert_eq!(ours, parsed);
}

#[test]
fn mot_de_passe_annonce() {
    let room = seed_mit().with_password(Some("secret".to_string()));
    assert!(room.room_info(0.0).password);
    // Un mot de passe vide revient à ne pas en avoir.
    let room = seed_mit().with_password(Some(String::new()));
    assert!(!room.room_info(0.0).password);
}

#[tokio::test]
async fn accueil_par_room_info() {
    let url = start(seed_mit()).await;
    let (_client, first) = connect(&url).await;
    assert_eq!(first.len(), 1);
    let ServerPacket::RoomInfo(info) = &first[0] else {
        panic!("RoomInfo attendu : {first:?}");
    };
    assert_eq!(info.seed_name, "72844992796894532502");
    assert!(info.time > 1_700_000_000.0);
}

#[tokio::test]
async fn plusieurs_clients_en_meme_temps() {
    let url = start(seed_mit()).await;
    let (_a, first_a) = connect(&url).await;
    let (_b, first_b) = connect(&url).await;
    assert!(matches!(first_a[0], ServerPacket::RoomInfo(_)));
    assert!(matches!(first_b[0], ServerPacket::RoomInfo(_)));
}

/// Avant `Connect`, le serveur officiel ignore les paquets sans répondre.
#[tokio::test]
async fn paquet_avant_connexion_ignore() {
    let url = start(seed_mit()).await;
    let (mut client, _) = connect(&url).await;
    client
        .send(Message::text(r#"[{"cmd":"Sync"},{"cmd":"Inconnu"}]"#))
        .await
        .unwrap();
    assert_silent(&mut client).await;
}

/// Un `cmd` qui n'est pas du texte : `InvalidPacket`, et la connexion reste ouverte.
#[tokio::test]
async fn cmd_pas_texte() {
    let url = start(seed_mit()).await;
    let (mut client, _) = connect(&url).await;
    client.send(Message::text(r#"[{"cmd":5}]"#)).await.unwrap();
    let reply = next_packets(&mut client).await.unwrap();
    let ServerPacket::InvalidPacket(invalid) = &reply[0] else {
        panic!("InvalidPacket attendu : {reply:?}");
    };
    assert_eq!(invalid.kind, "cmd");

    // Toujours ouverte : un deuxième envoi reçoit aussi sa réponse.
    client.send(Message::text(r#"[{"cmd":5}]"#)).await.unwrap();
    assert!(next_packets(&mut client).await.is_some());
}

/// JSON envoyé en binaire : accepté, comme par le serveur officiel.
#[tokio::test]
async fn trame_binaire_acceptee() {
    let url = start(seed_mit()).await;
    let (mut client, _) = connect(&url).await;
    client
        .send(Message::binary(br#"[{"cmd":5}]"#.to_vec()))
        .await
        .unwrap();
    let reply = next_packets(&mut client).await.unwrap();
    assert!(matches!(reply[0], ServerPacket::InvalidPacket(_)));
}

/// Paquet sans `cmd` : `InvalidPacket`, puis fermeture.
#[tokio::test]
async fn paquet_sans_cmd_ferme() {
    let url = start(seed_mit()).await;
    let (mut client, _) = connect(&url).await;
    client.send(Message::text(r#"[{"truc":1}]"#)).await.unwrap();
    let reply = next_packets(&mut client).await.unwrap();
    assert!(matches!(reply[0], ServerPacket::InvalidPacket(_)));
    assert!(next_packets(&mut client).await.is_none());
}

/// Trame qui n'est pas du JSON : fermeture, sans gêner les autres clients.
#[tokio::test]
async fn trame_illisible_ferme_seulement_ce_client() {
    let url = start(seed_mit()).await;
    let (mut bad, _) = connect(&url).await;
    let (mut good, _) = connect(&url).await;
    bad.send(Message::text("pas du json")).await.unwrap();
    assert!(next_packets(&mut bad).await.is_none());

    good.send(Message::text(r#"[{"cmd":5}]"#)).await.unwrap();
    assert!(next_packets(&mut good).await.is_some());
}
