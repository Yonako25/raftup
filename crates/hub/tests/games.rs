//! Parties hébergées par le hub (ADR 0052, étape 1), sans réseau extérieur :
//! envoi d'une seed, connexion avec `client-core` comme un mod non modifié (adresse
//! `hôte:port` sans schéma), arrêt et relance du hub avec reprise de la sauvegarde,
//! limites, export, suppression.
//!
//! Chaque test prend sa propre plage de ports (47100 et plus) pour que les tests
//! lancés en parallèle ne se gênent pas ; un port déjà pris est sauté par le hub.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ap_protocol::client::{ClientPacket, LocationChecks};
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use base64::Engine;
use client_core::{Client, ClientError, ConnectOptions, Fingerprint};
use hub::games::GamesConfig;
use hub::{AppState, EnvSettings, HubConfig, Store};
use multidata::raftup::{seed_sha256, SaveFile};
use serde_json::{json, Value};
use tower::ServiceExt;

mod common;

use common::*;

fn seed_bytes() -> Vec<u8> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago");
    std::fs::read(path).unwrap()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Réglages d'un hub de test qui héberge des parties dans `dir`, sur dix ports à
/// partir de `first_port`, seulement sur 127.0.0.1 (pas de fenêtre du pare-feu).
fn config(dir: &Path, first_port: u16, hosting: bool) -> HubConfig {
    HubConfig {
        settings_env: EnvSettings::from_lookup(|name| {
            (name == "RAFTUP_HUB_HOSTING").then(|| hosting.to_string())
        }),
        games: GamesConfig {
            data_dir: dir.to_path_buf(),
            first_port,
            last_port: first_port + 9,
            listen_host: "127.0.0.1".to_string(),
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

/// Envoie la seed de test (et une sauvegarde) au nom de `token`.
async fn upload(
    app: &Router,
    token: &str,
    name: &str,
    password: Option<&str>,
    save: Option<&[u8]>,
) -> (StatusCode, Value) {
    let mut body = json!({ "name": name, "seed": b64(&seed_bytes()) });
    if let Some(password) = password {
        body["password"] = json!(password);
    }
    if let Some(save) = save {
        body["save"] = json!(b64(save));
    }
    call(app, Method::POST, "/v1/games", Some(token), Some(body)).await
}

/// Connexion comme un mod : adresse `hôte:port`, `wss` essayé puis `ws`.
async fn join(
    address: &str,
    slot: &str,
    password: Option<&str>,
) -> Result<(Client, Vec<i64>), ClientError> {
    let mut options = ConnectOptions::new(address, slot, "ChecksFinder");
    options.password = password.map(str::to_string);
    let (client, session) = Client::connect(&options).await?;
    Ok((client, session.connected.checked_locations))
}

/// Attend que l'état de la partie montre `checked` lieux visités pour Alice.
async fn wait_for_checks(app: &Router, token: &str, id: &str, checked: u64) -> Value {
    for _ in 0..50 {
        let (status, body) = call(
            app,
            Method::GET,
            &format!("/v1/games/{id}"),
            Some(token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let alice = body["slots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|slot| slot["name"] == "Alice")
            .cloned()
            .unwrap();
        if alice["checked"] == checked {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("Alice n'a jamais {checked} lieux visités");
}

/// Envoie des lieux visités et attend la première réponse du serveur : la
/// sauvegarde est alors déjà sur le disque (NF07).
async fn check_locations(client: &mut Client, locations: Vec<i64>) {
    client
        .send(&[ClientPacket::LocationChecks(LocationChecks { locations })])
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), client.recv())
        .await
        .unwrap()
        .unwrap();
}

/// Requête brute (pour l'export, qui n'est pas du JSON).
async fn get_bytes(app: &Router, uri: &str, token: &str) -> (StatusCode, Vec<u8>) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 << 20)
        .await
        .unwrap();
    (status, bytes.to_vec())
}

fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut files = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        let mut content = Vec::new();
        file.read_to_end(&mut content).unwrap();
        files.push((file.name().unwrap().to_string(), content));
    }
    files
}

fn game_dir(root: &Path, id: &str) -> PathBuf {
    root.join(id)
}

#[tokio::test(flavor = "multi_thread")]
async fn heberger_jouer_puis_reprendre_apres_relance_du_hub() {
    let dir = tempfile::tempdir().unwrap();
    let store = test_store().await;
    let config = config(dir.path(), 47100, true);
    let (state, app) = hub(&store, config.clone()).await;
    let host = account(&app, "Hote").await;

    let (status, created) = upload(&app, &host, "Partie du jeudi", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(id.len(), 32);
    let port = created["port"].as_i64().unwrap();
    assert!((47100..=47109).contains(&port));
    assert_eq!(created["address"], format!("127.0.0.1:{port}"));
    assert_eq!(created["state"], "active");
    assert_eq!(created["players"], 3);
    assert_eq!(created["resumed"], false);
    let fingerprint = created["fingerprint"].as_str().unwrap().to_string();
    assert!(Fingerprint::parse(&fingerprint).is_some());
    let admin_password = created["admin_password"].as_str().unwrap().to_string();
    assert!(admin_password.len() >= 16);
    assert!(game_dir(dir.path(), &id)
        .join("partie.archipelago")
        .exists());

    // Un mod non modifié : seulement `hôte:port`. Le certificat auto-signé fait
    // échouer `wss`, le client retombe sur `ws` (port mixte, ADR 0043).
    let address = created["address"].as_str().unwrap().to_string();
    let mut options = ConnectOptions::new(&address, "Alice", "ChecksFinder");
    let (mut alice, session) = Client::connect(&options).await.unwrap();
    assert!(session.url.starts_with("ws://"), "{}", session.url);
    check_locations(&mut alice, vec![81003, 81006]).await;
    let status = wait_for_checks(&app, &host, &id, 2).await;
    assert!(status["connections"].as_u64().unwrap() >= 1);
    assert_eq!(status["is_host"], true);
    assert_eq!(status["admin_password"], admin_password.as_str());
    alice.close().await.unwrap();

    // Le launcher, lui, épingle le certificat : `wss` passe.
    options.pinned = Fingerprint::parse(&fingerprint);
    let (pinned, session) = Client::connect(&options).await.unwrap();
    assert!(session.url.starts_with("wss://"), "{}", session.url);
    pinned.close().await.unwrap();

    // Arrêt du hub (sauvegardes compactées), puis relance sur la même base.
    state.games().shutdown().await;
    assert!(join(&address, "Alice", None).await.is_err());
    let (_state, app) = hub(&store, config).await;
    let (status, body) = call(
        &app,
        Method::GET,
        &format!("/v1/games/{id}"),
        Some(&host),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "active");
    assert_eq!(body["port"], port);
    assert_eq!(body["fingerprint"], fingerprint.as_str());
    assert_eq!(body["admin_password"], admin_password.as_str());

    // La partie a repris là où elle en était.
    let (alice, checked) = join(&address, "Alice", None).await.unwrap();
    assert_eq!(checked, vec![81003, 81006]);
    alice.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mot_de_passe_chiffre_et_admin_seulement_pour_l_hote() {
    let dir = tempfile::tempdir().unwrap();
    let store = test_store().await;
    let (_state, app) = hub(&store, config(dir.path(), 47120, true)).await;
    let host = account(&app, "Hote").await;
    let other = account(&app, "Autre").await;

    let (status, created) = upload(&app, &host, "Privée", Some("sésame ouvre-toi"), None).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["has_password"], true);
    let id = created["id"].as_str().unwrap();
    let address = created["address"].as_str().unwrap();
    let admin_password = created["admin_password"].as_str().unwrap();

    // Ni le mot de passe ni celui d'admin ne sont en clair dans la base.
    let row = store.game(id).await.unwrap().unwrap();
    let stored = format!("{row:?}");
    assert!(!stored.contains("sésame"));
    assert!(!stored.contains(admin_password));
    assert!(row.password_enc.is_some());

    // Le serveur de jeu, lui, applique le mot de passe.
    assert!(matches!(
        join(address, "Alice", None).await,
        Err(ClientError::Refused(_))
    ));
    let (alice, _) = join(address, "Alice", Some("sésame ouvre-toi"))
        .await
        .unwrap();
    alice.close().await.unwrap();

    // Un joueur qui connaît l'identifiant voit l'état, pas le mot de passe d'admin.
    let (status, body) = call(
        &app,
        Method::GET,
        &format!("/v1/games/{id}"),
        Some(&other),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["is_host"], false);
    assert!(body.get("admin_password").is_none());
    // Il ne la voit pas dans « mes parties ».
    let (_, mine) = call(&app, Method::GET, "/v1/games", Some(&other), None).await;
    assert_eq!(mine["games"], json!([]));
    let (_, mine) = call(&app, Method::GET, "/v1/games", Some(&host), None).await;
    assert_eq!(mine["games"].as_array().unwrap().len(), 1);
    // Sans compte : refusé.
    let (status, _) = call(&app, Method::GET, &format!("/v1/games/{id}"), None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn limites_par_compte_par_hub_et_par_partie() {
    let dir = tempfile::tempdir().unwrap();
    let store = test_store().await;
    let mut config = config(dir.path(), 47140, true);
    config.games.max_total = 3;
    let (_state, app) = hub(&store, config).await;
    let first = account(&app, "Premier").await;
    let second = account(&app, "Second").await;

    // 2 parties par compte.
    for name in ["Une", "Deux"] {
        let (status, body) = upload(&app, &first, name, None, None).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    let (status, body) = upload(&app, &first, "Trois", None, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error_code(&body), "game_limit_reached");

    // Hub plein (3 ici, 40 par défaut).
    let (status, body) = upload(&app, &second, "Une", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body) = upload(&app, &second, "Deux", None, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(&body), "hub_full");

    // Trois parties, trois ports différents.
    let mut ports: Vec<i64> = store
        .all_games()
        .await
        .unwrap()
        .iter()
        .map(|row| row.port)
        .collect();
    ports.sort_unstable();
    ports.dedup();
    assert_eq!(ports.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn seeds_refusees() {
    let dir = tempfile::tempdir().unwrap();
    let store = test_store().await;
    let mut config = config(dir.path(), 47160, true);
    config.games.max_players = 2;
    let (_state, app) = hub(&store, config).await;
    let token = account(&app, "Hote").await;

    // Trop de joueurs (la seed en a 3, sans compter le groupe).
    let (status, body) = upload(&app, &token, "Grosse", None, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "too_many_players");

    // Pas une seed.
    let junk = json!({ "name": "x", "seed": b64(b"pas une seed du tout") });
    let (status, body) = call(&app, Method::POST, "/v1/games", Some(&token), Some(junk)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "invalid_seed");

    // Nom vide, base64 illisible.
    let empty = json!({ "name": "  ", "seed": b64(&seed_bytes()) });
    let (_, body) = call(&app, Method::POST, "/v1/games", Some(&token), Some(empty)).await;
    assert_eq!(error_code(&body), "invalid_game_name");
    let bad = json!({ "name": "x", "seed": "@@@" });
    let (_, body) = call(&app, Method::POST, "/v1/games", Some(&token), Some(bad)).await;
    assert_eq!(error_code(&body), "bad_request");

    // Rien n'est resté sur le disque ni en base.
    assert!(store.all_games().await.unwrap().is_empty());
    let leftovers = std::fs::read_dir(dir.path())
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.path().is_dir() && e.file_name() != "tls")
                .count()
        })
        .unwrap_or(0);
    assert_eq!(leftovers, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn seed_trop_grosse_et_sauvegarde_etrangere() {
    let dir = tempfile::tempdir().unwrap();
    let store = test_store().await;
    let mut config = config(dir.path(), 47180, true);
    config.games.max_seed_bytes = 1000;
    let (_state, app) = hub(&store, config.clone()).await;
    let token = account(&app, "Hote").await;
    let (status, body) = upload(&app, &token, "x", None, None).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(error_code(&body), "seed_too_large");

    // Corps de 100 Kio (au-delà des 16 Kio du reste de l'API) : la route a sa propre
    // limite, la requête arrive jusqu'à la lecture de la sauvegarde, refusée.
    let (_state, app) = hub(&test_store().await, {
        let mut c = config;
        c.games.max_seed_bytes = 32 << 20;
        c
    })
    .await;
    let token = account(&app, "Hote").await;
    let junk = vec![42u8; 100 * 1024];
    let (status, body) = upload(&app, &token, "x", None, Some(&junk)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(error_code(&body), "invalid_save");
}

#[tokio::test(flavor = "multi_thread")]
async fn hebergement_coupe_par_les_reglages() {
    let dir = tempfile::tempdir().unwrap();
    let store = test_store().await;
    let (_state, app) = hub(&store, config(dir.path(), 47200, false)).await;
    let (_, info) = call(&app, Method::GET, "/v1/info", None, None).await;
    assert_eq!(info["hosting"], false);
    let token = account(&app, "Hote").await;
    let (status, body) = upload(&app, &token, "x", None, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(&body), "hosting_disabled");

    // Activé dans la base (console) : passe avant l'environnement.
    store
        .put_setting(hub::SettingKey::Hosting.name(), "true", 0)
        .await
        .unwrap();
    let (_, info) = call(&app, Method::GET, "/v1/info", None, None).await;
    assert_eq!(info["hosting"], true);
    let (status, body) = upload(&app, &token, "x", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn export_reprise_ailleurs_et_suppression() {
    let dir = tempfile::tempdir().unwrap();
    let store = test_store().await;
    let (_state, app) = hub(&store, config(dir.path(), 47220, true)).await;
    let host = account(&app, "Hote").await;
    let other = account(&app, "Autre").await;

    let (_, created) = upload(&app, &host, "À exporter", None, None).await;
    let id = created["id"].as_str().unwrap().to_string();
    let address = created["address"].as_str().unwrap().to_string();
    let (mut alice, _) = join(&address, "Alice", None).await.unwrap();
    check_locations(&mut alice, vec![81001]).await;
    alice.close().await.unwrap();

    // Export : l'hôte seulement.
    let export = format!("/v1/games/{id}/export");
    let (status, _) = get_bytes(&app, &export, &other).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, zip) = get_bytes(&app, &export, &host).await;
    assert_eq!(status, StatusCode::OK);
    let files = unzip(&zip);
    let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["partie.archipelago", "partie.raftup"]);
    assert_eq!(files[0].1, seed_bytes());
    // La sauvegarde exportée se relit comme en self-host, avec le lieu visité.
    let save_path = dir.path().join("export.raftup");
    std::fs::write(&save_path, &files[1].1).unwrap();
    let (_, snapshot) = SaveFile::open(&save_path, &seed_sha256(&seed_bytes())).unwrap();
    let alice_checks = snapshot
        .location_checks
        .iter()
        .find(|entry| entry.slot == 1)
        .unwrap();
    assert_eq!(alice_checks.locations, vec![81001]);

    // Une partie déjà commencée part sur le hub avec sa sauvegarde (sens inverse).
    let (status, resumed) = upload(&app, &other, "Suite", None, Some(&files[1].1)).await;
    assert_eq!(status, StatusCode::CREATED, "{resumed}");
    assert_eq!(resumed["resumed"], true);
    let (alice, checked) = join(resumed["address"].as_str().unwrap(), "Alice", None)
        .await
        .unwrap();
    assert_eq!(checked, vec![81001]);
    alice.close().await.unwrap();

    // Suppression : l'hôte seulement ; serveur arrêté, base et dossier vidés.
    let uri = format!("/v1/games/{id}");
    let (status, body) = call(&app, Method::DELETE, &uri, Some(&other), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&body), "not_game_host");
    let (status, _) = call(&app, Method::DELETE, &uri, Some(&host), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = call(&app, Method::GET, &uri, Some(&host), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "game_not_found");
    assert!(!game_dir(dir.path(), &id).exists());
    assert!(join(&address, "Alice", None).await.is_err());
    // Identifiant mal formé : comme inconnu, sans toucher au disque.
    let (status, _) = call(&app, Method::GET, "/v1/games/..%2F..", Some(&host), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn suppression_du_compte_supprime_ses_parties() {
    let dir = tempfile::tempdir().unwrap();
    let store = test_store().await;
    let (_state, app) = hub(&store, config(dir.path(), 47240, true)).await;
    let host = account(&app, "Hote").await;
    let (_, created) = upload(&app, &host, "Éphémère", None, None).await;
    let id = created["id"].as_str().unwrap().to_string();
    let address = created["address"].as_str().unwrap().to_string();

    let (status, _) = call(
        &app,
        Method::DELETE,
        "/v1/me",
        Some(&host),
        Some(json!({ "password": "mot de passe solide" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(store.game(&id).await.unwrap().is_none());
    assert!(!game_dir(dir.path(), &id).exists());
    assert!(join(&address, "Alice", None).await.is_err());
}

/// Une base en version 6 (avant les parties hébergées) passe en version 8 : la
/// table `hosted_games` est créée et sert tout de suite.
#[tokio::test(flavor = "multi_thread")]
async fn base_en_version_6_migree() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hub.db");
    let url = format!("sqlite://{}", path.display());
    drop(Store::open(&url).await.unwrap());
    let raw = sqlx::AnyPool::connect(&format!("sqlite:{}", path.display()))
        .await
        .unwrap();
    for statement in [
        "DROP TABLE hosted_games",
        "UPDATE schema_version SET version = 6",
    ] {
        sqlx::query(statement).execute(&raw).await.unwrap();
    }
    raw.close().await;

    let store = Store::open(&url).await.unwrap();
    let version: i64 = sqlx::Row::get(
        &sqlx::query("SELECT version FROM schema_version")
            .fetch_one(
                &sqlx::AnyPool::connect(&format!("sqlite:{}", path.display()))
                    .await
                    .unwrap(),
            )
            .await
            .unwrap(),
        "version",
    );
    assert_eq!(version, hub::store::SCHEMA_VERSION);
    assert_eq!(version, 8);
    let (_state, app) = hub(&store, config(&dir.path().join("parties"), 47260, true)).await;
    let token = account(&app, "Hote").await;
    let (status, body) = upload(&app, &token, "Après migration", None, None).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}
