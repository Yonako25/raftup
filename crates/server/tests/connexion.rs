//! Connexion des joueurs (J1, étape 4), comparée au serveur officiel.
//!
//! `tests/data/connexion-seed-mit.json` contient des scénarios rejoués sur le
//! serveur officiel 0.6.8 avec la seed MIT (voir `tests/data/README.md`) :
//! pour chaque étape, le paquet envoyé et les trames reçues par chaque client.
//! Ce test rejoue les mêmes scénarios sur Raftup et compare les trames.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use multidata::{load_archipelago, ArchiveLimits};
use serde_json::Value;
use server::{Limits, Room, Server};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Marque d'une connexion fermée par le serveur, comme dans la capture.
const CLOSED: &str = "<fermé>";

fn data_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

async fn start() -> String {
    start_with(Limits::default()).await
}

/// Comme `start`, avec d'autres limites (ADR 0007) : le serveur officiel n'en a
/// pas, et une capture peut envoyer plus de chats que Raftup n'en accepte.
async fn start_with(limits: Limits) -> String {
    let path = data_path("../multidata/tests/data/seed-mit.archipelago");
    let data = load_archipelago(&std::fs::read(path).unwrap(), &ArchiveLimits::default()).unwrap();
    let server = Server::bind("127.0.0.1:0", Room::new(data))
        .await
        .unwrap()
        .with_limits(limits);
    let address = server.local_addr().unwrap();
    tokio::spawn(server.run());
    format!("ws://{address}")
}

/// Lit tout ce que le serveur envoie, jusqu'à un court silence.
async fn drain(client: &mut Client) -> Vec<String> {
    let mut frames = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_millis(250), client.next()).await {
            Err(_) => return frames,
            Ok(Some(Ok(Message::Text(text)))) => frames.push(text.to_string()),
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Ok(Some(Err(_))) => {
                frames.push(CLOSED.to_string());
                return frames;
            }
            Ok(Some(Ok(_))) => {}
        }
    }
}

/// Forme comparable d'une trame : la valeur JSON, avec les listes qui viennent
/// d'un ensemble Python triées (erreurs de `ConnectionRefused`, lieux de
/// `RoomUpdate`).
fn normalize(frame: &str) -> Value {
    if frame == CLOSED {
        return Value::String(CLOSED.to_string());
    }
    let frame = fixed_password_mask(frame);
    let mut value: Value = serde_json::from_str(&frame).unwrap();
    for packet in value.as_array_mut().unwrap() {
        if let Some(Value::Array(errors)) = packet.get_mut("errors") {
            errors.sort_by_key(|e| e.to_string());
        }
        // `RoomUpdate.checked_locations` vient aussi d'un ensemble Python.
        if packet["cmd"] == "RoomUpdate" {
            if let Some(Value::Array(locations)) = packet.get_mut("checked_locations") {
                locations.sort_by_key(|l| l.as_i64());
            }
        }
        // Les indices d'un slot (`_read_hints_…`) ne sont pas triés : Raftup
        // suit l'ordre du `set` Python (ADR 0018).
    }
    value
}

/// Mot de passe masqué après `!admin login ` : le serveur officiel tire 4 à 16
/// étoiles au hasard, Raftup en met toujours 8. On ramène chaque suite d'étoiles
/// à 8 pour comparer.
fn fixed_password_mask(frame: &str) -> String {
    const PREFIX: &str = "!admin login ";
    let mut out = String::new();
    let mut rest = frame;
    // `find` renvoie la position du préfixe ; on recopie ce qui le précède, puis
    // on saute les étoiles qui le suivent.
    while let Some(start) = rest.find(PREFIX) {
        let after = start + PREFIX.len();
        out.push_str(&rest[..after]);
        let stars = rest[after..].len() - rest[after..].trim_start_matches('*').len();
        if stars > 0 {
            out.push_str("********");
        }
        rest = &rest[after + stars..];
    }
    out.push_str(rest);
    out
}

