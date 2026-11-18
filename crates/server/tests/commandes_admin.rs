//! Commandes serveur de `!admin` (`ServerCommandProcessor`) : réponses
//! `AdminCommandResult` et effets sur la partie.

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

/// La seed de test, avec un mot de passe d'administration.
fn seed() -> MultiData {
    let mut data = load_archipelago(&seed_bytes(), &ArchiveLimits::default()).unwrap();
    data.server_options.server_password = Some("admin".to_string());
    data
}

async fn start() -> (RunningServer, String) {
    let server = Server::bind("127.0.0.1:0", Room::new(seed()))
        .await
        .unwrap()
        .start()
        .unwrap();
    let url = format!("ws://{}", server.local_addr());
    (server, url)
}

/// Ouvre une connexion et se connecte au slot `name` (ChecksFinder).
async fn join(url: &str, name: &str) -> Client {
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
    client.next().await.unwrap().unwrap(); // Connected
    drain(&mut client).await;
    client
}

/// Se connecte puis devient administrateur.
async fn admin(url: &str, name: &str) -> Client {
    let mut client = join(url, name).await;
    let packets = run(&mut client, "!admin login admin").await;
    assert_eq!(
        results(&packets),
        ["Login successful. You can now issue server side commands."]
    );
    client
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

/// Envoie `text` et renvoie les paquets reçus ensuite.
async fn run(client: &mut Client, text: &str) -> Vec<Value> {
    let packet = json!([{"cmd": "Say", "text": text}]);
    client
        .send(Message::text(packet.to_string()))
        .await
        .unwrap();
    drain(client).await
}

/// Textes des paquets `PrintJSON` d'un genre.
fn texts_of(packets: &[Value], kind: &str) -> Vec<String> {
    packets
        .iter()
        .filter(|p| p["type"] == kind)
        .map(|p| p["data"][0]["text"].as_str().unwrap().to_string())
        .collect()
}

fn results(packets: &[Value]) -> Vec<String> {
    texts_of(packets, "CommandResult")
}

fn admin_results(packets: &[Value]) -> Vec<String> {
    texts_of(packets, "AdminCommandResult")
}

/// Textes de `PrintJSON` sans genre (annonces du serveur).
fn plain_texts(packets: &[Value]) -> Vec<String> {
    packets
        .iter()
        .filter(|p| p["cmd"] == "PrintJSON" && p.get("type").is_none_or(Value::is_null))
        .map(|p| p["data"][0]["text"].as_str().unwrap().to_string())
        .collect()
}

/// Numéros d'objets des `ReceivedItems`.
fn received_ids(packets: &[Value]) -> Vec<i64> {
    packets
        .iter()
        .filter(|p| p["cmd"] == "ReceivedItems")
        .flat_map(|p| p["items"].as_array().unwrap().clone())
        .map(|i| i["item"].as_i64().unwrap())
        .collect()
}

/// Lieux des `RoomUpdate` (`checked_locations`).
fn checked_in_updates(packets: &[Value]) -> Vec<i64> {
    packets
        .iter()
        .filter(|p| p["cmd"] == "RoomUpdate")
        .filter_map(|p| p["checked_locations"].as_array())
        .flatten()
        .map(|l| l.as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn players_et_status_repondent_a_l_admin_seul() {
    let (_server, url) = start().await;
    let mut alice = admin(&url, "Alice").await;
    let mut bob = join(&url, "Bob").await;
    drain(&mut alice).await;

    let packets = run(&mut alice, "!admin /players").await;
    assert_eq!(
        admin_results(&packets),
        ["2 players of 3 connected :: Team #1: Alice Bob (Chloé)"]
    );
    // Contrairement à `!players`, rien n'est diffusé.
    assert!(results(&drain(&mut bob).await).is_empty());
    assert!(results(&packets).is_empty());

    let packets = run(&mut alice, "!admin /status").await;
    assert_eq!(
        admin_results(&packets),
        ["Player Status on team 0:\n\
Alice has 1 connection. (0/25)\n\
Bob has 1 connection. (0/25)\n\
Chloé has 0 connections. (0/88)"]
    );
    let packets = run(&mut alice, "!admin /players en trop").await;
    assert_eq!(admin_results(&packets), ["Invalid arguments for /players."]);
}

#[tokio::test]
async fn send_donne_un_objet() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice").await;
    let mut bob = admin(&url, "Bob").await;
    drain(&mut alice).await;

    // Nom de joueur et d'objet approchés, comme l'officiel.
    let packets = run(&mut bob, "!admin /send alice map height").await;
    assert!(admin_results(&packets).is_empty());
    assert_eq!(
        plain_texts(&packets),
        ["Cheat console: sending \"Map Height\" to Alice"]
    );
    // Alice reçoit l'objet (80001 = Map Height), venu du serveur.
    let seen = drain(&mut alice).await;
    assert_eq!(received_ids(&seen), [80001]);
    let item = seen
        .iter()
        .find(|p| p["cmd"] == "ReceivedItems")
        .unwrap()
        .clone();
    assert_eq!(item["items"][0]["location"], -1);
    assert_eq!(item["items"][0]["player"], 0);
    assert_eq!(
        plain_texts(&seen),
        ["Cheat console: sending \"Map Height\" to Alice"]
    );

    let packets = run(&mut bob, "!admin /send_multiple 3 Alice Map Height").await;
    assert_eq!(
        plain_texts(&packets),
        ["Cheat console: sending 3 of \"Map Height\" to Alice"]
    );
    assert_eq!(received_ids(&drain(&mut alice).await), [80001; 3]);

    let packets = run(&mut bob, "!admin /send Zorglub Map Height").await;
    assert!(admin_results(&packets)[0].starts_with("Didn't find something that closely matches"));
    let packets = run(&mut bob, "!admin /send_multiple 101 Alice Map Height").await;
    assert_eq!(admin_results(&packets), ["101 is invalid. Maximum is 100."]);
}

#[tokio::test]
async fn send_erreurs() {
    let (_server, url) = start().await;
    let mut bob = admin(&url, "Bob").await;
    let packets = run(&mut bob, "!admin /send").await;
    assert_eq!(admin_results(&packets), ["Invalid arguments for /send."]);
    let packets = run(&mut bob, "!admin /send Alice zzzzzzzzzzzz").await;
    assert!(admin_results(&packets)[0].starts_with("Didn't find something that closely matches"));
    let packets = run(&mut bob, "!admin /send_multiple beaucoup Alice Map Height").await;
    assert_eq!(
        admin_results(&packets),
        ["Invalid arguments for /send_multiple."]
    );
}

#[tokio::test]
async fn send_location_coche_le_lieu() {
    let (_server, url) = start().await;
    let mut alice = admin(&url, "Alice").await;
    drain(&mut alice).await;

    // Par nom (approché) : « Tile 1 » est un lieu d'Alice.
    let packets = run(&mut alice, "!admin /send_location Alice tile 1").await;
    assert_eq!(checked_in_updates(&packets).len(), 1);
    assert!(admin_results(&packets).is_empty());
    // Par numéro.
    let packets = run(&mut alice, "!admin /send_location alice 81002").await;
    assert_eq!(checked_in_updates(&packets), [81002]);
    // Le statut le confirme : deux lieux visités sur 25.
    let packets = run(&mut alice, "!admin /status").await;
    assert!(admin_results(&packets)[0].contains("Alice has 1 connection. (2/25)"));
    let packets = run(&mut alice, "!admin /send_location Alice zzzzzzzzzzzz").await;
    assert!(admin_results(&packets)[0].starts_with("Didn't find something that closely matches"));
}

#[tokio::test]
async fn release_envoie_les_objets_du_joueur() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice").await;
    let mut bob = admin(&url, "Bob").await;
    drain(&mut alice).await;

    let packets = run(&mut bob, "!admin /release nobody").await;
    assert_eq!(
        admin_results(&packets),
        ["Could not find player nobody to release"]
    );
    // Casse ignorée si un seul nom correspond.
    let packets = run(&mut bob, "!admin /release alice").await;
    assert_eq!(
        texts_of(&packets, "Release"),
        ["Alice (Team #1) has released all remaining items from their world."]
    );
    // Les 25 lieux d'Alice sont visités.
    let seen = drain(&mut alice).await;
    let unique: std::collections::BTreeSet<i64> = checked_in_updates(&seen).into_iter().collect();
    assert_eq!(unique.len(), 25);
    let packets = run(&mut bob, "!admin /release").await;
    assert_eq!(admin_results(&packets), ["Invalid arguments for /release."]);
}

#[tokio::test]
async fn collect_recupere_les_objets_du_joueur() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice").await;
    let mut bob = admin(&url, "Bob").await;
    drain(&mut alice).await;

    let packets = run(&mut bob, "!admin /collect zorglub").await;
    assert_eq!(
        admin_results(&packets),
        ["Could not find player zorglub to collect"]
    );
    let packets = run(&mut bob, "!admin /collect Alice").await;
    assert_eq!(
        texts_of(&packets, "Collect"),
        ["Alice (Team #1) has collected their items from other worlds."]
    );
    // Effet : le lieu 81018 (monde de Bob) cache un objet d'Alice, il est visité.    assert!(checked_in_updates(&packets).contains(&81018));
}

