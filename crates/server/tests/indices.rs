//! Indices dans la salle : `LocationScouts` avec `create_as_hint`, `CreateHints`,
//! `UpdateHint`. Les textes attendus viennent du code Python (MultiServer.py 0.6.8).
//!
//! Seed MIT : Alice = slot 1, Bob = slot 2, Chloé = slot 3, groupe Lien = slot 4.
//! Chez Alice, 81000 contient un objet de Chloé et 81001 un objet de Bob.
//! Chez Bob, 81002 contient un objet de Chloé, 81018 un objet d'Alice.
//! Alice a déjà des indices de départ dans la seed (dont Bob 81004).

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use multidata::{load_archipelago, ArchiveLimits};
use serde_json::{json, Value};
use server::{Room, RunningServer, Server};
use tokio::net::TcpStream;
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

/// Connecte `name` (ChecksFinder) et vide ce qui suit `Connected`.
async fn join(url: &str, name: &str, tags: Value) -> Client {
    let (mut client, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    client.next().await.unwrap().unwrap(); // RoomInfo
    let connect = json!([{"cmd":"Connect","password":null,"game":"ChecksFinder","name":name,
        "uuid":"u","version":{"major":0,"minor":6,"build":8,"class":"Version"},
        "items_handling":7,"tags":tags}]);
    client
        .send(Message::text(connect.to_string()))
        .await
        .unwrap();
    let frame = recv(&mut client).await;
    assert_eq!(frame[0]["cmd"], "Connected");
    packets(&mut client).await;
    client
}

async fn send(client: &mut Client, packet: Value) {
    client
        .send(Message::text(json!([packet]).to_string()))
        .await
        .unwrap();
}

/// Prochaine trame (tableau de paquets).
async fn recv(client: &mut Client) -> Vec<Value> {
    let message = tokio::time::timeout(Duration::from_secs(5), client.next())
        .await
        .expect("pas de trame")
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

/// Tous les paquets reçus jusqu'à un court silence, à plat.
async fn packets(client: &mut Client) -> Vec<Value> {
    let mut all = Vec::new();
    while let Ok(Some(Ok(message))) =
        tokio::time::timeout(Duration::from_millis(300), client.next()).await
    {
        if let Ok(text) = message.to_text() {
            let frame: Vec<Value> = serde_json::from_str(text).unwrap_or_default();
            all.extend(frame);
        }
    }
    all
}

/// Les `cmd` des paquets, dans l'ordre.
fn cmds(packets: &[Value]) -> Vec<&str> {
    packets.iter().map(|p| p["cmd"].as_str().unwrap()).collect()
}

/// La connexion est fermée par le serveur.
async fn assert_closed(client: &mut Client) {
    loop {
        let next = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .expect("connexion encore ouverte");
        match next {
            None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
            Some(Ok(_)) => {}
        }
    }
}

/// Réponse unique attendue : un `InvalidPacket` avec ce texte.
async fn expect_invalid(client: &mut Client, original_cmd: &str, text: &str) {
    let got = packets(client).await;
    assert_eq!(
        got,
        vec![
            json!({"cmd":"InvalidPacket","type":"arguments","text":text,"original_cmd":original_cmd})
        ]
    );
}

async fn subscribe(client: &mut Client, keys: &[&str]) {
    send(client, json!({"cmd":"SetNotify","keys":keys})).await;
}

/// L'indice d'une liste `_read_hints_…` pour ce lieu (la seed en contient déjà d'autres).
fn hint_at(list: &Value, finding: u32, location: i64) -> Value {
    let found = list
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["finding_player"] == finding && h["location"] == location);
    found.cloned().expect("indice absent de la liste")
}

fn hint_json(
    receiving: u32,
    finding: u32,
    location: i64,
    item: i64,
    found: bool,
    status: u8,
) -> Value {
    json!({"class":"Hint","receiving_player":receiving,"finding_player":finding,
        "location":location,"item":item,"found":found,"entrance":"",
        "item_flags":1,"status":status})
}

#[tokio::test]
async fn scouts_sans_indice() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", json!([])).await;
    subscribe(&mut alice, &["_read_hints_0_1"]).await;
    send(
        &mut alice,
        json!({"cmd":"LocationScouts","locations":[81001]}),
    )
    .await;
    send(
        &mut alice,
        json!({"cmd":"LocationScouts","locations":[81001],"create_as_hint":0}),
    )
    .await;
    let got = packets(&mut alice).await;
    assert_eq!(cmds(&got), ["LocationInfo", "LocationInfo"]);
}

