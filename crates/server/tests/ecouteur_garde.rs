//! Arrêt sans fermer le port, puis relance sur le même écouteur avec la connexion
//! qui a réveillé la partie (veille d'une partie hébergée par le hub, ADR 0052).

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt;
use multidata::{load_archipelago, ArchiveLimits, MultiData};
use server::{Room, Server};
use tokio_tungstenite::tungstenite::Message;

fn seed() -> MultiData {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago");
    load_archipelago(&std::fs::read(path).unwrap(), &ArchiveLimits::default()).unwrap()
}

/// Ouvre une connexion WebSocket et lit le premier paquet (`RoomInfo`).
async fn room_info(url: String) -> String {
    let (mut client, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(10), client.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    match frame {
        Message::Text(text) => text.to_string(),
        other => panic!("trame inattendue : {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn relance_sur_le_meme_port_avec_la_connexion_en_attente() {
    let server = Server::bind("127.0.0.1:0", Room::new(seed()))
        .await
        .unwrap();
    let address = server.local_addr().unwrap();
    let running = server.start().unwrap();
    assert!(room_info(format!("ws://{address}"))
        .await
        .contains("RoomInfo"));

    // Arrêt sans rendre le port : l'écouteur revient à l'appelant.
    let listener = running.stop_keeping_listener().await.unwrap();
    assert_eq!(listener.local_addr().unwrap(), address);

    // Un client arrive pendant la « veille » : l'appelant accepte la connexion,
    // relance le serveur sur le même écouteur et lui passe la connexion.
    let client = tokio::spawn(room_info(format!("ws://{address}")));
    let (stream, peer) = listener.accept().await.unwrap();
    let running = Server::from_listener(listener, Room::new(seed()))
        .with_pending_connection(stream, peer)
        .start()
        .unwrap();
    assert!(client.await.unwrap().contains("RoomInfo"));
    // Les connexions suivantes passent par l'accueil normal.
    assert!(room_info(format!("ws://{address}"))
        .await
        .contains("RoomInfo"));
    running.stop().await;
    // `stop` ferme le port.
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
}
