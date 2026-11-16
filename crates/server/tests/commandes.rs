//! Commandes du chat (`!help`, `!players`, `!missing`…) : textes du serveur officiel.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use multidata::{load_archipelago, ArchiveLimits, MultiData};
use serde_json::{json, Value};
use server::{Room, RunningServer, Server};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn seed() -> MultiData {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago");
    load_archipelago(&std::fs::read(path).unwrap(), &ArchiveLimits::default()).unwrap()
}

async fn start_with(data: MultiData) -> (RunningServer, String) {
    let server = Server::bind("127.0.0.1:0", Room::new(data))
        .await
        .unwrap()
        .start()
        .unwrap();
    let url = format!("ws://{}", server.local_addr());
    (server, url)
}

async fn start() -> (RunningServer, String) {
    start_with(seed()).await
}

/// Ouvre une connexion et se connecte au slot `name`.
async fn join(url: &str, name: &str, game: &str, tags: &str) -> Client {
    join_with(url, name, game, tags, "null").await
}

/// Comme `join`, avec le mot de passe de la salle en JSON (`"null"` ou `"\"texte\""`).
async fn join_with(url: &str, name: &str, game: &str, tags: &str, password: &str) -> Client {
    let (mut client, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    client.next().await.unwrap().unwrap(); // RoomInfo
    let connect = format!(
        r#"[{{"cmd":"Connect","password":{password},"game":"{game}","name":"{name}",
        "uuid":"u","version":{{"major":0,"minor":6,"build":8,"class":"Version"}},
        "items_handling":7,"tags":{tags}}}]"#
    );
    client.send(Message::text(connect)).await.unwrap();
    client.next().await.unwrap().unwrap(); // Connected
    drain(&mut client).await;
    client
}

/// Lit tout ce qui arrive jusqu'à un court silence ; une trame par élément.
async fn drain_frames(client: &mut Client) -> Vec<Vec<Value>> {
    let mut frames = Vec::new();
    while let Ok(Some(Ok(message))) =
        tokio::time::timeout(Duration::from_millis(300), client.next()).await
    {
        let packets: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        frames.push(packets.as_array().unwrap().clone());
    }
    frames
}

async fn drain(client: &mut Client) -> Vec<Value> {
    drain_frames(client).await.into_iter().flatten().collect()
}

async fn say(client: &mut Client, text: &str) {
    let packet = json!([{"cmd": "Say", "text": text}]);
    client
        .send(Message::text(packet.to_string()))
        .await
        .unwrap();
}

/// Envoie `text` et renvoie les paquets reçus ensuite.
async fn run(client: &mut Client, text: &str) -> Vec<Value> {
    say(client, text).await;
    drain(client).await
}

/// Textes des `CommandResult` d'une liste de paquets.
fn results(packets: &[Value]) -> Vec<String> {
    packets
        .iter()
        .filter(|p| p["type"] == "CommandResult")
        .map(|p| p["data"][0]["text"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn commande_inconnue() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", "ChecksFinder", "[]").await;
    let packets = run(&mut alice, "!Bidule truc").await;
    // Le chat est diffusé d'abord, puis la réponse.
    assert_eq!(packets[0]["type"], "Chat");
    assert_eq!(packets[0]["data"][0]["text"], "Alice: !Bidule truc");
    assert_eq!(
        results(&packets),
        [
            "Could not find command Bidule. Known commands: help, license, options, admin, \
players, status, release, collect, countdown, remaining, missing, checked, alias, getitem, \
hint, hint_location"
        ]
    );
    // Le point d'exclamation seul est aussi une commande inconnue, au nom vide.
    let packets = run(&mut alice, "!").await;
    assert!(results(&packets)[0].starts_with("Could not find command . Known commands:"));
}

#[tokio::test]
async fn chat_simple_sans_reponse() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", "ChecksFinder", "[]").await;
    let packets = run(&mut alice, "bonjour !help").await;
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0]["type"], "Chat");
    // Une ligne d'espaces : chat, rien d'autre.
    let packets = run(&mut alice, "   ").await;
    assert_eq!(packets.len(), 1);
}

#[tokio::test]
async fn guillemets_et_majuscules() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", "ChecksFinder", "[]").await;
    // Premier mot entre guillemets, nom en majuscules : c'est `!license`.
    let packets = run(&mut alice, "\"!LICENSE\"").await;
    assert!(results(&packets)[0].starts_with("Raftup is dual-licensed"));
    // Espace de tête : shlex l'ignore.
    let packets = run(&mut alice, " !license").await;
    assert_eq!(results(&packets).len(), 1);
    // Guillemet non fermé : repli sur le découpage par espaces.
    let packets = run(&mut alice, "!status \"DeathLink").await;
    assert_eq!(results(&packets).len(), 1);
    // Trop d'arguments : texte court à la place de la trace Python.
    let packets = run(&mut alice, "!help encore").await;
    assert_eq!(results(&packets), ["Invalid arguments for !help."]);
}