#[tokio::test]
async fn scouts_cree_un_indice() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", json!([])).await;
    let mut bob = join(&url, "Bob", json!(["NoText"])).await;
    packets(&mut alice).await; // « Bob a rejoint la partie »
    subscribe(&mut alice, &["_read_hints_0_1"]).await;
    send(
        &mut alice,
        json!({"cmd":"LocationScouts","locations":[81001],"create_as_hint":1}),
    )
    .await;

    let hint = hint_json(2, 1, 81001, 80001, false, 0);
    let got = packets(&mut alice).await;
    // LocationInfo d'abord, puis SetReply et RoomUpdate du slot 1, puis le texte.
    assert_eq!(
        cmds(&got),
        ["LocationInfo", "SetReply", "RoomUpdate", "PrintJSON"]
    );
    assert_eq!(got[1]["key"], "_read_hints_0_1");
    assert_eq!(hint_at(&got[1]["value"], 1, 81001), hint);
    assert_eq!(got[2]["hint_points"], 0);
    let text = &got[3];
    assert_eq!(text["type"], "Hint");
    assert_eq!(text["receiving"], 2);
    assert_eq!(text["found"], false);
    let rendered: String = text["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| part["text"].as_str().unwrap())
        .collect();
    assert_eq!(
        rendered,
        "[Hint]: 2's 80001 is at 81001 in 1's World. (unspecified)"
    );

    // Bob (NoText) ne reçoit que la mise à jour des points, pas le texte.
    let got = packets(&mut bob).await;
    assert_eq!(cmds(&got), ["RoomUpdate"]);

    // Les clés de lecture le confirment, pour les deux slots.
    send(
        &mut alice,
        json!({"cmd":"Get","keys":["_read_hints_0_1","_read_hints_0_2"]}),
    )
    .await;
    let got = packets(&mut alice).await;
    assert_eq!(hint_at(&got[0]["keys"]["_read_hints_0_1"], 1, 81001), hint);
    assert_eq!(hint_at(&got[0]["keys"]["_read_hints_0_2"], 1, 81001), hint);
}

#[tokio::test]
async fn scouts_deux_ignore_les_indices_connus() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", json!([])).await;
    subscribe(&mut alice, &["_read_hints_0_1"]).await;
    let scout =
        |mode: u8| json!({"cmd":"LocationScouts","locations":[81001],"create_as_hint":mode});
    send(&mut alice, scout(2)).await;
    assert_eq!(
        cmds(&packets(&mut alice).await),
        ["LocationInfo", "SetReply", "RoomUpdate", "PrintJSON"]
    );
    // Mode 2, indice déjà connu : seulement `LocationInfo`.
    send(&mut alice, scout(2)).await;
    assert_eq!(cmds(&packets(&mut alice).await), ["LocationInfo"]);
    // Mode 1 : l'indice connu est renvoyé en texte, mais rien de nouveau n'est mémorisé.
    send(&mut alice, scout(1)).await;
    assert_eq!(
        cmds(&packets(&mut alice).await),
        ["LocationInfo", "PrintJSON"]
    );
}

