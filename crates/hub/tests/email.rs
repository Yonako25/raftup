//! E-mails (ADR 0033) : vérification d'adresse et mot de passe oublié, avec le faux
//! service d'envoi [`MemoryMailer`] qui garde les messages en mémoire.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use axum::Router;
use hub::{AppState, HubConfig, MemoryMailer, Store};
use serde_json::json;
use tower::ServiceExt;

mod common;

use common::*;

const PASSWORD: &str = "mot de passe solide";

async fn app_with_mailer(config: HubConfig) -> (Router, Arc<MemoryMailer>, Store) {
    let store = test_store().await;
    let mailer = Arc::new(MemoryMailer::new());
    let state = AppState::with_mailer(store.clone(), config, mailer.clone())
        .await
        .unwrap();
    (hub::router(state), mailer, store)
}

/// Le jeton d'un message : le mot de 64 caractères hexadécimaux.
fn token_in(body: &str) -> String {
    body.split(|c: char| !c.is_ascii_hexdigit())
        .find(|word| word.len() == 64)
        .unwrap()
        .to_string()
}

async fn me(app: &Router, token: &str) -> serde_json::Value {
    let (status, body) = call(app, Method::GET, "/v1/me", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
    body
}

#[tokio::test]
async fn inscription_envoie_le_lien_de_verification() {
    let (app, mailer, _) = app_with_mailer(HubConfig::for_tests()).await;
    let alice = account(&app, "Alice").await;
    assert_eq!(me(&app, &alice).await["email_verified"], false);

    let sent = mailer.wait_for(1).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, "alice@example.org");
    assert!(sent[0].body.contains("Alice"));
    assert!(sent[0]
        .body
        .contains("http://127.0.0.1:8787/v1/email/verify?token="));

    let token = token_in(&sent[0].body);
    let (status, _) = call(
        &app,
        Method::POST,
        "/v1/email/verify",
        None,
        Some(json!({ "token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(me(&app, &alice).await["email_verified"], true);

    // Usage unique : le même jeton ne sert pas deux fois.
    let (status, body) = call(
        &app,
        Method::POST,
        "/v1/email/verify",
        None,
        Some(json!({ "token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "invalid_token");
}

#[tokio::test]
async fn lien_du_message_ouvert_dans_le_navigateur() {
    let (app, mailer, _) = app_with_mailer(HubConfig::for_tests()).await;
    let alice = account(&app, "Alice").await;
    let token = token_in(&mailer.wait_for(1).await[0].body);

    let request = axum::http::Request::get(format!("/v1/email/verify?token={token}"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&text).contains("Adresse vérifiée"));
    assert_eq!(me(&app, &alice).await["email_verified"], true);

    let request = axum::http::Request::get("/v1/email/verify?token=faux")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn lien_expire_refuse() {
    let config = HubConfig {
        email_verify_lifetime: Duration::ZERO,
        ..HubConfig::for_tests()
    };
    let (app, mailer, _) = app_with_mailer(config).await;
    account(&app, "Alice").await;
    let token = token_in(&mailer.wait_for(1).await[0].body);
    let (status, body) = call(
        &app,
        Method::POST,
        "/v1/email/verify",
        None,
        Some(json!({ "token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "invalid_token");
}

#[tokio::test]
async fn renvoi_du_message_de_verification() {
    let config = HubConfig {
        mails_per_window: 2,
        ..HubConfig::for_tests()
    };
    let (app, mailer, _) = app_with_mailer(config).await;
    let alice = account(&app, "Alice").await;
    let first = token_in(&mailer.wait_for(1).await[0].body);

    let (status, _) = call(
        &app,
        Method::POST,
        "/v1/email/verify/resend",
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let sent = mailer.wait_for(2).await;
    let second = token_in(&sent[1].body);
    assert_ne!(first, second);

    // Seul le dernier lien reste valable.
    let verify = |token: String| {
        let app = app.clone();
        async move {
            call(
                &app,
                Method::POST,
                "/v1/email/verify",
                None,
                Some(json!({ "token": token })),
            )
            .await
            .0
        }
    };
    assert_eq!(verify(first).await, StatusCode::BAD_REQUEST);

    // Limite d'envoi : 2 par fenêtre.
    call(
        &app,
        Method::POST,
        "/v1/email/verify/resend",
        Some(&alice),
        None,
    )
    .await;
    let (status, _) = call(
        &app,
        Method::POST,
        "/v1/email/verify/resend",
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    let last = token_in(&mailer.wait_for(3).await[2].body);
    assert_eq!(verify(last).await, StatusCode::NO_CONTENT);
    let (status, body) = call(
        &app,
        Method::POST,
        "/v1/email/verify/resend",
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error_code(&body), "already_verified");
}

async fn forgot(app: &Router, email: &str) -> StatusCode {
    call(
        app,
        Method::POST,
        "/v1/password/forgot",
        None,
        Some(json!({ "email": email })),
    )
    .await
    .0
}

async fn reset(app: &Router, token: &str, password: &str) -> (StatusCode, serde_json::Value) {
    call(
        app,
        Method::POST,
        "/v1/password/reset",
        None,
        Some(json!({ "token": token, "password": password })),
    )
    .await
}

#[tokio::test]
async fn mot_de_passe_oublie_bout_a_bout() {
    let (app, mailer, _) = app_with_mailer(HubConfig::for_tests()).await;
    let alice = account(&app, "Alice").await;
    mailer.wait_for(1).await;

    assert_eq!(
        forgot(&app, "Alice@Example.org").await,
        StatusCode::ACCEPTED
    );
    let sent = mailer.wait_for(2).await;
    assert_eq!(sent[1].to, "alice@example.org");
    assert!(sent[1].body.contains("raftup://password-reset?token="));
    let token = token_in(&sent[1].body);

    // Mot de passe trop court : refusé, et le code reste valable.
    let (status, body) = reset(&app, &token, "court").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "weak_password");

    let (status, _) = reset(&app, &token, "nouveau mot de passe").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Toutes les sessions sont fermées, l'ancien mot de passe ne marche plus.
    let (status, _) = call(&app, Method::GET, "/v1/me", Some(&alice), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        login(&app, "alice@example.org", PASSWORD).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, session) = login(&app, "alice@example.org", "nouveau mot de passe").await;
    assert_eq!(status, StatusCode::OK);
    // Lire le message prouve qu'on reçoit l'adresse : elle devient vérifiée.
    let token_session = session["token"].as_str().unwrap();
    assert_eq!(me(&app, token_session).await["email_verified"], true);

    // Usage unique.
    let (status, body) = reset(&app, &token, "encore un autre").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "invalid_token");
}

#[tokio::test]
async fn adresse_inconnue_meme_reponse_et_rien_n_est_envoye() {
    let (app, mailer, _) = app_with_mailer(HubConfig::for_tests()).await;
    assert_eq!(
        forgot(&app, "personne@example.org").await,
        StatusCode::ACCEPTED
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(mailer.sent().is_empty());
}

#[tokio::test]
async fn jeton_de_verification_ne_change_pas_le_mot_de_passe() {
    let (app, mailer, _) = app_with_mailer(HubConfig::for_tests()).await;
    account(&app, "Alice").await;
    let verify_token = token_in(&mailer.wait_for(1).await[0].body);
    let (status, body) = reset(&app, &verify_token, "nouveau mot de passe").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "invalid_token");
}

#[tokio::test]
async fn code_de_reinitialisation_expire() {
    let config = HubConfig {
        password_reset_lifetime: Duration::ZERO,
        ..HubConfig::for_tests()
    };
    let (app, mailer, store) = app_with_mailer(config).await;
    account(&app, "Alice").await;
    forgot(&app, "alice@example.org").await;
    let token = token_in(&mailer.wait_for(2).await[1].body);
    assert_eq!(
        reset(&app, &token, "nouveau mot de passe").await.0,
        StatusCode::BAD_REQUEST
    );
    // Le ménage retire les jetons expirés (vérification et réinitialisation).
    let removed = store
        .purge_expired_email_tokens(hub::now_unix() + 48 * 3600)
        .await
        .unwrap();
    assert_eq!(removed, 2);
}

#[tokio::test]
async fn limite_d_envoi_silencieuse() {
    let config = HubConfig {
        mails_per_window: 1,
        ..HubConfig::for_tests()
    };
    let (app, mailer, _) = app_with_mailer(config).await;
    account(&app, "Alice").await;
    mailer.wait_for(1).await;
    assert_eq!(
        forgot(&app, "alice@example.org").await,
        StatusCode::ACCEPTED
    );
    // Au-delà de la limite : même réponse, mais plus de message.
    assert_eq!(
        forgot(&app, "alice@example.org").await,
        StatusCode::ACCEPTED
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(mailer.sent().len(), 2);
}

#[tokio::test]
async fn le_jeton_n_est_jamais_stocke_en_clair() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hub.db");
    let store = Store::open(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    let mailer = Arc::new(MemoryMailer::new());
    let state = AppState::with_mailer(store, HubConfig::for_tests(), mailer.clone())
        .await
        .unwrap();
    let app = hub::router(state);
    account(&app, "Alice").await;
    let token = token_in(&mailer.wait_for(1).await[0].body);

    // On lit la base et son WAL octet par octet : l'empreinte y est, le jeton non.
    let mut bytes = std::fs::read(&path).unwrap();
    if let Ok(wal) = std::fs::read(dir.path().join("hub.db-wal")) {
        bytes.extend(wal);
    }
    let contains = |needle: &str| bytes.windows(needle.len()).any(|w| w == needle.as_bytes());
    assert!(contains(&hub::session::token_hash(&token)));
    assert!(!contains(&token));
}
