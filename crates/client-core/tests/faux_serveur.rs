//! Tests du client contre un faux serveur local, qui rejoue les paquets capturés
//! auprès du serveur officiel (voir `crates/ap-protocol/tests/captures/`).

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used)]

use std::future::Future;
use std::path::Path;

use ap_protocol::client::LocationChecks;
use ap_protocol::{decode_client_frame, ClientPacket, ServerPacket};
use client_core::{Client, ClientError, ConnectOptions};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

type ServerSocket = WebSocketStream<TcpStream>;

/// Lit une capture du serveur officiel.
fn capture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ap-protocol/tests/captures/serveur-0.6.8")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

/// Lance un faux serveur sur un port libre et lui fait jouer `script`.
///
/// Les connexions dont la poignée de main WebSocket échoue (par exemple un
/// essai en `wss://`) sont ignorées : on attend la suivante.
/// `F: FnOnce(...) -> Fut` : `script` est une fonction `async` qui reçoit le
/// WebSocket ; `Send + 'static` permet de la lancer dans une tâche tokio.
async fn fake_server<F, Fut>(script: F) -> (String, JoinHandle<()>)
where
    F: FnOnce(ServerSocket) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let task = tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            if let Ok(socket) = tokio_tungstenite::accept_async(tcp).await {
                script(socket).await;
                return;
            }
        }
    });
    (address, task)
}

/// Lit la prochaine trame texte envoyée par le client.
async fn read_frame(socket: &mut ServerSocket) -> Vec<ClientPacket> {
    loop {
        match socket.next().await.unwrap().unwrap() {
            Message::Text(text) => return decode_client_frame(text.as_str()).unwrap(),
            Message::Close(_) => panic!("le client a fermé la connexion"),
            _ => {}
        }
    }
}

/// Début de session normal : `RoomInfo`, lecture du `Connect`, puis
/// `Connected` + `ReceivedItems` (trame réelle du serveur officiel).
async fn accept_session(socket: &mut ServerSocket) -> ap_protocol::client::Connect {
    socket
        .send(Message::text(capture("RoomInfo.json")))
        .await
        .unwrap();
    let mut frame = read_frame(socket).await;
    let ClientPacket::Connect(connect) = frame.remove(0) else {
        panic!("Connect attendu");
    };
    socket
        .send(Message::text(capture("trame_multiple.json")))
        .await
        .unwrap();
    connect
}

fn options(address: &str) -> ConnectOptions {
    ConnectOptions::new(&format!("ws://{address}"), "Rancher1", "Slime Rancher")
}

#[tokio::test]
async fn connexion_puis_reception() {
    let (address, server) = fake_server(|mut socket| async move {
        let connect = accept_session(&mut socket).await;
        assert_eq!(connect.name, "Rancher1");
        assert_eq!(connect.game, "Slime Rancher");
        assert_eq!(connect.version, client_core::CLIENT_VERSION);
        socket
            .send(Message::text(capture("PrintJSON_Join.json")))
            .await
            .unwrap();
        socket.close(None).await.unwrap();
    })
    .await;

    let (mut client, session) = Client::connect(&options(&address)).await.unwrap();
    assert_eq!(session.room.seed_name, "13515657874892102023");
    assert_eq!(session.connected.slot, 1);
    // `ReceivedItems` était dans la même trame que `Connected` : il ne doit pas être perdu.
    assert_eq!(client.recv().await.unwrap().unwrap().cmd(), "ReceivedItems");
    assert_eq!(client.recv().await.unwrap().unwrap().cmd(), "PrintJSON");
    // Le serveur a fermé : plus de paquet.
    assert!(client.recv().await.unwrap().is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn connexion_refusee() {
    let (address, server) = fake_server(|mut socket| async move {
        socket
            .send(Message::text(capture("RoomInfo.json")))
            .await
            .unwrap();
        read_frame(&mut socket).await;
        socket
            .send(Message::text(capture("ConnectionRefused.json")))
            .await
            .unwrap();
    })
    .await;

    match Client::connect(&options(&address)).await {
        Err(ClientError::Refused(errors)) => assert_eq!(errors, ["InvalidSlot"]),
        Err(other) => panic!("erreur inattendue : {other}"),
        Ok(_) => panic!("la connexion aurait dû être refusée"),
    }
    server.await.unwrap();
}

/// Sans schéma, le client essaie `wss://` (refusé par ce serveur en clair),
/// puis `ws://` (ADR 0005).
#[tokio::test]
async fn adresse_sans_schema_bascule_en_ws() {
    let (address, server) = fake_server(|mut socket| async move {
        accept_session(&mut socket).await;
    })
    .await;

    let options = ConnectOptions::new(&address, "Rancher1", "Slime Rancher");
    let (_client, session) = Client::connect(&options).await.unwrap();
    assert_eq!(session.url, format!("ws://{address}"));
    server.await.unwrap();
}

#[tokio::test]
async fn envoi_de_lieux_visites() {
    let (address, server) = fake_server(|mut socket| async move {
        accept_session(&mut socket).await;
        let frame = read_frame(&mut socket).await;
        assert_eq!(
            frame,
            [ClientPacket::LocationChecks(LocationChecks {
                locations: vec![1, 2, 3]
            })]
        );
    })
    .await;

    let (mut client, _session) = Client::connect(&options(&address)).await.unwrap();
    client
        .send(&[ClientPacket::LocationChecks(LocationChecks {
            locations: vec![1, 2, 3],
        })])
        .await
        .unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn premier_paquet_autre_que_room_info() {
    let (address, server) = fake_server(|mut socket| async move {
        socket
            .send(Message::text(capture("PrintJSON_Join.json")))
            .await
            .unwrap();
    })
    .await;

    match Client::connect(&options(&address)).await {
        Err(ClientError::UnexpectedPacket(cmd)) => assert_eq!(cmd, "PrintJSON"),
        Err(other) => panic!("erreur inattendue : {other}"),
        Ok(_) => panic!("la connexion aurait dû échouer"),
    }
    server.await.unwrap();
}

#[tokio::test]
async fn message_trop_gros_refuse() {
    let (address, server) = fake_server(|mut socket| async move {
        accept_session(&mut socket).await;
        let huge = format!(r#"[{{"cmd":"Say","text":"{}"}}]"#, "x".repeat(10_000));
        // Le client coupe la connexion : l'envoi peut échouer, peu importe.
        let _ = socket.send(Message::text(huge)).await;
    })
    .await;

    let mut options = options(&address);
    // Assez pour la trame `Connected` (environ 3 Kio), pas pour le message géant.
    options.max_message_size = 8 * 1024;
    let (mut client, _session) = Client::connect(&options).await.unwrap();
    assert_eq!(client.recv().await.unwrap().unwrap().cmd(), "ReceivedItems");
    assert!(matches!(
        client.recv().await,
        Err(ClientError::WebSocket(_))
    ));
    server.await.unwrap();
}

/// Les paquets inconnus ne coupent pas la session.
#[tokio::test]
async fn paquet_inconnu_transmis() {
    let (address, server) = fake_server(|mut socket| async move {
        accept_session(&mut socket).await;
        socket
            .send(Message::text(r#"[{"cmd":"Nouveaute","x":1}]"#))
            .await
            .unwrap();
    })
    .await;

    let (mut client, _session) = Client::connect(&options(&address)).await.unwrap();
    client.recv().await.unwrap(); // ReceivedItems
    let packet = client.recv().await.unwrap().unwrap();
    assert!(matches!(packet, ServerPacket::Unknown(_)));
    server.await.unwrap();
}
