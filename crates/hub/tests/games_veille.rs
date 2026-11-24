//! Veille et réveil des parties hébergées (ADR 0052, étape 3, point 6), sans réseau
//! extérieur et avec des délais de quelques centaines de millisecondes :
//! - une partie où quelqu'un joue reste éveillée, et sa dernière activité est notée ;
//! - vide trop longtemps, elle s'endort, mais son port reste ouvert ;
//! - un mod qui se connecte sur ce port la réveille et joue, sans rien savoir ;
//! - `POST /v1/games/{id}/wake` la réveille aussi, puis elle se rendort après la
//!   grâce si personne n'entre ;
//! - en veille au moment d'un arrêt du hub, elle repart en veille ;
//! - une base en version 7 passe en version 8.
//!
//! Plage de ports propre à ce fichier : 47500 et plus.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::time::{Duration, Instant};

use ap_protocol::client::{ClientPacket, LocationChecks};
use axum::http::{Method, StatusCode};
use axum::Router;
use base64::Engine;
use client_core::{Client, ConnectOptions};
use hub::games::GamesConfig;
use hub::{AppState, EnvSettings, HubConfig, Store};
use serde_json::{json, Value};

mod common;

use common::*;

/// Délai de veille des tests ; la grâce après un réveil est la même.
const IDLE: Duration = Duration::from_millis(400);

fn seed_bytes() -> Vec<u8> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago");
    std::fs::read(path).unwrap()
}

fn config(dir: &Path, first_port: u16) -> HubConfig {
    HubConfig {
        settings_env: EnvSettings::from_lookup(|name| {
            (name == "RAFTUP_HUB_HOSTING").then(|| "true".to_string())
        }),
        games: GamesConfig {
            data_dir: dir.to_path_buf(),
            first_port,
            last_port: first_port + 9,
            listen_host: "127.0.0.1".to_string(),
            idle_after: IDLE,
            wake_grace: IDLE,
            activity_check: Duration::from_millis(50),
            ..GamesConfig::default()
        },
        ..HubConfig::for_tests()
    }
}

async fn hub(store: &Store, config: HubConfig) -> (AppState, Router) {
    let state = AppState::new(store.clone(), config).await.unwrap();
    let app = hub::router(state.clone());
    (state, app)
}

