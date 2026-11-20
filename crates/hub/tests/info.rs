//! `GET /v1/info` : ce que le hub propose au launcher (moyens de connexion,
//! mode d'inscription). Discord n'y figure que si la version le permet
//! (ADR 0038) et que Discord est réglé.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use hub::{AppState, DisabledMailer, FakeDiscord, HubConfig};
use serde_json::json;

mod common;

use common::*;

/// Hub de test : `discord_allowed` et faux Discord au choix.
async fn info_state(discord_allowed: bool, with_discord: bool) -> AppState {
    let discord: Option<Arc<dyn hub::DiscordApi>> = if with_discord {
        Some(Arc::new(FakeDiscord::new()))
    } else {
        None
    };
    AppState::with_services(
        test_store().await,
        HubConfig {
            discord_allowed,
            ..HubConfig::for_tests()
        },
        Arc::new(DisabledMailer),
        discord,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn info_propose_discord_dans_la_version_officielle() {
    let app = hub::router(info_state(true, true).await);
    let (status, body) = call(&app, Method::GET, "/v1/info", None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["service"], "raftup-hub");
    assert_eq!(body["api_version"], 1);
    assert_eq!(body["login_methods"], json!(["password", "discord"]));
    assert_eq!(body["registration_mode"], "open");
    assert!(body["name"].is_string());
    assert!(body["version"].is_string());
}

#[tokio::test]
async fn info_sans_discord_hors_version_officielle() {
    // Discord réglé, mais hub auto-hébergé : pas proposé.
    let app = hub::router(info_state(false, true).await);
    let (_, body) = call(&app, Method::GET, "/v1/info", None, None).await;
    assert_eq!(body["login_methods"], json!(["password"]));
}

#[tokio::test]
async fn info_sans_discord_non_regle() {
    // Version officielle, mais application Discord pas réglée.
    let app = hub::router(info_state(true, false).await);
    let (_, body) = call(&app, Method::GET, "/v1/info", None, None).await;
    assert_eq!(body["login_methods"], json!(["password"]));
}

#[tokio::test]
async fn info_suit_le_mode_d_inscription() {
    let state = info_state(true, true).await;
    let app = hub::router(state.clone());
    state
        .update_setting(None, "registration_mode", Some("invite"))
        .await
        .unwrap();
    let (_, body) = call(&app, Method::GET, "/v1/info", None, None).await;
    assert_eq!(body["registration_mode"], "invite");
}