async fn replay(url: &str, name: &str, steps: &[Value]) {
    // Les clients du scénario, ouverts dans l'ordre alphabétique comme en Python.
    let mut names: Vec<&str> = steps
        .iter()
        .map(|s| s["client"].as_str().unwrap())
        .collect();
    names.sort();
    names.dedup();
    let mut clients = Vec::new();
    for _ in &names {
        let (mut client, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        drain(&mut client).await; // RoomInfo
        clients.push(client);
    }

    // `closed[i]` : le serveur a fermé la connexion du client `i` ; comme dans la
    // capture, il n'envoie plus rien et chaque étape note de nouveau `<fermé>`.
    let mut closed = vec![false; names.len()];
    for (number, step) in steps.iter().enumerate() {
        let sender = step["client"].as_str().unwrap();
        let index = names.iter().position(|n| *n == sender).unwrap();
        let packets = serde_json::to_string(&step["envoi"]).unwrap();
        if !closed[index] {
            clients[index].send(Message::text(packets)).await.unwrap();
        }

        for (i, client_name) in names.iter().enumerate() {
            let frames = if closed[i] {
                vec![CLOSED.to_string()]
            } else {
                drain(&mut clients[i]).await
            };
            if frames.last().is_some_and(|f| f == CLOSED) {
                closed[i] = true;
            }
            let ours: Vec<Value> = frames.iter().map(|f| normalize(f)).collect();
            let theirs: Vec<Value> = step["recu"][client_name]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| normalize(f.as_str().unwrap()))
                .collect();
            assert_eq!(
                ours, theirs,
                "scénario « {name} », étape {number}, trames reçues par « {client_name} »"
            );
        }
    }
}

#[tokio::test]
async fn connexion_comme_python() {
    let text = std::fs::read_to_string(data_path("tests/data/connexion-seed-mit.json")).unwrap();
    let scenarios: serde_json::Map<String, Value> = serde_json::from_str(&text).unwrap();
    assert!(scenarios.len() >= 16);
    // Un serveur neuf par scénario : aucun ne dépend des précédents.
    for (name, steps) in &scenarios {
        let url = start().await;
        replay(&url, name, steps.as_array().unwrap()).await;
    }
}