#[tokio::test]
async fn aide_et_licence() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", "ChecksFinder", "[]").await;
    let packets = run(&mut alice, "!help").await;
    let text = &results(&packets)[0];
    assert!(text.starts_with("!help \n    Returns the help listing\n!license \n"));
    assert!(text.contains("!status [tag] \n    Get information about your team.\n"));
    assert!(text.contains("!countdown seconds=10 \n"));
    assert!(text.ends_with("to get a spoiler peek for that location.\n"));
    // Le résultat n'est que pour le demandeur.
    let mut bob = join(&url, "Bob", "ChecksFinder", "[]").await;
    drain(&mut alice).await;
    let _ = run(&mut alice, "!license").await;
    assert!(results(&drain(&mut bob).await).is_empty());
}

#[tokio::test]
async fn options_une_trame_par_ligne() {
    let mut data = seed();
    data.server_options.password = Some("secret".to_string());
    data.server_options.server_password = Some("admin".to_string());
    data.server_options.disable_item_cheat = true;
    let (_server, url) = start_with(data).await;
    let mut alice = join_with(&url, "Alice", "ChecksFinder", "[]", "\"secret\"").await;
    say(&mut alice, "!options").await;
    let frames = drain_frames(&mut alice).await;
    // Chat, puis « Current options: » et dix options, chacun dans sa trame.
    assert_eq!(frames.len(), 12);
    assert!(frames.iter().all(|f| f.len() == 1));
    let lines: Vec<String> = frames[1..]
        .iter()
        .map(|f| f[0]["data"][0]["text"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(lines[0], "Current options:");
    assert_eq!(lines[1], "Option hint_cost is set to 10");
    assert_eq!(lines[2], "Option location_check_points is set to 1");
    assert_eq!(lines[3], "Option server_password is set to ********");
    assert_eq!(lines[4], "Option password is set to ********");
    assert_eq!(lines[5], "Option release_mode is set to auto");
    assert_eq!(lines[6], "Option remaining_mode is set to goal");
    assert_eq!(lines[7], "Option collect_mode is set to auto");
    assert_eq!(lines[8], "Option countdown_mode is set to auto");
    assert_eq!(lines[9], "Option item_cheat is set to False");
    assert_eq!(lines[10], "Option compatibility is set to 2");
}

#[tokio::test]
async fn joueurs_connectes() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", "ChecksFinder", "[]").await;
    let mut bob = join(&url, "Bob", "ChecksFinder", "[]").await;
    drain(&mut alice).await;
    // Moins de 10 noms : tout le monde reçoit la réponse.
    say(&mut alice, "!players").await;
    let first = results(&drain(&mut alice).await);
    assert_eq!(
        first,
        ["2 players of 3 connected :: Team #1: Alice Bob (Chloé)"]
    );
    assert_eq!(results(&drain(&mut bob).await), first);
}

#[tokio::test]
async fn statut_de_l_equipe() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", "ChecksFinder", r#"["DeathLink"]"#).await;
    let _bob = join(&url, "Bob", "ChecksFinder", "[]").await;
    drain(&mut alice).await;
    alice
        .send(Message::text(
            json!([{"cmd":"LocationChecks","locations":[81000,81001]},
                   {"cmd":"StatusUpdate","status":10}])
            .to_string(),
        ))
        .await
        .unwrap();
    drain(&mut alice).await;
    let packets = run(&mut alice, "!status").await;
    assert_eq!(
        results(&packets),
        ["Player Status on team 0:\n\
Alice has 1 connection and is ready. (2/25)\n\
Bob has 1 connection. (0/25)\n\
Chloé has 0 connections. (0/88)"]
    );
    let packets = run(&mut alice, "!status DeathLink").await;
    assert_eq!(
        results(&packets),
        ["Player Status on team 0:\n\
Alice has 1 connection 1 of which are tagged DeathLink and is ready. (2/25)\n\
Bob has 1 connection 0 of which are tagged DeathLink. (0/25)\n\
Chloé has 0 connections. (0/88)"]
    );
}

