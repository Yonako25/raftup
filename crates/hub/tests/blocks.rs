//! Blocages entre joueurs (ADR 0033) : bloquer, lever le blocage, et leurs effets
//! sur les amis, les demandes et la présence.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used)]

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

mod common;

use common::*;

async fn block(app: &axum::Router, token: &str, username: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/v1/blocks",
        Some(token),
        Some(json!({ "username": username })),
    )
    .await
}

async fn blocks(app: &axum::Router, token: &str) -> Value {
    let (status, body) = call(app, Method::GET, "/v1/blocks", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
    body
}

async fn requests(app: &axum::Router, token: &str) -> Value {
    let (status, body) = call(app, Method::GET, "/v1/friends/requests", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
    body
}

#[tokio::test]
async fn bloquer_retire_l_amitie_et_la_presence() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;
    send_request(&app, &alice, "Bob").await;
    send_request(&app, &bob, "Alice").await;
    assert_eq!(
        set_presence(&app, &bob, json!({ "status": "online" })).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(friends(&app, &alice).await.as_array().unwrap().len(), 1);

    let (status, _) = block(&app, &alice, "Bob").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Plus d'amitié dans les deux sens, donc plus de présence visible.
    assert_eq!(friends(&app, &alice).await, json!([]));
    assert_eq!(friends(&app, &bob).await, json!([]));
    assert_eq!(blocks(&app, &alice).await, json!([{ "username": "Bob" }]));
    // Bob ne voit pas qu'il est bloqué dans ses propres listes.
    assert_eq!(blocks(&app, &bob).await, json!([]));
}

#[tokio::test]
async fn bloquer_retire_les_demandes_dans_les_deux_sens() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;
    send_request(&app, &bob, "Alice").await;
    assert_eq!(
        requests(&app, &alice).await["incoming"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    block(&app, &alice, "Bob").await;
    assert_eq!(
        requests(&app, &alice).await,
        json!({ "incoming": [], "outgoing": [] })
    );
    assert_eq!(
        requests(&app, &bob).await,
        json!({ "incoming": [], "outgoing": [] })
    );
}

#[tokio::test]
async fn plus_de_demande_tant_que_le_blocage_dure() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;
    block(&app, &alice, "Bob").await;

    // Celui qui a bloqué est prévenu.
    let (status, body) = send_request(&app, &alice, "Bob").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error_code(&body), "you_blocked");
    // Le joueur bloqué reçoit la même réponse qu'un pseudo inconnu.
    let (status, body) = send_request(&app, &bob, "Alice").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "user_not_found");
    let (_, unknown) = send_request(&app, &bob, "Personne").await;
    assert_eq!(body, unknown);
}

#[tokio::test]
async fn lever_le_blocage_ne_rend_pas_l_amitie() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;
    send_request(&app, &alice, "Bob").await;
    send_request(&app, &bob, "Alice").await;
    block(&app, &alice, "Bob").await;

    let (status, _) = call(&app, Method::DELETE, "/v1/blocks/bob", Some(&alice), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(blocks(&app, &alice).await, json!([]));
    assert_eq!(friends(&app, &alice).await, json!([]));
    // Une nouvelle demande redevient possible.
    let (status, _) = send_request(&app, &bob, "Alice").await;
    assert_eq!(status, StatusCode::CREATED);

    // Lever un blocage absent : erreur claire.
    let (status, body) = call(&app, Method::DELETE, "/v1/blocks/Bob", Some(&alice), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "block_not_found");
}

#[tokio::test]
async fn blocages_croises_et_erreurs() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;

    let (status, body) = block(&app, &alice, "Alice").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "self_block");
    let (status, body) = block(&app, &alice, "Personne").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "user_not_found");

    // Bloquer deux fois ne change rien ; les deux peuvent se bloquer.
    assert_eq!(block(&app, &alice, "Bob").await.0, StatusCode::NO_CONTENT);
    assert_eq!(block(&app, &alice, "Bob").await.0, StatusCode::NO_CONTENT);
    assert_eq!(block(&app, &bob, "Alice").await.0, StatusCode::NO_CONTENT);
    assert_eq!(blocks(&app, &bob).await, json!([{ "username": "Alice" }]));

    // Sans session : refusé.
    let (status, _) = call(&app, Method::GET, "/v1/blocks", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn blocages_ranges_par_pseudo() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    for name in ["zoe", "Bob", "marc"] {
        account(&app, name).await;
        block(&app, &alice, name).await;
    }
    assert_eq!(
        blocks(&app, &alice).await,
        json!([{ "username": "Bob" }, { "username": "marc" }, { "username": "zoe" }])
    );
}

/// Une base créée en version 1 (sans table `blocks`) est migrée à l'ouverture.
#[tokio::test]
async fn base_en_version_1_migree() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("hub.db").display());
    let store = hub::Store::open(&url).await.unwrap();
    drop(store);
    // On ramène la base à l'état de la version 1.
    let raw = sqlx::AnyPool::connect(&format!("sqlite:{}", dir.path().join("hub.db").display()))
        .await
        .unwrap();
    sqlx::query("DROP TABLE blocks")
        .execute(&raw)
        .await
        .unwrap();
    // La colonne `role` n'arrive qu'en version 5 (ADR 0036) : la migration la rajoute.
    sqlx::query("ALTER TABLE users DROP COLUMN role")
        .execute(&raw)
        .await
        .unwrap();
    sqlx::query("UPDATE schema_version SET version = 1")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;

    let store = hub::Store::open(&url).await.unwrap();
    let app = hub::router(
        hub::AppState::new(store, hub::HubConfig::for_tests())
            .await
            .unwrap(),
    );
    let alice = account(&app, "Alice").await;
    account(&app, "Bob").await;
    assert_eq!(block(&app, &alice, "Bob").await.0, StatusCode::NO_CONTENT);
}