/// Le départ d'un joueur est annoncé aux autres (`Part`).
#[tokio::test]
async fn depart_annonce() {
    let url = start().await;
    let connect = |name: &str, game: &str, tags: &str| {
        format!(
            r#"[{{"cmd":"Connect","password":null,"game":"{game}","name":"{name}","uuid":"u",
            "version":{{"major":0,"minor":6,"build":8,"class":"Version"}},
            "items_handling":7,"tags":{tags}}}]"#
        )
    };
    let (mut alice, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let (mut bob, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    alice
        .send(Message::text(connect("Alice", "ChecksFinder", "[]")))
        .await
        .unwrap();
    bob.send(Message::text(connect("Bob", "", r#"["TextOnly"]"#)))
        .await
        .unwrap();
    drain(&mut alice).await;
    drain(&mut bob).await;

    bob.close(None).await.unwrap();
    let frames = drain(&mut alice).await;
    assert_eq!(frames.len(), 1, "{frames:?}");
    let part = &normalize(&frames[0])[0];
    assert_eq!(part["type"], "Part");
    assert_eq!(
        part["data"][0]["text"],
        "Bob (Team #1) has stopped viewing the game. Client(0.6.8), ['TextOnly']."
    );
}

/// Mot de passe de la salle : refusé s'il est faux, accepté s'il est juste.
#[tokio::test]
async fn mot_de_passe() {
    let path = data_path("../multidata/tests/data/seed-mit.archipelago");
    let data = load_archipelago(&std::fs::read(path).unwrap(), &ArchiveLimits::default()).unwrap();
    let room = Room::new(data).with_password(Some("secret".to_string()));
    let server = Server::bind("127.0.0.1:0", room).await.unwrap();
    let url = format!("ws://{}", server.local_addr().unwrap());
    tokio::spawn(server.run());

    let connect = |password: &str| {
        format!(
            r#"[{{"cmd":"Connect","password":{password},"game":"ChecksFinder","name":"Alice",
            "uuid":"u","version":{{"major":0,"minor":6,"build":8,"class":"Version"}},
            "items_handling":7,"tags":[]}}]"#
        )
    };
    let (mut client, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    drain(&mut client).await;

    for wrong in ["null", r#""""#, r#""Secret""#] {
        client.send(Message::text(connect(wrong))).await.unwrap();
        let frames = drain(&mut client).await;
        let reply = &normalize(&frames[0])[0];
        assert_eq!(reply["cmd"], "ConnectionRefused", "mot de passe {wrong}");
        assert_eq!(reply["errors"], serde_json::json!(["InvalidPassword"]));
    }

    client
        .send(Message::text(connect(r#""secret""#)))
        .await
        .unwrap();
    let frames = drain(&mut client).await;
    assert_eq!(normalize(&frames[0])[0]["cmd"], "Connected");
}

/// Une partie entière (lieux visités, objets, `Sync`, `ConnectUpdate`),
/// capturée sur le serveur officiel : `tests/data/partie-seed-mit.json`.
#[tokio::test]
async fn partie_comme_python() {
    let text = std::fs::read_to_string(data_path("tests/data/partie-seed-mit.json")).unwrap();
    let scenarios: serde_json::Map<String, Value> = serde_json::from_str(&text).unwrap();
    for (name, steps) in &scenarios {
        let url = start().await;
        replay(&url, name, steps.as_array().unwrap()).await;
    }
}

/// Statuts et objectif atteint, avec `collect` et `release` automatiques :
/// `tests/data/objectif-seed-mit.json`, capturé sur le serveur officiel.
#[tokio::test]
async fn objectif_comme_python() {
    let text = std::fs::read_to_string(data_path("tests/data/objectif-seed-mit.json")).unwrap();
    let scenarios: serde_json::Map<String, Value> = serde_json::from_str(&text).unwrap();
    for (name, steps) in &scenarios {
        let url = start().await;
        replay(&url, name, steps.as_array().unwrap()).await;
    }
}

/// `LocationScouts`, chat et `Bounce` : `tests/data/echanges-seed-mit.json`,
/// capturé sur le serveur officiel.
#[tokio::test]
async fn echanges_comme_python() {
    let text = std::fs::read_to_string(data_path("tests/data/echanges-seed-mit.json")).unwrap();
    let scenarios: serde_json::Map<String, Value> = serde_json::from_str(&text).unwrap();
    for (name, steps) in &scenarios {
        let url = start().await;
        replay(&url, name, steps.as_array().unwrap()).await;
    }
}

/// Stockage de données (`Get`, `Set`, `SetNotify`, clés `_read_…`) :
/// `tests/data/stockage-seed-mit.json`, capturé sur le serveur officiel.
#[tokio::test]
async fn stockage_comme_python() {
    let text = std::fs::read_to_string(data_path("tests/data/stockage-seed-mit.json")).unwrap();
    let scenarios: serde_json::Map<String, Value> = serde_json::from_str(&text).unwrap();
    for (name, steps) in &scenarios {
        let url = start().await;
        replay(&url, name, steps.as_array().unwrap()).await;
    }
}

/// Paquets mal formés (`cmd` qui n'est pas du texte, paquet sans `cmd`, outil
/// de suivi qui envoie des lieux) : écarts trouvés par les tests différentiels,
/// capturés ensuite sur le serveur officiel (`tests/data/malformes-seed-mit.json`).
#[tokio::test]
async fn malformes_comme_python() {
    let text = std::fs::read_to_string(data_path("tests/data/malformes-seed-mit.json")).unwrap();
    let scenarios: serde_json::Map<String, Value> = serde_json::from_str(&text).unwrap();
    for (name, steps) in &scenarios {
        let url = start().await;
        replay(&url, name, steps.as_array().unwrap()).await;
    }
}

/// Un `Connect` refusé d'un client déjà connecté change quand même son
/// `items_handling` : écart trouvé par les tests différentiels, capturé ensuite
/// sur le serveur officiel (`tests/data/refus-connecte-seed-mit.json`).
#[tokio::test]
async fn refus_connecte_comme_python() {
    let text =
        std::fs::read_to_string(data_path("tests/data/refus-connecte-seed-mit.json")).unwrap();
    let scenarios: serde_json::Map<String, Value> = serde_json::from_str(&text).unwrap();
    for (name, steps) in &scenarios {
        let url = start().await;
        replay(&url, name, steps.as_array().unwrap()).await;
    }
}

/// Commandes `!` du chat (aide, statut, lieux restants ou visités, `!admin`) :
/// `tests/data/commandes-seed-mit.json`, capturé sur le serveur officiel.
#[tokio::test]
async fn commandes_comme_python() {
    let text = std::fs::read_to_string(data_path("tests/data/commandes-seed-mit.json")).unwrap();
    let scenarios: serde_json::Map<String, Value> = serde_json::from_str(&text).unwrap();
    // Plus de 5 chats en 10 s : la limite de Raftup les refuserait.
    let limits = Limits {
        chat_messages: 100,
        ..Limits::default()
    };
    for (name, steps) in &scenarios {
        // `Limits` est `Copy` : chaque appel reçoit sa propre copie.
        let url = start_with(limits).await;
        replay(&url, name, steps.as_array().unwrap()).await;
    }
}

/// Indices (`LocationScouts` avec `create_as_hint`, `CreateHints`, `UpdateHint`) :
/// `tests/data/indices-seed-mit.json`, capturé sur le serveur officiel. Le
/// serveur officiel y ferme une connexion, que le rejeu suit.
#[tokio::test]
async fn indices_comme_python() {
    let text = std::fs::read_to_string(data_path("tests/data/indices-seed-mit.json")).unwrap();
    let scenarios: serde_json::Map<String, Value> = serde_json::from_str(&text).unwrap();
    for (name, steps) in &scenarios {
        let url = start().await;
        replay(&url, name, steps.as_array().unwrap()).await;
    }
}