/// Envoie la seed de test et rend la réponse du hub.
async fn upload(app: &Router, token: &str) -> Value {
    let seed = base64::engine::general_purpose::STANDARD.encode(seed_bytes());
    let body = json!({ "name": "Partie longue", "seed": seed });
    let (status, created) = call(app, Method::POST, "/v1/games", Some(token), Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created
}

async fn game(app: &Router, token: &str, id: &str) -> Value {
    let uri = format!("/v1/games/{id}");
    let (status, body) = call(app, Method::GET, &uri, Some(token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Attend que la partie soit dans l'état `expected` (5 s au plus).
async fn wait_for_state(app: &Router, token: &str, id: &str, expected: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let body = game(app, token, id).await;
        if body["state"] == expected {
            return body;
        }
        assert!(Instant::now() < deadline, "jamais « {expected} » : {body}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Connexion comme un mod non modifié : `hôte:port`, `wss` essayé puis `ws`. Rend
/// le client et les lieux déjà visités.
async fn join(address: &str) -> (Client, Vec<i64>) {
    let options = ConnectOptions::new(address, "Alice", "ChecksFinder");
    let (client, session) = Client::connect(&options).await.unwrap();
    (client, session.connected.checked_locations)
}

async fn check_location(client: &mut Client, location: i64) {
    client
        .send(&[ClientPacket::LocationChecks(LocationChecks {
            locations: vec![location],
        })])
        .await
        .unwrap();
    // La première réponse arrive après l'écriture de la sauvegarde (NF07).
    tokio::time::timeout(Duration::from_secs(5), client.recv())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn veille_puis_reveil_par_un_mod_et_par_le_launcher() {
    let dir = tempfile::tempdir().unwrap();
    let store = test_store().await;
    let config = config(dir.path(), 47500);
    let (state, app) = hub(&store, config.clone()).await;
    let host = account(&app, "Hote").await;

    let created = upload(&app, &host).await;
    let id = created["id"].as_str().unwrap().to_string();
    let address = created["address"].as_str().unwrap().to_string();
    assert!(created["last_activity_at"].is_null());

    // Alice joue plus longtemps que le délai de veille : la partie reste éveillée,
    // et sa dernière activité est notée.
    let (mut alice, _) = join(&address).await;
    check_location(&mut alice, 81001).await;
    tokio::time::sleep(IDLE * 2).await;
    let body = game(&app, &host, &id).await;
    assert_eq!(body["state"], "active", "{body}");
    assert!(body["last_activity_at"].as_i64().is_some(), "{body}");
    alice.close().await.unwrap();

    // Plus personne : la partie s'endort, garde son empreinte et son port ouvert.
    let body = wait_for_state(&app, &host, &id, "sleeping").await;
    assert_eq!(body["fingerprint"], created["fingerprint"]);
    assert_eq!(body["slots"].as_array().unwrap().len(), 0);
    let (status, list) = call(&app, Method::GET, "/v1/games", Some(&host), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["games"][0]["state"], "sleeping");

    // Un mod se connecte sur le port : la partie se réveille et il joue, avec la
    // sauvegarde d'avant la veille (compactée à l'arrêt).
    let (mut alice, checked) = join(&address).await;
    assert_eq!(checked, vec![81001]);
    assert_eq!(game(&app, &host, &id).await["state"], "active");
    check_location(&mut alice, 81002).await;
    alice.close().await.unwrap();
    wait_for_state(&app, &host, &id, "sleeping").await;

    // Le launcher la réveille ; sans personne qui entre, elle se rendort après la
    // grâce. N'importe quel compte qui connaît l'identifiant peut la réveiller.
    let other = account(&app, "Autre").await;
    let uri = format!("/v1/games/{id}/wake");
    let (status, woken) = call(&app, Method::POST, &uri, Some(&other), None).await;
    assert_eq!(status, StatusCode::OK, "{woken}");
    assert_eq!(woken["state"], "active");
    // Réveiller une partie éveillée ne change rien.
    let (status, again) = call(&app, Method::POST, &uri, Some(&other), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["state"], "active");
    wait_for_state(&app, &host, &id, "sleeping").await;
    let (status, body) = call(
        &app,
        Method::POST,
        "/v1/games/0123456789abcdef0123456789abcdef/wake",
        Some(&host),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "game_not_found");

    // Arrêt du hub pendant la veille : au redémarrage, la partie est toujours en
    // veille sur son port, et un mod la réveille comme avant.
    state.games().shutdown().await;
    let (_state, app) = hub(&store, config).await;
    assert_eq!(game(&app, &host, &id).await["state"], "sleeping");
    let (alice, checked) = join(&address).await;
    assert_eq!(checked, vec![81001, 81002]);
    alice.close().await.unwrap();
    wait_for_state(&app, &host, &id, "sleeping").await;

    // Supprimée pendant la veille : le port se ferme.
    let uri = format!("/v1/games/{id}");
    let (status, _) = call(&app, Method::DELETE, &uri, Some(&host), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(tokio::net::TcpStream::connect(&address).await.is_err());
}

/// Une base en version 7 (parties hébergées sans veille) passe en version 8 : les
/// colonnes `last_activity_at` et `asleep` sont ajoutées, la partie existante est
/// gardée et repart éveillée.
#[tokio::test(flavor = "multi_thread")]
async fn base_en_version_7_migree() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hub.db");
    let url = format!("sqlite://{}", path.display());
    let parties = dir.path().join("parties");
    let mut config = config(&parties, 47520);
    // Pas de veille pendant ce test : seule la migration compte.
    config.games.idle_after = Duration::from_secs(3600);
    // Le jeton de session est gardé en base : il sert encore après la migration.
    let (host, id) = {
        let store = Store::open(&url).await.unwrap();
        let (state, app) = hub(&store, config.clone()).await;
        let host = account(&app, "Hote").await;
        let id = upload(&app, &host).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        state.games().shutdown().await;
        (host, id)
    };
    let raw = sqlx::AnyPool::connect(&format!("sqlite:{}", path.display()))
        .await
        .unwrap();
    for statement in [
        "ALTER TABLE hosted_games DROP COLUMN last_activity_at",
        "ALTER TABLE hosted_games DROP COLUMN asleep",
        "UPDATE schema_version SET version = 7",
    ] {
        sqlx::query(statement).execute(&raw).await.unwrap();
    }
    raw.close().await;

    let store = Store::open(&url).await.unwrap();
    let (_state, app) = hub(&store, config).await;
    let row = store.game(&id).await.unwrap().unwrap();
    assert!(!row.asleep);
    assert_eq!(row.last_activity_at, None);
    let body = game(&app, &host, &id).await;
    assert_eq!(body["state"], "active", "{body}");
    assert!(body["last_activity_at"].is_null());
}