#[tokio::test]
async fn restant_selon_le_mode() {
    // Mode `goal` (défaut) : refusé tant que le jeu n'est pas fini.
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", "ChecksFinder", "[]").await;
    let packets = run(&mut alice, "!remaining").await;
    assert_eq!(
        results(&packets),
        ["Sorry, !remaining requires you to have beaten the game on this server"]
    );
    // Objectif atteint : la liste apparaît.
    alice
        .send(Message::text(
            json!([{"cmd":"StatusUpdate","status":30}]).to_string(),
        ))
        .await
        .unwrap();
    drain(&mut alice).await;
    let packets = run(&mut alice, "!remaining").await;
    let text = &results(&packets)[0];
    // L'objectif déclenche la distribution automatique : il ne reste rien.
    assert_eq!(text, "No remaining items found.");

    let mut data = seed();
    data.server_options.remaining_mode = "disabled".to_string();
    let (_server, url) = start_with(data).await;
    let mut alice = join(&url, "Alice", "ChecksFinder", "[]").await;
    let packets = run(&mut alice, "!remaining").await;
    assert_eq!(
        results(&packets),
        ["Sorry, !remaining has been disabled on this server."]
    );

    let mut data = seed();
    data.server_options.remaining_mode = "enabled".to_string();
    let (_server, url) = start_with(data).await;
    let mut alice = join(&url, "Alice", "ChecksFinder", "[]").await;
    let packets = run(&mut alice, "!remaining").await;
    let before = results(&packets)[0].clone();
    assert!(before.starts_with("Remaining items: "), "{before}");
    // Tous les lieux visités : plus rien.
    let all: Vec<u32> = (81000..81025).collect();
    alice
        .send(Message::text(
            json!([{"cmd":"LocationChecks","locations":all}]).to_string(),
        ))
        .await
        .unwrap();
    drain(&mut alice).await;
    let packets = run(&mut alice, "!remaining").await;
    assert_eq!(results(&packets), ["No remaining items found."]);
}

#[tokio::test]
async fn lieux_manquants_et_visites() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", "ChecksFinder", "[]").await;
    say(&mut alice, "!checked").await;
    let frames = drain_frames(&mut alice).await;
    assert_eq!(
        results(&frames.concat()),
        ["No done location checks found."]
    );

    alice
        .send(Message::text(
            json!([{"cmd":"LocationChecks","locations":[81000,81001]}]).to_string(),
        ))
        .await
        .unwrap();
    drain(&mut alice).await;
    say(&mut alice, "!checked").await;
    let frames = drain_frames(&mut alice).await;
    // Chat, puis une seule trame avec tous les résultats.
    assert_eq!(frames.len(), 2);
    let texts = results(&frames[1]);
    assert_eq!(texts.len(), 3);
    assert!(texts[0].starts_with("Checked: "));
    assert_eq!(texts[2], "Found 2 done location checks");

    say(&mut alice, "!missing").await;
    let frames = drain_frames(&mut alice).await;
    let texts = results(&frames[1]);
    assert_eq!(texts.len(), 24);
    assert_eq!(texts[23], "Found 23 missing location checks");

    // Filtre : sous-chaîne sensible à la casse (une casse fausse ne trouve rien).
    let first = texts[0].trim_start_matches("Missing: ").to_string();
    let piece = &first[..3];
    say(&mut alice, &format!("!missing {piece}")).await;
    let frames = drain_frames(&mut alice).await;
    let texts = results(&frames[1]);
    assert!(texts.len() >= 2);
    let last = texts.last().unwrap();
    assert!(
        last.starts_with("Found 23 missing location checks, displaying "),
        "{last}"
    );
    say(&mut alice, "!missing ZZZ-introuvable").await;
    let frames = drain_frames(&mut alice).await;
    assert_eq!(
        results(&frames[1]),
        ["Found 23 missing location checks, displaying 0 of them."]
    );
}

#[tokio::test]
async fn admin_masque_le_mot_de_passe() {
    let mut data = seed();
    data.server_options.server_password = Some("admin".to_string());
    let (_server, url) = start_with(data).await;
    let mut alice = join(&url, "Alice", "ChecksFinder", "[]").await;
    let mut bob = join(&url, "Bob", "ChecksFinder", "[]").await;
    drain(&mut alice).await; // son message « Join »
    let packets = run(&mut alice, "!admin login admin").await;
    let chat = &packets[0];
    assert_eq!(chat["type"], "Chat");
    assert_eq!(chat["data"][0]["text"], "Alice: !admin login ********");
    assert_eq!(chat["message"], "!admin login ********");
    // Un seul message de chat, et Bob le reçoit aussi, masqué.
    assert_eq!(packets.iter().filter(|p| p["type"] == "Chat").count(), 1);
    let seen = drain(&mut bob).await;
    assert_eq!(seen[0]["data"][0]["text"], "Alice: !admin login ********");

    // Sans mot de passe d'administration : désactivé.
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", "ChecksFinder", "[]").await;
    let packets = run(&mut alice, "!admin /help").await;
    assert_eq!(
        results(&packets),
        ["Sorry, Remote administration is disabled"]
    );
    assert_eq!(packets.iter().filter(|p| p["type"] == "Chat").count(), 1);
}