#[tokio::test]
async fn allow_et_forbid_release() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice").await;
    let mut bob = admin(&url, "Bob").await;
    drain(&mut alice).await;
    let refused = "Sorry, client item releasing requires you to have beaten the game on this \
                   server. You can ask the server admin for a /release";

    // Mode « auto » : sans but atteint, Alice est refusée.
    assert_eq!(results(&run(&mut alice, "!release").await), [refused]);
    let packets = run(&mut bob, "!admin /allow_release alice").await;
    assert_eq!(
        admin_results(&packets),
        ["Player Alice is now allowed to use the !release command at any time."]
    );
    // Alice le peut maintenant.
    let packets = run(&mut alice, "!release").await;
    assert_eq!(texts_of(&packets, "Release").len(), 1);

    let packets = run(&mut bob, "!admin /forbid_release ALICE").await;
    assert_eq!(
        admin_results(&packets),
        ["Player Alice has to follow the server restrictions on use of the !release command."]
    );
    assert_eq!(results(&run(&mut alice, "!release").await), [refused]);

    let packets = run(&mut bob, "!admin /allow_release nobody").await;
    assert_eq!(
        admin_results(&packets),
        ["Could not find player nobody to allow the !release command for."]
    );
    let packets = run(&mut bob, "!admin /forbid_release nobody").await;
    assert_eq!(
        admin_results(&packets),
        ["Could not find player nobody to forbid the !release command for."]
    );
}

