//! Suivi d'une partie depuis l'extérieur (ADR 0010) : photo de l'état,
//! fil d'événements et arrêt du serveur.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use multidata::{load_archipelago, ArchiveLimits};
use server::{GameEvent, Room, RunningServer, Server};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn start() -> (RunningServer, String) {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago");
    let data = load_archipelago(&std::fs::read(path).unwrap(), &ArchiveLimits::default()).unwrap();
    let server = Server::bind("127.0.0.1:0", Room::new(data))
        .await
        .unwrap()
        .start()
        .unwrap();
    let url = format!("ws://{}", server.local_addr());
    (server, url)
}

/// Ouvre une connexion et se connecte au slot `name` sur ChecksFinder.
async fn join(url: &str, name: &str, tags: &str) -> Client {
    let (mut client, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    client.next().await.unwrap().unwrap(); // RoomInfo
    let connect = format!(
        r#"[{{"cmd":"Connect","password":null,"game":"ChecksFinder","name":"{name}",
        "uuid":"u","version":{{"major":0,"minor":6,"build":8,"class":"Version"}},
        "items_handling":7,"tags":{tags}}}]"#
    );
    client.send(Message::text(connect)).await.unwrap();
    client.next().await.unwrap().unwrap(); // Connected
    client
}

/// Prochain événement du fil, en sautant ceux qu'on ne cherche pas.
async fn next_matching(
    events: &mut broadcast::Receiver<GameEvent>,
    wanted: impl Fn(&GameEvent) -> bool,
) -> GameEvent {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("pas d'événement")
            .unwrap();
        if wanted(&event) {
            return event;
        }
    }
}

#[tokio::test]
async fn photo_de_la_partie() {
    let (server, url) = start().await;

    let status = server.status().await.unwrap();
    let names: Vec<&str> = status.slots.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Alice", "Bob", "Chloé", "Lien"]);
    assert!(status.slots[3].is_group);
    assert_eq!(status.slots[0].total, 25);
    assert_eq!(status.slots[0].connections, 0);
    assert_eq!(status.slots[0].last_activity, None);

    let mut alice = join(&url, "Alice", r#"["DeathLink"]"#).await;
    alice
        .send(Message::text(
            r#"[{"cmd":"LocationChecks","locations":[81000,81001]}]"#,
        ))
        .await
        .unwrap();
    // On attend que la salle ait traité les lieux visités.
    let mut alice_slot = None;
    for _ in 0..50 {
        let status = server.status().await.unwrap();
        if status.slots[0].checked == 2 {
            alice_slot = Some(status.slots[0].clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let alice_slot = alice_slot.expect("lieux visités non vus");
    assert_eq!(alice_slot.connections, 1);
    assert_eq!(alice_slot.tags, ["DeathLink"]);
    assert_eq!(alice_slot.status, 5, "connecté");
    assert!(alice_slot.last_activity.is_some());
}

#[tokio::test]
async fn evenements_de_la_partie() {
    let (server, url) = start().await;
    let mut events = server.subscribe();

    let mut alice = join(&url, "Alice", "[]").await;
    let joined = next_matching(&mut events, |e| matches!(e, GameEvent::Joined { .. })).await;
    let GameEvent::Joined { name, game, .. } = joined else {
        unreachable!()
    };
    assert_eq!((name.as_str(), game.as_str()), ("Alice", "ChecksFinder"));

    alice
        .send(Message::text(
            r#"[{"cmd":"LocationChecks","locations":[81000]}]"#,
        ))
        .await
        .unwrap();
    let sent = next_matching(&mut events, |e| matches!(e, GameEvent::ItemSent { .. })).await;
    let GameEvent::ItemSent {
        finder, location, ..
    } = sent
    else {
        unreachable!()
    };
    assert_eq!(finder, "Alice");
    // Un vrai nom de lieu, pas un numéro.
    assert!(!location.starts_with("lieu "), "{location}");

    alice
        .send(Message::text(r#"[{"cmd":"Say","text":"coucou"}]"#))
        .await
        .unwrap();
    let chat = next_matching(&mut events, |e| matches!(e, GameEvent::Chat { .. })).await;
    assert!(matches!(chat, GameEvent::Chat { ref text, .. } if text == "coucou"));

    alice
        .send(Message::text(r#"[{"cmd":"StatusUpdate","status":30}]"#))
        .await
        .unwrap();
    let goal = next_matching(&mut events, |e| matches!(e, GameEvent::Goal { .. })).await;
    assert!(matches!(goal, GameEvent::Goal { ref name, .. } if name == "Alice"));

    server.announce("bienvenue".to_string()).await.unwrap();
    let announced = next_matching(&mut events, |e| matches!(e, GameEvent::ServerChat { .. })).await;
    assert!(matches!(announced, GameEvent::ServerChat { ref text, .. } if text == "bienvenue"));
    // Le joueur reçoit le message, comme ceux de la console du serveur officiel.
    let frame = loop {
        let Some(Ok(Message::Text(text))) = alice.next().await else {
            panic!("connexion fermée");
        };
        if text.contains("ServerChat") {
            break text.to_string();
        }
    };
    assert!(frame.contains("[Server]: bienvenue"), "{frame}");

    alice.close(None).await.unwrap();
    let left = next_matching(&mut events, |e| matches!(e, GameEvent::Left { .. })).await;
    assert!(matches!(left, GameEvent::Left { ref name, .. } if name == "Alice"));
}

#[tokio::test]
async fn arret_du_serveur() {
    let (server, url) = start().await;
    let address = server.local_addr();
    let mut alice = join(&url, "Alice", "[]").await;

    server.stop().await;

    // Le client connecté est déconnecté…
    let end = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match alice.next().await {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(end.is_ok(), "le client aurait dû être déconnecté");
    // … et le port est libre : on peut relancer un serveur dessus.
    let listener = tokio::net::TcpListener::bind(address).await;
    assert!(listener.is_ok(), "port encore pris : {listener:?}");
}