#[tokio::test]
async fn lieu_visite_passe_a_trouve() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", json!([])).await;
    subscribe(&mut alice, &["_read_hints_0_1"]).await;
    send(
        &mut alice,
        json!({"cmd":"LocationScouts","locations":[81001],"create_as_hint":1}),
    )
    .await;
    packets(&mut alice).await;

    send(
        &mut alice,
        json!({"cmd":"LocationChecks","locations":[81001]}),
    )
    .await;
    let got = packets(&mut alice).await;
    let reply = got
        .iter()
        .find(|p| p["cmd"] == "SetReply")
        .expect("SetReply");
    assert_eq!(
        hint_at(&reply["value"], 1, 81001),
        hint_json(2, 1, 81001, 80001, true, 40)
    );
    // Un lieu visité rapporte un point d'indice.
    let update = got
        .iter()
        .find(|p| p["cmd"] == "RoomUpdate")
        .expect("RoomUpdate");
    assert_eq!(update["hint_points"], 1);
}

#[tokio::test]
async fn create_hints_valide() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", json!([])).await;
    let mut bob = join(&url, "Bob", json!([])).await;
    packets(&mut alice).await; // « Bob a rejoint la partie »
    subscribe(&mut alice, &["_read_hints_0_1"]).await;
    // Lieu de Bob dont l'objet revient à Alice : elle peut choisir le statut.
    send(
        &mut alice,
        json!({"cmd":"CreateHints","locations":[81018],"player":2,"status":30}),
    )
    .await;
    // Pas de `LocationInfo` : SetReply, RoomUpdate, texte.
    let got = packets(&mut alice).await;
    assert_eq!(cmds(&got), ["SetReply", "RoomUpdate", "PrintJSON"]);
    assert_eq!(
        hint_at(&got[0]["value"], 2, 81018),
        hint_json(1, 2, 81018, 80001, false, 30)
    );
    let parts = got[2]["data"].as_array().unwrap();
    assert_eq!(parts.last().unwrap()["text"], "(priority)");
    // Bob, qui trouve l'objet, est prévenu aussi.
    assert_eq!(cmds(&packets(&mut bob).await), ["RoomUpdate", "PrintJSON"]);

    // Déjà connu : `only_new`, rien de nouveau (même avec un autre statut).
    send(
        &mut alice,
        json!({"cmd":"CreateHints","locations":[81018],"player":2,"status":10}),
    )
    .await;
    assert!(packets(&mut alice).await.is_empty());

    // Lieu d'Alice avec un objet de Bob : statut « non précisé » par défaut.
    send(&mut alice, json!({"cmd":"CreateHints","locations":[81001]})).await;
    let got = packets(&mut alice).await;
    assert_eq!(cmds(&got), ["SetReply", "RoomUpdate", "PrintJSON"]);
    assert_eq!(
        hint_at(&got[0]["value"], 1, 81001),
        hint_json(2, 1, 81001, 80001, false, 0)
    );
}

#[tokio::test]
async fn create_hints_erreurs() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", json!([])).await;
    let count = |value: &Value| value.as_array().unwrap().len();
    send(&mut alice, json!({"cmd":"Get","keys":["_read_hints_0_1"]})).await;
    let before = count(&packets(&mut alice).await[0]["keys"]["_read_hints_0_1"]);

    send(&mut alice, json!({"cmd":"CreateHints","locations":[]})).await;
    expect_invalid(
        &mut alice,
        "CreateHints",
        "CreateHints: No locations specified.",
    )
    .await;

    send(
        &mut alice,
        json!({"cmd":"CreateHints","locations":[81000],"status":7}),
    )
    .await;
    expect_invalid(
        &mut alice,
        "CreateHints",
        "Unknown Status: 7 is not a valid HintStatus",
    )
    .await;
    // Hors de la plage d'un octet : même réponse.
    send(
        &mut alice,
        json!({"cmd":"CreateHints","locations":[81000],"status":300}),
    )
    .await;
    expect_invalid(
        &mut alice,
        "CreateHints",
        "Unknown Status: 300 is not a valid HintStatus",
    )
    .await;

    send(
        &mut alice,
        json!({"cmd":"CreateHints","locations":[99999],"player":2}),
    )
    .await;
    expect_invalid(
        &mut alice,
        "CreateHints",
        "CreateHints: One or more of the locations do not exist for the specified off-world \
         player. Please refrain from hinting other slot's locations that you don't know \
         contain your items.",
    )
    .await;

    // 81000 contient un objet de Chloé : pas d'autre statut que « non précisé ».
    send(
        &mut alice,
        json!({"cmd":"CreateHints","locations":[81000],"status":30}),
    )
    .await;
    expect_invalid(
        &mut alice,
        "CreateHints",
        "CreateHints: Must use \"unspecified\"/None status for items from other players.",
    )
    .await;

    // Lieu de Bob dont l'objet est pour Chloé : ni à Alice, ni dans son monde.
    send(
        &mut alice,
        json!({"cmd":"CreateHints","locations":[81002],"player":2}),
    )
    .await;
    expect_invalid(
        &mut alice,
        "CreateHints",
        "CreateHints: Can only create hints for own items or own locations.",
    )
    .await;

    // Aucune erreur ci-dessus n'a laissé d'indice.
    send(&mut alice, json!({"cmd":"Get","keys":["_read_hints_0_1"]})).await;
    let got = packets(&mut alice).await;
    assert_eq!(count(&got[0]["keys"]["_read_hints_0_1"]), before);
}