#[tokio::test]
async fn hint_et_hint_location_sans_cout() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice").await;
    let mut bob = admin(&url, "Bob").await;
    drain(&mut alice).await;

    // Alice n'a aucun point, mais l'admin donne l'indice sans payer.
    run(&mut bob, "!admin /hint alice map hieght").await;
    // Alice reçoit tous les indices de son objet (seuls les slots concernés les voient).
    let seen = drain(&mut alice).await;
    let hints: Vec<&Value> = seen.iter().filter(|p| p["type"] == "Hint").collect();
    assert!(hints.len() > 1);
    assert!(hints.iter().all(|h| h["receiving"] == 1));
    assert!(hints.iter().all(|h| h["item"]["item"] == 80001));
    assert!(seen
        .iter()
        .any(|p| p["cmd"] == "RoomUpdate" && p["hint_points"] == 0));

    run(&mut bob, "!admin /hint_location Alice Tile 1").await;
    let seen = drain(&mut alice).await;
    let hints: Vec<&Value> = seen.iter().filter(|p| p["type"] == "Hint").collect();
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0]["item"]["player"], 1);

    let packets = run(&mut bob, "!admin /hint Alice 99999999999999999999").await;
    assert_eq!(admin_results(&packets), ["No hints found."]);
}

#[tokio::test]
async fn hint_erreurs() {
    let (_server, url) = start().await;
    let mut bob = admin(&url, "Bob").await;
    let packets = run(&mut bob, "!admin /hint Alice zzzzzzzzzzzz").await;
    assert!(admin_results(&packets)[0].starts_with("Didn't find something that closely matches"));
    let packets = run(&mut bob, "!admin /hint").await;
    assert_eq!(admin_results(&packets), ["Invalid arguments for /hint."]);
    let packets = run(&mut bob, "!admin /hint_location Alice 99999999999999999999").await;
    assert_eq!(admin_results(&packets), ["No hints found."]);
}

