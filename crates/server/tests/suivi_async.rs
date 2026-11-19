//! Suivi asynchrone (ADR 0031) : `!away`, `!back`, `!summary`, `!vote_release`,
//! résumé de retour, clés `_read_raftup_…`, sauvegarde. Seed : Alice, Bob,
//! Chloé (ChecksFinder) et le groupe Lien.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use multidata::raftup::{seed_sha256, SaveFile, SaveHeader, Snapshot};
use multidata::{load_archipelago, ArchiveLimits, MultiData};
use serde_json::{json, Value};
use server::{GameEvent, Room, RunningServer, Server};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn seed_bytes() -> Vec<u8> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago");
    std::fs::read(path).unwrap()
}

fn seed() -> MultiData {
    load_archipelago(&seed_bytes(), &ArchiveLimits::default()).unwrap()
}

async fn start_room(room: Room, save: Option<(SaveFile, Snapshot)>) -> (RunningServer, String) {
    let mut server = Server::bind("127.0.0.1:0", room).await.unwrap();
    if let Some((file, snapshot)) = save {
        server = server.with_save(file, snapshot);
    }
    let server = server.start().unwrap();
    let url = format!("ws://{}", server.local_addr());
    (server, url)
}

/// Serveur avec le suivi asynchrone.
async fn start() -> (RunningServer, String) {
    start_room(Room::new(seed()).with_async_follow(true), None).await
}

/// Se connecte au slot `name` ; renvoie le client et ce qui suit `Connected`.
async fn join(url: &str, name: &str) -> (Client, Vec<Value>) {
    let (mut client, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    client.next().await.unwrap().unwrap(); // RoomInfo
    let connect = json!([{"cmd": "Connect", "password": null, "game": "ChecksFinder",
        "name": name, "uuid": "u",
        "version": {"major": 0, "minor": 6, "build": 8, "class": "Version"},
        "items_handling": 7, "tags": []}]);
    client
        .send(Message::text(connect.to_string()))
        .await
        .unwrap();
    // La première trame est attendue sans limite (la salle peut être lente
    // à écrire sa sauvegarde) ; le reste jusqu'à un court silence.
    let first = client.next().await.unwrap().unwrap();
    let first: Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
    let mut after: Vec<Value> = first.as_array().unwrap().clone();
    assert_eq!(after[0]["cmd"], "Connected");
    after.extend(drain(&mut client).await);
    (client, after)
}

async fn drain(client: &mut Client) -> Vec<Value> {
    let mut all = Vec::new();
    while let Ok(Some(Ok(message))) =
        tokio::time::timeout(Duration::from_millis(300), client.next()).await
    {
        let packets: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        all.extend(packets.as_array().unwrap().iter().cloned());
    }
    all
}

async fn send(client: &mut Client, packet: Value) {
    client
        .send(Message::text(packet.to_string()))
        .await
        .unwrap();
}

async fn say(client: &mut Client, text: &str) -> Vec<Value> {
    send(client, json!([{"cmd": "Say", "text": text}])).await;
    drain(client).await
}

/// Textes de tous les `PrintJSON`.
fn texts(packets: &[Value]) -> Vec<String> {
    packets
        .iter()
        .filter(|p| p["cmd"] == "PrintJSON")
        .map(|p| {
            p["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|part| part["text"].as_str().unwrap_or_default())
                .collect()
        })
        .collect()
}

fn has(packets: &[Value], text: &str) -> bool {
    texts(packets).iter().any(|t| t == text)
}

#[tokio::test]
async fn sans_option_rien_ne_change() {
    let (_server, url) = start_room(Room::new(seed()), None).await;
    let (mut alice, after) = join(&url, "Alice").await;
    // Première connexion, et pas d'option : aucun résumé.
    assert!(!texts(&after).iter().any(|t| t.starts_with("Welcome back")));
    let packets = say(&mut alice, "!away").await;
    assert!(texts(&packets).iter().any(
        |t| t.starts_with("Could not find command away.") && t.ends_with("hint, hint_location")
    ));
    send(
        &mut alice,
        json!([{"cmd": "Get", "keys": ["_read_raftup_away_0"]}]),
    )
    .await;
    let packets = drain(&mut alice).await;
    assert_eq!(packets[0]["keys"]["_read_raftup_away_0"], Value::Null);
}

#[tokio::test]
async fn aide_et_commandes_connues() {
    let (_server, url) = start().await;
    let (mut alice, _) = join(&url, "Alice").await;
    let packets = say(&mut alice, "!bidule").await;
    assert!(texts(&packets)
        .iter()
        .any(|t| t.ends_with("hint, hint_location, away, back, summary, vote_release")));
    let packets = say(&mut alice, "!help").await;
    assert!(texts(&packets)
        .iter()
        .any(|t| t.contains("!vote_release [player]")));
}

#[tokio::test]
async fn absence_et_message_automatique() {
    let (server, url) = start().await;
    let mut events = server.subscribe();
    let (mut alice, _) = join(&url, "Alice").await;
    let (mut bob, _) = join(&url, "Bob").await;
    drain(&mut alice).await;

    let packets = say(&mut alice, "!away  partie le week-end ").await;
    assert!(has(
        &packets,
        "You are now marked as away. Use !back when you are back."
    ));
    let seen_by_bob = drain(&mut bob).await;
    assert!(has(&seen_by_bob, "Alice is away: partie le week-end"));

    // Bob cite Alice : lui seul reçoit le message d'absence.
    let packets = say(&mut bob, "alice tu es là ?").await;
    assert!(has(
        &packets,
        "Alice is away (for less than a minute): partie le week-end"
    ));
    let seen_by_alice = drain(&mut alice).await;
    assert!(!texts(&seen_by_alice)
        .iter()
        .any(|t| t.starts_with("Alice is away (for")));

    // Repli par le stockage.
    send(
        &mut bob,
        json!([{"cmd": "Get", "keys": ["_read_raftup_away_0"]}]),
    )
    .await;
    let packets = drain(&mut bob).await;
    let away = &packets[0]["keys"]["_read_raftup_away_0"]["1"];
    assert_eq!(away["message"], "partie le week-end");

    // Photo pour le launcher.
    let status = server.status().await.unwrap();
    assert_eq!(status.slots[0].away.as_deref(), Some("partie le week-end"));

    let packets = say(&mut alice, "!back").await;
    assert!(has(&packets, "Alice is back."));
    let packets = say(&mut alice, "!back").await;
    assert!(has(&packets, "You are not marked as away."));

    // Le fil d'événements du launcher a vu l'absence puis le retour.
    let mut kinds = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            GameEvent::Away { message, .. } => kinds.push(format!("away:{message}")),
            GameEvent::Back { name, .. } => kinds.push(format!("back:{name}")),
            _ => {}
        }
    }
    assert_eq!(kinds, ["away:partie le week-end", "back:Alice"]);
}

