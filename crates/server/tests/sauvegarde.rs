//! Sauvegarde `.raftup` branchée dans le serveur (ADR 0014, NF07) :
//! écrite sur le disque avant la réponse, et reprise au redémarrage.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use multidata::raftup::{seed_sha256, SaveFile, SaveHeader, Snapshot};
use multidata::{load_archipelago, ArchiveLimits, MultiData};
use serde_json::{json, Value};
use server::{Room, RunningServer, Server};
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

/// Chemin neuf, sous le dossier temporaire de cargo.
fn save_path(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("raftup-server-save");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.raftup"));
    let _ = std::fs::remove_file(&path);
    path
}

fn sha() -> String {
    seed_sha256(&seed_bytes())
}

async fn start_with_save(file: SaveFile, snapshot: Snapshot) -> (RunningServer, String) {
    let server = Server::bind("127.0.0.1:0", Room::new(seed()))
        .await
        .unwrap()
        .with_save(file, snapshot)
        .start()
        .unwrap();
    let url = format!("ws://{}", server.local_addr());
    (server, url)
}

/// Se connecte au slot `name` et renvoie le paquet `Connected`.
async fn join(url: &str, name: &str) -> (Client, Value) {
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
    let reply = client.next().await.unwrap().unwrap();
    let packets: Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    let connected = packets[0].clone();
    assert_eq!(connected["cmd"], "Connected");
    drain(&mut client).await;
    (client, connected)
}

/// Lit tout ce qui arrive jusqu'à un court silence.
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

#[tokio::test]
async fn ecrit_avant_de_repondre_puis_reprend() {
    let path = save_path("reprise");
    let header = SaveHeader::new("0.1.0", &seed().seed_name, &sha(), 1_760_000_000.0);
    let file = SaveFile::create(&path, header).unwrap();
    let (server, url) = start_with_save(file, Snapshot::default()).await;

    let (mut alice, _) = join(&url, "Alice").await;
    send(
        &mut alice,
        json!([{"cmd": "LocationChecks", "locations": [81003, 81006]}]),
    )
    .await;
    // Dès la première réponse, quelle qu'elle soit, les lieux sont déjà sur
    // le disque (NF07).
    alice.next().await.unwrap().unwrap();
    let (_, on_disk) = SaveFile::open(&path, &sha()).unwrap();
    let checks = on_disk
        .location_checks
        .iter()
        .find(|e| e.slot == 1)
        .unwrap();
    assert_eq!(checks.locations, vec![81003, 81006]);
    drain(&mut alice).await;

    // Un surnom et une valeur du stockage, à retrouver après la reprise.
    send(&mut alice, json!([{"cmd": "Say", "text": "!alias Zoé"}])).await;
    send(
        &mut alice,
        json!([{"cmd": "Set", "key": "compteur", "default": 0, "want_reply": true,
            "operations": [{"operation": "add", "value": 5}]}]),
    )
    .await;
    drain(&mut alice).await;
    drop(alice);
    // Pendant la partie : en-tête, instantané, puis des deltas.
    let lines = |p: &Path| std::fs::read_to_string(p).unwrap().lines().count();
    assert!(lines(&path) > 2);
    server.stop().await;
    // Arrêt propre : compacté en en-tête + un seul instantané.
    assert_eq!(lines(&path), 2);

    // Reprise : même fichier, nouvel état de départ lu dedans.
    let (file, snapshot) = SaveFile::open(&path, &sha()).unwrap();
    let (_server, url) = start_with_save(file, snapshot).await;
    let (mut alice, connected) = join(&url, "Alice").await;
    let checked: Vec<i64> = connected["checked_locations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert!(checked.contains(&81003) && checked.contains(&81006));
    let players = connected["players"].as_array().unwrap();
    assert_eq!(players[0]["alias"], "Zoé (Alice)");
    send(&mut alice, json!([{"cmd": "Get", "keys": ["compteur"]}])).await;
    let reply = drain(&mut alice).await;
    let retrieved = reply.iter().find(|p| p["cmd"] == "Retrieved").unwrap();
    assert_eq!(retrieved["keys"]["compteur"], 5);
}