#[tokio::test]
async fn save_sans_sauvegarde_et_avec() {
    let (_server, url) = start().await;
    let mut alice = admin(&url, "Alice").await;
    let packets = run(&mut alice, "!admin /save").await;
    assert_eq!(admin_results(&packets), ["Saving is disabled."]);

    // Avec une sauvegarde.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("raftup-server-admin");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("save.raftup");
    let _ = std::fs::remove_file(&path);
    let sha = seed_sha256(&seed_bytes());
    let header = SaveHeader::new("0.1.0", &seed().seed_name, &sha, 1_760_000_000.0);
    let file = SaveFile::create(&path, header).unwrap();
    let server = Server::bind("127.0.0.1:0", Room::new(seed()))
        .await
        .unwrap()
        .with_save(file, Snapshot::default())
        .start()
        .unwrap();
    let url = format!("ws://{}", server.local_addr());
    let mut alice = admin(&url, "Alice").await;
    let packets = run(&mut alice, "!admin /save").await;
    assert_eq!(admin_results(&packets), ["Game saved"]);
    server.stop().await;
}

#[tokio::test]
async fn commande_inconnue_refusee_et_texte_simple() {
    let (_server, url) = start().await;
    let mut alice = admin(&url, "Alice").await;
    let mut bob = join(&url, "Bob").await;
    drain(&mut alice).await;

    let packets = run(&mut alice, "!admin /bidule").await;
    assert_eq!(
        admin_results(&packets),
        [
            "Could not find command bidule. Known commands: help, license, options, save, \
players, status, exit, alias, collect, countdown, release, allow_release, forbid_release, \
send_multiple, send, send_location, hint, hint_location, option, datastore"
        ]
    );
    // Connue de l'officiel, pas encore de Raftup : refus clair.
    let packets = run(&mut alice, "!admin /exit").await;
    assert_eq!(
        admin_results(&packets),
        ["Server command /exit is not available in Raftup yet."]
    );
    // Sans `/` : message du serveur, vu de tous.
    let packets = run(&mut alice, "!admin bonjour tout le monde").await;
    assert_eq!(
        texts_of(&packets, "ServerChat"),
        ["[Server]: bonjour tout le monde"]
    );
    let seen = drain(&mut bob).await;
    assert_eq!(
        texts_of(&seen, "ServerChat"),
        ["[Server]: bonjour tout le monde"]
    );
}

#[tokio::test]
async fn countdown_de_l_admin() {
    let (_server, url) = start().await;
    let mut alice = admin(&url, "Alice").await;
    let packets = run(&mut alice, "!admin /countdown 0").await;
    let texts = texts_of(&packets, "Countdown");
    assert_eq!(
        texts,
        ["[Server]: Starting countdown of 0s", "[Server]: GO"]
    );
    let packets = run(&mut alice, "!admin /countdown 3601").await;
    assert_eq!(
        admin_results(&packets),
        ["3601 is invalid. Maximum is 1 hour."]
    );
}