#[tokio::test]
async fn un_lieu_visite_leve_l_absence() {
    let (_server, url) = start().await;
    let (mut alice, _) = join(&url, "Alice").await;
    say(&mut alice, "!away").await;
    send(
        &mut alice,
        json!([{"cmd": "LocationChecks", "locations": [81000]}]),
    )
    .await;
    let packets = drain(&mut alice).await;
    assert!(has(&packets, "Alice is back."));
}

#[tokio::test]
async fn release_par_vote() {
    let (server, url) = start().await;
    let (mut alice, _) = join(&url, "Alice").await;
    let (mut bob, _) = join(&url, "Bob").await;
    drain(&mut alice).await;

    let packets = say(&mut alice, "!vote_release Alice").await;
    assert!(has(
        &packets,
        "You can't vote for your own world. Use !release instead."
    ));
    let packets = say(&mut alice, "!vote_release").await;
    assert!(has(
        &packets,
        "No vote in progress. Usage: !vote_release <player>"
    ));

    // 2 autres joueurs pour Chloé : il faut 2 voix.
    let packets = say(&mut alice, "!vote_release chloé").await;
    assert!(has(&packets, "Alice voted to release Chloé's world (1/2)."));
    let packets = say(&mut alice, "!vote_release Chloé").await;
    assert!(has(
        &packets,
        "You already voted to release Chloé's world (1/2)."
    ));
    send(
        &mut bob,
        json!([{"cmd": "Get", "keys": ["_read_raftup_release_votes_0"]}]),
    )
    .await;
    let packets = drain(&mut bob).await;
    let votes = packets.iter().find(|p| p["cmd"] == "Retrieved").unwrap();
    assert_eq!(
        votes["keys"]["_read_raftup_release_votes_0"]["3"],
        json!([1])
    );
    assert_eq!(server.status().await.unwrap().slots[2].release_votes, 1);

    let packets = say(&mut bob, "!vote_release Chloé").await;
    assert!(has(&packets, "Bob voted to release Chloé's world (2/2)."));
    assert!(has(&packets, "The vote passed: releasing Chloé's world."));
    // Même relâchement que `/release`.
    assert!(packets.iter().any(|p| p["type"] == "Release"));
    let status = server.status().await.unwrap();
    assert_eq!(status.slots[2].checked, status.slots[2].total);
    assert_eq!(status.slots[2].release_votes, 0);

    // Plus rien à relâcher.
    let packets = say(&mut alice, "!vote_release Chloé").await;
    assert!(has(&packets, "Chloé's world has nothing left to release."));
}

