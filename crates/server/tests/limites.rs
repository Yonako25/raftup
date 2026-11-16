//! Limites des connexions (J1, étape 6 ; NF03, NF04, NF05, ADR 0007).
//!
//! Chaque test vérifie aussi qu'un autre client, lui, continue de marcher.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use multidata::{load_archipelago, ArchiveLimits};
use serde_json::Value;
use server::{Limits, Room, Server};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Lance un serveur avec ces limites ; renvoie son adresse `ws://`.
async fn start(limits: Limits) -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago");
    let data = load_archipelago(&std::fs::read(path).unwrap(), &ArchiveLimits::default()).unwrap();
    let server = Server::bind("127.0.0.1:0", Room::new(data))
        .await
        .unwrap()
        .with_limits(limits);
    let address = server.local_addr().unwrap();
    tokio::spawn(server.run());
    format!("ws://{address}")
}

/// Ouvre une connexion et lit le `RoomInfo` d'accueil.
async fn open(url: &str) -> Client {
    let (mut client, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let first = next_text(&mut client).await.unwrap();
    assert!(first.contains("RoomInfo"), "{first}");
    client
}

/// Ouvre une connexion et se connecte au slot d'Alice.
async fn open_alice(url: &str) -> Client {
    let mut client = open(url).await;
    let connect = r#"[{"cmd":"Connect","password":null,"game":"ChecksFinder","name":"Alice",
        "uuid":"u","version":{"major":0,"minor":6,"build":8,"class":"Version"},
        "items_handling":7,"tags":[]}]"#;
    client.send(Message::text(connect)).await.unwrap();
    drain(&mut client).await;
    client
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

/// Tout ce qui arrive jusqu'à un court silence.
async fn drain(client: &mut Client) -> Vec<String> {
    let mut frames = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_millis(250), client.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => frames.push(text.to_string()),
            Ok(Some(Ok(_))) => {}
            _ => return frames,
        }
    }
}

/// Lit jusqu'à la fermeture ; renvoie le code reçu, `None` sans trame de fermeture.
async fn wait_closed(client: &mut Client) -> Option<CloseCode> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let message = tokio::time::timeout_at(deadline, client.next())
            .await
            .expect("la connexion devait être fermée");
        match message {
            Some(Ok(Message::Close(frame))) => return frame.map(|f| f.code),
            None | Some(Err(_)) => return None,
            Some(Ok(_)) => continue,
        }
    }
}

/// Vérifie qu'un client marche encore : il reçoit les noms des jeux.
async fn assert_alive(client: &mut Client) {
    let request = r#"[{"cmd":"GetDataPackage","games":["Archipelago"]}]"#;
    client.send(Message::text(request)).await.unwrap();
    let reply = next_text(client).await.expect("connexion fermée à tort");
    assert!(reply.contains("DataPackage"), "{reply}");
}

/// Messages texte (`PrintJSON`) d'une liste de trames.
fn texts(frames: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for frame in frames {
        let packets: Vec<Value> = serde_json::from_str(frame).unwrap();
        for packet in packets {
            if packet["cmd"] == "PrintJSON" {
                out.push(packet["data"][0]["text"].as_str().unwrap_or("").to_string());
            }
        }
    }
    out
}

fn say(text: &str) -> Message {
    let packet = serde_json::json!([{ "cmd": "Say", "text": text }]);
    Message::text(packet.to_string())
}

#[test]
fn valeurs_de_la_spec() {
    let limits = Limits::default();
    assert_eq!(limits.max_message_size, 1024 * 1024);
    assert_eq!(limits.max_chat_length, 2000);
    assert_eq!(limits.packets_per_second, 50);
    assert_eq!(limits.chat_messages, 5);
    assert_eq!(limits.chat_window, Duration::from_secs(10));
    assert_eq!(limits.ping_interval, Duration::from_secs(20));
    assert_eq!(limits.silence_timeout, Duration::from_secs(40));
}

/// NF03 : un message de plus de 1 Mio ferme cette connexion (code 1009).
#[tokio::test]
async fn message_trop_gros() {
    let url = start(Limits::default()).await;
    let mut other = open(&url).await;
    let mut client = open(&url).await;

    // Un tableau JSON valide, juste au-dessus de 1 Mio.
    let big = format!("[\"{}\"]", "a".repeat(1024 * 1024));
    let _ = client.send(Message::text(big)).await;
    assert_eq!(wait_closed(&mut client).await, Some(CloseCode::Size));
    assert_alive(&mut other).await;
}

/// NF03 : un message de 1 Mio tout juste est accepté.
#[tokio::test]
async fn message_a_la_limite() {
    let url = start(Limits::default()).await;
    let mut client = open(&url).await;
    // `[` + 1 Mio - 2 espaces + `]` : exactement 1 Mio, un tableau vide.
    let frame = format!("[{}]", " ".repeat(1024 * 1024 - 2));
    client.send(Message::text(frame)).await.unwrap();
    assert_alive(&mut client).await;
}

/// NF03 : un chat de plus de 2 000 caractères n'est pas diffusé.
#[tokio::test]
async fn chat_trop_long() {
    let url = start(Limits::default()).await;
    let mut alice = open_alice(&url).await;
    let mut other = open_alice(&url).await;
    drain(&mut alice).await;

    // 2 001 caractères, dont des accents (2 octets chacun) : on compte les caractères.
    alice.send(say(&"é".repeat(2001))).await.unwrap();
    let mine = texts(&drain(&mut alice).await);
    assert_eq!(mine.len(), 1, "{mine:?}");
    assert!(mine[0].contains("2000 characters"), "{mine:?}");
    assert!(texts(&drain(&mut other).await).is_empty());

    // 2 000 caractères : diffusé à tous.
    alice.send(say(&"é".repeat(2000))).await.unwrap();
    assert_eq!(texts(&drain(&mut alice).await).len(), 1);
    assert_eq!(texts(&drain(&mut other).await).len(), 1);
}

