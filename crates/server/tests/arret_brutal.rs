//! Arrêt brutal (NF07, ADR 0014) : le vrai binaire `raftup-server` est tué
//! sans prévenir (comme une coupure de courant pour lui) juste après avoir
//! répondu. Relancé, il doit avoir gardé tout ce qu'il avait acquitté.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Copie la seed dans un dossier neuf : la sauvegarde se crée à côté d'elle.
fn seed_copy() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("arret-brutal");
    std::fs::create_dir_all(&dir).unwrap();
    let _ = std::fs::remove_file(dir.join("seed-mit.raftup"));
    let _ = std::fs::remove_file(dir.join("seed-mit.raftup.tmp"));
    let seed = dir.join("seed-mit.archipelago");
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago");
    std::fs::copy(source, &seed).unwrap();
    seed
}

/// Un port libre : le système en donne un, on le relâche aussitôt.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Lance le binaire (cargo le compile avant les tests d'intégration).
fn launch(seed: &Path, port: u16) -> Child {
    Command::new(env!("CARGO_BIN_EXE_raftup-server"))
        .arg(seed)
        .args([
            "--no-tls",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// Se connecte dès que le serveur écoute (au plus 10 s), puis entre dans le
/// slot d'Alice. Renvoie le client et le paquet `Connected`.
async fn join(port: u16) -> (Client, Value) {
    let url = format!("ws://127.0.0.1:{port}");
    let mut attempt = 0;
    let (mut client, _) = loop {
        match tokio_tungstenite::connect_async(&url).await {
            Ok(ok) => break ok,
            // Pas encore à l'écoute : on réessaie un peu plus tard.
            Err(_) if attempt < 100 => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => panic!("serveur injoignable : {error}"),
        }
    };
    client.next().await.unwrap().unwrap(); // RoomInfo
    let connect = json!([{"cmd": "Connect", "password": null, "game": "ChecksFinder",
        "name": "Alice", "uuid": "u",
        "version": {"major": 0, "minor": 6, "build": 8, "class": "Version"},
        "items_handling": 7, "tags": []}]);
    client
        .send(Message::text(connect.to_string()))
        .await
        .unwrap();
    let reply = client.next().await.unwrap().unwrap();
    let packets: Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    assert_eq!(packets[0]["cmd"], "Connected");
    (client, packets[0].clone())
}

/// Lit jusqu'au `RoomUpdate` dont `checked_locations` contient `location`.
async fn wait_room_update(client: &mut Client, location: i64) {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .expect("pas de RoomUpdate pour ce lieu")
            .unwrap()
            .unwrap();
        let packets: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        let acknowledged = packets.as_array().unwrap().iter().any(|p| {
            p["cmd"] == "RoomUpdate"
                && p["checked_locations"]
                    .as_array()
                    .is_some_and(|l| l.contains(&json!(location)))
        });
        if acknowledged {
            return;
        }
    }
}

/// Lit jusqu'à un court silence.
async fn drain(client: &mut Client) {
    while let Ok(Some(Ok(_))) =
        tokio::time::timeout(Duration::from_millis(300), client.next()).await
    {}
}

#[tokio::test]
async fn rien_d_acquitte_n_est_perdu() {
    let seed = seed_copy();
    let port = free_port();
    let mut server = launch(&seed, port);
    let (mut alice, _) = join(port).await;
    drain(&mut alice).await;

    // Trois envois. Pour chacun, on attend le `RoomUpdate` qui annonce CE
    // lieu : la réponse qui l'acquitte (une trame plus ancienne ne compte pas).
    for location in [81003, 81006, 81009] {
        let checks = json!([{"cmd": "LocationChecks", "locations": [location]}]);
        alice.send(Message::text(checks.to_string())).await.unwrap();
        wait_room_update(&mut alice, location).await;
    }
    // Tué tout de suite : ni compactage, ni arrêt propre.
    server.kill().unwrap();
    server.wait().unwrap();

    // Relancé sur la même seed : il reprend la sauvegarde.
    let port = free_port();
    let mut server = launch(&seed, port);
    let (_alice, connected) = join(port).await;
    server.kill().unwrap();
    server.wait().unwrap();

    let checked: Vec<i64> = connected["checked_locations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    for location in [81003, 81006, 81009] {
        assert!(
            checked.contains(&location),
            "{location} perdu : {checked:?}"
        );
    }
}