#[tokio::test]
async fn create_hints_plantages_du_serveur_officiel() {
    let (_server, url) = start().await;
    // Joueur inconnu.
    let mut alice = join(&url, "Alice", json!([])).await;
    send(
        &mut alice,
        json!({"cmd":"CreateHints","locations":[81000],"player":9}),
    )
    .await;
    assert_closed(&mut alice).await;
    // Lieu inconnu dans son propre monde.
    let mut alice = join(&url, "Alice", json!([])).await;
    send(&mut alice, json!({"cmd":"CreateHints","locations":[99999]})).await;
    assert_closed(&mut alice).await;
    // Champ obligatoire absent.
    let mut alice = join(&url, "Alice", json!([])).await;
    send(&mut alice, json!({"cmd":"CreateHints"})).await;
    assert_closed(&mut alice).await;
}

/// Alice a un indice pour Bob ; Bob s'abonne aux deux listes.
async fn alice_hints_bob(url: &str) -> (Client, Client) {
    let mut alice = join(url, "Alice", json!([])).await;
    let mut bob = join(url, "Bob", json!([])).await;
    packets(&mut alice).await; // « Bob a rejoint la partie »
    subscribe(&mut bob, &["_read_hints_0_1", "_read_hints_0_2"]).await;
    send(&mut alice, json!({"cmd":"CreateHints","locations":[81001]})).await;
    packets(&mut alice).await;
    packets(&mut bob).await;
    (alice, bob)
}

#[tokio::test]
async fn update_hint_valide() {
    let (_server, url) = start().await;
    let (_alice, mut bob) = alice_hints_bob(&url).await;
    send(
        &mut bob,
        json!({"cmd":"UpdateHint","player":1,"location":81001,"status":30}),
    )
    .await;
    // Ni `PrintJSON` ni `RoomUpdate` : un `SetReply` par slot concerné.
    let got = packets(&mut bob).await;
    assert_eq!(cmds(&got), ["SetReply", "SetReply"]);
    let hint = hint_json(2, 1, 81001, 80001, false, 30);
    assert_eq!(got[0]["key"], "_read_hints_0_1");
    assert_eq!(hint_at(&got[0]["value"], 1, 81001), hint);
    assert_eq!(got[1]["key"], "_read_hints_0_2");
    assert_eq!(hint_at(&got[1]["value"], 1, 81001), hint);
    // Même statut : rien ne change, aucune réponse.
    send(
        &mut bob,
        json!({"cmd":"UpdateHint","player":1,"location":81001,"status":30}),
    )
    .await;
    assert!(packets(&mut bob).await.is_empty());
}

#[tokio::test]
async fn update_hint_cas_ignores() {
    let (_server, url) = start().await;
    let (_alice, mut bob) = alice_hints_bob(&url).await;
    // Indice inconnu, statut nul : ignorés sans réponse.
    send(
        &mut bob,
        json!({"cmd":"UpdateHint","player":1,"location":81002,"status":30}),
    )
    .await;
    send(
        &mut bob,
        json!({"cmd":"UpdateHint","player":1,"location":81001,"status":null}),
    )
    .await;
    assert!(packets(&mut bob).await.is_empty());
}