/// NF04 : au plus 5 messages de chat par fenêtre ; la connexion reste ouverte.
#[tokio::test]
async fn chat_trop_frequent() {
    let limits = Limits {
        chat_window: Duration::from_millis(800),
        ..Limits::default()
    };
    let url = start(limits).await;
    let mut alice = open_alice(&url).await;
    let mut other = open_alice(&url).await;
    drain(&mut alice).await;

    for i in 0..6 {
        alice.send(say(&format!("message {i}"))).await.unwrap();
    }
    let received = texts(&drain(&mut other).await);
    assert_eq!(received.len(), 5, "{received:?}");
    let mine = texts(&drain(&mut alice).await);
    assert!(
        mine.last().unwrap().contains("at most 5 messages"),
        "{mine:?}"
    );

    // Une fois la fenêtre passée, on peut parler de nouveau.
    tokio::time::sleep(Duration::from_millis(800)).await;
    alice.send(say("de retour")).await.unwrap();
    assert_eq!(texts(&drain(&mut other).await).len(), 1);
}

/// NF04 : plus de 50 paquets d'un coup ferment la connexion (code 1008).
#[tokio::test]
async fn trop_de_paquets() {
    let url = start(Limits::default()).await;
    // `Sync` avant `Connect` est ignoré sans réponse : seul le débit compte ici.
    let frame = |count: usize| format!("[{}]", vec![r#"{"cmd":"Sync"}"#; count].join(","));

    let mut sage = open(&url).await;
    sage.send(Message::text(frame(50))).await.unwrap();

    let mut client = open(&url).await;
    client.send(Message::text(frame(51))).await.unwrap();
    assert_eq!(wait_closed(&mut client).await, Some(CloseCode::Policy));

    // 50 paquets d'un coup restent permis. On attend un peu que la réserve se
    // remplisse : un 51ᵉ paquet tout de suite serait de trop.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_alive(&mut sage).await;
}

/// NF05 : le serveur envoie des pings ; un client qui y répond reste connecté.
#[tokio::test]
async fn ping_et_client_vivant() {
    let limits = Limits {
        ping_interval: Duration::from_millis(100),
        silence_timeout: Duration::from_millis(300),
        ..Limits::default()
    };
    let url = start(limits).await;
    let mut client = open(&url).await;

    // On lit pendant 1 s : tungstenite répond tout seul aux pings.
    let mut pings = 0;
    let end = tokio::time::Instant::now() + Duration::from_secs(1);
    while let Ok(message) = tokio::time::timeout_at(end, client.next()).await {
        match message {
            Some(Ok(Message::Ping(_))) => pings += 1,
            Some(Ok(_)) => {}
            other => panic!("connexion fermée à tort : {other:?}"),
        }
    }
    assert!(pings >= 5, "{pings} pings seulement");
    assert_alive(&mut client).await;
}

/// NF05 : une connexion muette est fermée après le délai.
#[tokio::test]
async fn connexion_muette() {
    let limits = Limits {
        ping_interval: Duration::from_millis(100),
        silence_timeout: Duration::from_millis(300),
        ..Limits::default()
    };
    let url = start(limits).await;
    let mut other = open(&url).await;
    let mut silent = open(&url).await;

    // Pendant ce temps, `silent` ne lit rien, donc ne répond pas aux pings.
    // `other` lit, lui, et répond.
    let end = tokio::time::Instant::now() + Duration::from_millis(600);
    while tokio::time::timeout_at(end, other.next()).await.is_ok() {}

    // La connexion doit être fermée. Le code (1001, `Away`) n'arrive que si le
    // client lit avant la fin du délai de grâce du serveur (1 s) : sur une
    // machine lente, ses réponses aux pings arrivent après la coupure, et
    // Windows efface alors la trame de fermeture (RST). Un vrai client muet
    // ne lit de toute façon plus rien : seule la fermeture compte.
    let code = wait_closed(&mut silent).await;
    assert!(matches!(code, Some(CloseCode::Away) | None), "{code:?}");
    assert_alive(&mut other).await;
}

/// Un client qui ne lit pas ses réponses est fermé quand sa file est pleine.
#[tokio::test]
async fn client_trop_lent() {
    let limits = Limits {
        client_queue: 4,
        packets_per_second: 10_000,
        ..Limits::default()
    };
    let url = start(limits).await;
    let mut other = open(&url).await;
    let mut slow = open(&url).await;

    // 2 000 demandes d'un coup, sans lire les réponses.
    let request = r#"{"cmd":"GetDataPackage","games":["Archipelago"]}"#;
    let frame = format!("[{}]", vec![request; 2000].join(","));
    slow.send(Message::text(frame)).await.unwrap();

    // Le client lit enfin : quelques réponses, puis la fermeture.
    let mut replies = 0;
    loop {
        match tokio::time::timeout(Duration::from_secs(5), slow.next())
            .await
            .unwrap()
        {
            Some(Ok(Message::Text(_))) => replies += 1,
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            Some(Ok(_)) => {}
        }
    }
    assert!(
        replies < 2000,
        "{replies} réponses : le client n'a pas été fermé"
    );
    assert_alive(&mut other).await;
}
