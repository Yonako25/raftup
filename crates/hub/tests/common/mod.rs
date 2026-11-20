//! Aides partagées par les tests d'intégration du hub.
//!
//! Par défaut, chaque test a sa base SQLite en mémoire. Si la variable
//! `RAFTUP_HUB_TEST_POSTGRES` contient l'URL d'un PostgreSQL de test
//! (`postgres://utilisateur@localhost:5432/base`), chaque test y reçoit à la place
//! son propre schéma, neuf, pour que les tests lancés en parallèle ne se gênent pas.
//! De même, `RAFTUP_HUB_TEST_MYSQL` (`mysql://root:motdepasse@localhost:3306`, pour
//! MySQL ou MariaDB) donne à chaque test sa propre base, neuve (ADR 0037).

// Chaque fichier de test n'utilise pas toutes les aides.
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use hub::{AppState, HubConfig, Store};
use serde_json::{json, Value};
use tower::ServiceExt;

/// Variable d'environnement qui active les tests sur PostgreSQL.
pub const POSTGRES_ENV: &str = "RAFTUP_HUB_TEST_POSTGRES";

/// Variable d'environnement qui active les tests sur MySQL / MariaDB.
pub const MYSQL_ENV: &str = "RAFTUP_HUB_TEST_MYSQL";

/// Base de test : SQLite en mémoire, ou un schéma neuf de PostgreSQL, ou une base
/// neuve de MySQL / MariaDB.
pub async fn test_store() -> Store {
    if let Ok(url) = std::env::var(MYSQL_ENV) {
        if !url.trim().is_empty() {
            return mysql_store(url.trim()).await;
        }
    }
    match std::env::var(POSTGRES_ENV) {
        Ok(url) if !url.trim().is_empty() => postgres_store(url.trim()).await,
        _ => Store::in_memory().await.unwrap(),
    }
}

/// Nom unique pour la base ou le schéma d'un test.
fn unique_name() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "raftup_test_{}_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        nanos % 1_000_000_000
    )
}

/// Crée une base au nom unique sur le serveur MySQL / MariaDB, puis ouvre le hub dessus.
pub async fn mysql_store(server_url: &str) -> Store {
    let name = unique_name();
    sqlx::any::install_default_drivers();
    let admin = sqlx::AnyPool::connect(server_url).await.unwrap();
    sqlx::query(&format!(
        "CREATE DATABASE {name} CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
    ))
    .execute(&admin)
    .await
    .unwrap();
    admin.close().await;
    let base = server_url.trim_end_matches('/');
    Store::open(&format!("{base}/{name}")).await.unwrap()
}

/// Crée un schéma au nom unique, puis ouvre le hub dessus (`search_path`).
pub async fn postgres_store(url: &str) -> Store {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!(
        "raftup_test_{}_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        nanos % 1_000_000_000
    );
    sqlx::any::install_default_drivers();
    let admin = sqlx::AnyPool::connect(url).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    // `options=-c search_path=…` (encodé dans l'URL) : les tables du hub vont dans ce schéma.
    let separator = if url.contains('?') { '&' } else { '?' };
    let url = format!("{url}{separator}options=-c%20search_path%3D{schema}");
    Store::open(&url).await.unwrap()
}

// ---------- Requêtes vers le routeur ----------

pub async fn app_with(config: HubConfig) -> Router {
    let store = test_store().await;
    let state = AppState::new(store, config).await.unwrap();
    hub::router(state)
}

pub async fn app() -> Router {
    app_with(HubConfig::for_tests()).await
}

/// Envoie une requête et renvoie le code HTTP et le corps JSON (`Null` si vide).
pub async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    // `oneshot` consomme le service : on clone le routeur (bon marché).
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

pub async fn register(
    app: &Router,
    email: &str,
    username: &str,
    password: &str,
) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/v1/accounts",
        None,
        Some(json!({ "email": email, "username": username, "password": password })),
    )
    .await
}

pub async fn login(app: &Router, email: &str, password: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/v1/sessions",
        None,
        Some(json!({ "email": email, "password": password })),
    )
    .await
}

/// Crée un compte et renvoie son jeton de session.
pub async fn account(app: &Router, username: &str) -> String {
    let email = format!("{}@example.org", username.to_lowercase());
    let (status, _) = register(app, &email, username, "mot de passe solide").await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = login(app, &email, "mot de passe solide").await;
    assert_eq!(status, StatusCode::OK);
    body["token"].as_str().unwrap().to_string()
}

pub fn error_code(body: &Value) -> &str {
    body["error"].as_str().unwrap_or("")
}

pub async fn send_request(app: &Router, token: &str, username: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/v1/friends/requests",
        Some(token),
        Some(json!({ "username": username })),
    )
    .await
}

pub async fn friends(app: &Router, token: &str) -> Value {
    let (status, body) = call(app, Method::GET, "/v1/friends", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
    body
}

pub async fn set_presence(app: &Router, token: &str, presence: Value) -> StatusCode {
    call(
        app,
        Method::PUT,
        "/v1/presence",
        Some(token),
        Some(presence),
    )
    .await
    .0
}
