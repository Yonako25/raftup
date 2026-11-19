//! RGPD (ADR 0002, 0033) : export des données d'un compte et suppression du compte.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used)]

use axum::http::{Method, StatusCode};
use hub::{AppState, HubConfig};
use serde_json::{json, Value};

mod common;

use common::*;

const PASSWORD: &str = "mot de passe solide";

async fn export(app: &axum::Router, token: &str) -> Value {
    let (status, body) = call(app, Method::GET, "/v1/me/export", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
    body
}

async fn delete_me(app: &axum::Router, token: &str, body: Value) -> (StatusCode, Value) {
    call(app, Method::DELETE, "/v1/me", Some(token), Some(body)).await
}

#[tokio::test]
async fn export_contient_tout_le_compte_sans_secret() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;
    account(&app, "Chloe").await;
    account(&app, "Dan").await;
    send_request(&app, &alice, "Bob").await;
    send_request(&app, &bob, "Alice").await;
    send_request(&app, &alice, "Chloe").await;
    call(
        &app,
        Method::POST,
        "/v1/blocks",
        Some(&alice),
        Some(json!({ "username": "Dan" })),
    )
    .await;
    set_presence(
        &app,
        &alice,
        json!({ "status": "in_game", "game": "APQuest" }),
    )
    .await;

    let body = export(&app, &alice).await;
    assert_eq!(body["format_version"], 1);
    assert_eq!(body["service"], "raftup-hub");
    let account = &body["account"];
    assert_eq!(account["username"], "Alice");
    assert_eq!(account["identities"][0]["provider"], "email");
    assert_eq!(account["identities"][0]["subject"], "alice@example.org");
    assert_eq!(account["identities"][0]["verified"], false);
    assert_eq!(account["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(account["friends"][0]["username"], "Bob");
    assert_eq!(account["requests_sent"][0]["username"], "Chloe");
    assert_eq!(account["requests_received"], json!([]));
    assert_eq!(account["blocked"][0]["username"], "Dan");
    assert_eq!(
        body["presence"],
        json!({ "status": "in_game", "game": "APQuest" })
    );

    // Ni hash de mot de passe, ni jeton, ni empreinte de session.
    let text = body.to_string();
    assert!(!text.contains("argon2"), "{text}");
    assert!(!text.contains(&alice), "{text}");
    assert!(
        !text.contains("secret_hash") && !text.contains("token"),
        "{text}"
    );
}

#[tokio::test]
async fn export_demande_une_session() {
    let app = app().await;
    let (status, _) = call(&app, Method::GET, "/v1/me/export", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn suppression_efface_tout_et_libere_pseudo_et_adresse() {
    let store = test_store().await;
    let app = hub::router(
        AppState::new(store.clone(), HubConfig::for_tests())
            .await
            .unwrap(),
    );
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;
    let chloe = account(&app, "Chloe").await;
    send_request(&app, &alice, "Bob").await;
    send_request(&app, &bob, "Alice").await;
    send_request(&app, &chloe, "Alice").await;
    call(
        &app,
        Method::POST,
        "/v1/blocks",
        Some(&chloe),
        Some(json!({ "username": "Alice" })),
    )
    .await;
    let alice_id = export(&app, &alice).await["account"]["user_id"]
        .as_i64()
        .unwrap();
    assert!(store.rows_of_user(alice_id).await.unwrap() > 0);

    let (status, _) = delete_me(&app, &alice, json!({ "password": PASSWORD })).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Plus aucune ligne, dans aucune table.
    assert_eq!(store.rows_of_user(alice_id).await.unwrap(), 0);
    // La session est morte, Alice a disparu des listes de Bob et de Chloé.
    let (status, _) = call(&app, Method::GET, "/v1/me", Some(&alice), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(friends(&app, &bob).await, json!([]));
    let (_, blocks) = call(&app, Method::GET, "/v1/blocks", Some(&chloe), None).await;
    assert_eq!(blocks, json!([]));
    // Connexion impossible ; pseudo et adresse de nouveau libres.
    let (status, _) = login(&app, "alice@example.org", PASSWORD).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = register(&app, "alice@example.org", "Alice", PASSWORD).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn suppression_demande_le_mot_de_passe() {
    let app = app().await;
    let alice = account(&app, "Alice").await;

    let (status, body) = delete_me(&app, &alice, json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(&body), "invalid_credentials");
    let (status, body) = delete_me(&app, &alice, json!({ "password": "pas le bon" })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(&body), "invalid_credentials");
    // Le compte est toujours là.
    let (status, _) = call(&app, Method::GET, "/v1/me", Some(&alice), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn suppression_limitee_en_cas_d_echecs() {
    let config = HubConfig {
        login_max_failures: 2,
        ..HubConfig::for_tests()
    };
    let app = app_with(config).await;
    let alice = account(&app, "Alice").await;
    for _ in 0..2 {
        delete_me(&app, &alice, json!({ "password": "pas le bon" })).await;
    }
    // Même le bon mot de passe est refusé tant que la limite dure.
    let (status, body) = delete_me(&app, &alice, json!({ "password": PASSWORD })).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error_code(&body), "too_many_attempts");
}