#[tokio::test]
async fn le_vote_tombe_si_le_joueur_revient() {
    let (_server, url) = start().await;
    let (mut alice, _) = join(&url, "Alice").await;
    let (mut bob, _) = join(&url, "Bob").await;
    drain(&mut alice).await;
    say(&mut alice, "!vote_release Bob").await;
    drain(&mut bob).await;
    send(
        &mut bob,
        json!([{"cmd": "LocationChecks", "locations": [81000]}]),
    )
    .await;
    let packets = drain(&mut bob).await;
    assert!(has(
        &packets,
        "The vote to release Bob's world is cancelled: Bob is playing again."
    ));
}

#[tokio::test]
async fn resume_de_retour() {
    let (_server, url) = start().await;
    let (alice, after) = join(&url, "Alice").await;
    // Première connexion : pas de résumé.
    assert!(!texts(&after).iter().any(|t| t.starts_with("Welcome back")));
    drop(alice);
    // Laisse la salle voir le départ.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let (mut bob, _) = join(&url, "Bob").await;
    say(&mut bob, "!away brb").await;
    // Bob relâche son monde : des objets partent vers Alice.
    send(
        &mut bob,
        json!([{"cmd": "LocationChecks", "locations": (81000..81025).collect::<Vec<i64>>()}]),
    )
    .await;
    drain(&mut bob).await;
    say(&mut bob, "!away brb").await;

    let (mut alice, after) = join(&url, "Alice").await;
    let lines = texts(&after);
    let welcome = lines
        .iter()
        .position(|t| t.starts_with("Welcome back, Alice! You were away for "))
        .expect("résumé absent");
    // Le résumé vient après le tutoriel.
    let tutorial = lines
        .iter()
        .position(|t| t.starts_with("Now that you are connected"))
        .unwrap();
    assert!(welcome > tutorial);
    assert!(lines.iter().any(|t| t.starts_with("You received ")));
    assert!(lines.iter().any(|t| t == "Bob went away: brb"));
    assert!(lines.iter().any(|t| t == "Bob came back."));
    assert!(lines
        .iter()
        .any(|t| t == "Currently away: Bob (for less than a minute: brb)."));

    // `!summary` redonne l'état à la demande.
    let packets = say(&mut alice, "!summary").await;
    let lines = texts(&packets);
    assert!(lines.iter().any(|t| t.ends_with("Bob went away: brb")));
}

#[tokio::test]
async fn sauvegarde_et_reprise() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("raftup-suivi-async");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("partie.raftup");
    let _ = std::fs::remove_file(&path);
    let header = SaveHeader::new("0.1.0", "AP_TEST", &seed_sha256(&seed_bytes()), 0.0);
    let file = SaveFile::create(&path, header).unwrap();

    let room = || Room::new(seed()).with_async_follow(true);
    let (server, url) = start_room(room(), Some((file, Snapshot::default()))).await;
    let (mut alice, _) = join(&url, "Alice").await;
    let (mut bob, _) = join(&url, "Bob").await;
    say(&mut alice, "!away vacances").await;
    say(&mut bob, "!vote_release Chloé").await;
    // Arrêt avec Alice et Bob connectés : ils sont notés « vus » à l'arrêt.
    server.stop().await;

    let (file, snapshot) = SaveFile::open(&path, &seed_sha256(&seed_bytes())).unwrap();
    assert_eq!(snapshot.away.len(), 1);
    assert_eq!(snapshot.release_votes[0].voters, vec![2]);
    assert_eq!(snapshot.seen.len(), 2);
    assert!(!snapshot.feed.is_empty());
    let (server, url) = start_room(room(), Some((file, snapshot))).await;
    let status = server.status().await.unwrap();
    assert_eq!(status.slots[0].away.as_deref(), Some("vacances"));
    assert_eq!(status.slots[2].release_votes, 1);
    let (_alice, after) = join(&url, "Alice").await;
    let lines = texts(&after);
    assert!(lines.iter().any(|t| t.starts_with("Welcome back, Alice!")));
    assert!(lines
        .iter()
        .any(|t| t == "You are still marked as away. Use !back to clear it."));
    assert!(lines
        .iter()
        .any(|t| t == "Vote to release Chloé's world: 1/2."));
}