#[tokio::test]
async fn update_hint_erreurs() {
    let (_server, url) = start().await;
    let (mut alice, mut bob) = alice_hints_bob(&url).await;
    let update =
        |status: Value| json!({"cmd":"UpdateHint","player":1,"location":81001,"status":status});
    // Alice trouve l'objet mais ne le reçoit pas : pas le droit.
    send(&mut alice, update(json!(30))).await;
    expect_invalid(&mut alice, "UpdateHint", "UpdateHint: No Permission").await;

    send(&mut bob, update(json!(7))).await;
    expect_invalid(&mut bob, "UpdateHint", "UpdateHint: Invalid Status").await;
    send(&mut bob, update(json!(40))).await;
    expect_invalid(
        &mut bob,
        "UpdateHint",
        "UpdateHint: Cannot manually update status to \"HINT_FOUND\"",
    )
    .await;
    // Mauvais types.
    send(
        &mut bob,
        json!({"cmd":"UpdateHint","player":"1","location":81001,"status":30}),
    )
    .await;
    expect_invalid(&mut bob, "UpdateHint", "UpdateHint").await;
    send(&mut bob, update(json!("x"))).await;
    expect_invalid(&mut bob, "UpdateHint", "UpdateHint").await;
    // Statut hors de la plage d'un octet : refusé comme invalide, l'indice existe.
    send(&mut bob, update(json!(300))).await;
    expect_invalid(&mut bob, "UpdateHint", "UpdateHint: Invalid Status").await;
    // Champ `player` absent : le serveur officiel plante, la connexion est fermée.
    send(
        &mut bob,
        json!({"cmd":"UpdateHint","location":81001,"status":30}),
    )
    .await;
    assert_closed(&mut bob).await;
}

#[tokio::test]
async fn create_hints_statut_de_tout_type() {
    let (_server, url) = start().await;
    let mut alice = join(&url, "Alice", json!([])).await;
    // `HintStatus(valeur)` de Python : le message montre le `repr()` de la valeur.
    for (status, shown) in [
        (json!(null), "None"),
        (json!("30"), "'30'"),
        (json!(30.5), "30.5"),
        (json!([]), "[]"),
        (json!(true), "True"),
    ] {
        send(
            &mut alice,
            json!({"cmd":"CreateHints","locations":[81000],"status":status}),
        )
        .await;
        let text = format!("Unknown Status: {shown} is not a valid HintStatus");
        expect_invalid(&mut alice, "CreateHints", &text).await;
    }
    // `0.0` et `false` valent 0 en Python : statut « non précisé », accepté.
    for status in [json!(0.0), json!(false)] {
        send(
            &mut alice,
            json!({"cmd":"CreateHints","locations":[81001],"status":status}),
        )
        .await;
        let got = packets(&mut alice).await;
        assert!(
            !cmds(&got).contains(&"InvalidPacket"),
            "{status} refusé : {got:?}"
        );
    }
    // `player` à `null` : le serveur officiel plante, connexion fermée.
    send(
        &mut alice,
        json!({"cmd":"CreateHints","locations":[81000],"player":null}),
    )
    .await;
    assert_closed(&mut alice).await;
}

#[tokio::test]
async fn update_hint_statut_absent_ou_booleen() {
    let (_server, url) = start().await;
    let (_alice, mut bob) = alice_hints_bob(&url).await;
    // `True` est l'entier 1 en Python : statut invalide, pas un mauvais type.
    send(
        &mut bob,
        json!({"cmd":"UpdateHint","player":1,"location":81001,"status":true}),
    )
    .await;
    expect_invalid(&mut bob, "UpdateHint", "UpdateHint: Invalid Status").await;
    // `status` absent (et non `null`) : `KeyError` en Python, connexion fermée.
    send(
        &mut bob,
        json!({"cmd":"UpdateHint","player":1,"location":81001}),
    )
    .await;
    assert_closed(&mut bob).await;
}
