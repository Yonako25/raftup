//! Bout en bout : le **vrai client du launcher** (`launcher_core::hub_client`) contre le
//! **vrai routeur du hub**, avec le faux Discord [`FakeDiscord`] (aucun appel réseau
//! hors de `127.0.0.1`).
//!
//! Sens des dépendances : le hub (AGPL) dépend de `launcher-core` (MIT / Apache) en
//! test seulement ; l'inverse est interdit (ADR 0003, test `licence.rs`).
//!
//! Le test joue le navigateur : il suit à la main le renvoi du hub vers le petit
//! serveur local du launcher.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used)]

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use hub::{AppState, DisabledMailer, DiscordUser, FakeDiscord, HubConfig};
use launcher_core::hub_client::{DiscordOutcome, DiscordReturn, HubClient, HubUrl, DISCORD_WAIT};

mod common;

use common::*;

/// Lance le vrai routeur du hub sur un port libre de `127.0.0.1`.
async fn serve(fake: Arc<FakeDiscord>) -> (HubUrl, AppState) {
    let state = AppState::with_services(
        test_store().await,
        HubConfig::for_tests(),
        Arc::new(DisabledMailer),
        Some(fake),
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = hub::router(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let url = HubUrl::parse(&format!("http://127.0.0.1:{port}")).unwrap();
    (url, state)
}

/// Valeur d'un paramètre dans une adresse (`state=…`).
fn param(url: &str, name: &str) -> String {
    let query = url.split_once('?').unwrap().1;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .unwrap()
        .to_string()
}

/// Le « navigateur » : Discord a dit oui (code `code`), le hub reçoit le retour et
/// renvoie vers le serveur local du launcher, qu'on visite à notre tour.
fn browser(hub: &HubUrl, oauth_state: &str, code: &str) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into();
    let callback = format!(
        "{}/v1/oauth/discord/callback?state={oauth_state}&code={code}",
        hub.as_str()
    );
    let response = agent.get(&callback).call().unwrap();
    assert_eq!(response.status().as_u16(), 303);
    let location = response
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(location.starts_with("http://127.0.0.1:"), "{location}");
    let back = agent.get(&location).call().unwrap();
    assert_eq!(back.status().as_u16(), 200);
}

/// Un parcours Discord complet côté launcher, jusqu'à `finish`. Bloquant : appelé
/// dans `spawn_blocking`.
fn discord_login(
    hub: &HubUrl,
    fake: &FakeDiscord,
    user: DiscordUser,
) -> (
    HubClient,
    DiscordOutcome,
    launcher_core::hub_client::DiscordTicket,
) {
    let mut client = HubClient::new(hub.clone());
    let local = DiscordReturn::bind().unwrap();
    let start = client.discord_start(Some(local.port())).unwrap();
    let oauth_state = param(&start.authorize_url, "state");
    let code = format!("code-{}", user.id);
    fake.accept(&code, user);
    let hub_for_browser = hub.clone();
    let browser = std::thread::spawn(move || browser(&hub_for_browser, &oauth_state, &code));
    let ticket = local
        .wait(&start, DISCORD_WAIT, &AtomicBool::new(false))
        .unwrap();
    browser.join().unwrap();
    let outcome = client.discord_finish(&ticket).unwrap();
    (client, outcome, ticket)
}

#[tokio::test(flavor = "multi_thread")]
async fn launcher_se_connecte_par_discord() {
    let fake = Arc::new(FakeDiscord::new());
    let (url, _state) = serve(Arc::clone(&fake)).await;

    // `spawn_blocking` : le client du launcher est bloquant (ureq), il ne doit pas
    // tourner sur un thread de tokio qui sert aussi le hub.
    tokio::task::spawn_blocking(move || {
        let info = HubClient::new(url.clone()).hub_info().unwrap();
        assert!(info.offers_discord());
        assert_eq!(info.registration_mode, "open");

        // Nouveau joueur : pseudo proposé, puis compte créé.
        let user = DiscordUser {
            id: "42".to_string(),
            username: "mimi".to_string(),
            global_name: Some("Mimi".to_string()),
        };
        let (mut client, outcome, ticket) = discord_login(&url, &fake, user.clone());
        assert_eq!(
            outcome,
            DiscordOutcome::NeedsUsername {
                suggested_username: Some("Mimi".to_string()),
                invite_required: false,
            }
        );
        let session = client.discord_register(&ticket, "Mimi", None).unwrap();
        assert_eq!(session.username, "Mimi");
        assert_eq!(client.me().unwrap().username, "Mimi");

        // Même compte Discord, plus tard : connexion directe.
        let (client, outcome, _) = discord_login(&url, &fake, user);
        assert!(matches!(outcome, DiscordOutcome::LoggedIn(ref s) if s.username == "Mimi"));
        assert_eq!(client.me().unwrap().username, "Mimi");
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn launcher_discord_sur_invitation() {
    let fake = Arc::new(FakeDiscord::new());
    let (url, state) = serve(Arc::clone(&fake)).await;
    state
        .update_setting(None, "registration_mode", Some("invite"))
        .await
        .unwrap();
    tokio::task::spawn_blocking(move || {
        let info = HubClient::new(url.clone()).hub_info().unwrap();
        assert_eq!(info.registration_mode, "invite");
        let user = DiscordUser {
            id: "7".to_string(),
            username: "zoe".to_string(),
            global_name: None,
        };
        let (mut client, outcome, ticket) = discord_login(&url, &fake, user);
        assert!(matches!(
            outcome,
            DiscordOutcome::NeedsUsername {
                invite_required: true,
                ..
            }
        ));
        // Sans code : refusé, mais le ticket reste bon pour réessayer.
        let error = client.discord_register(&ticket, "Zoe", None).unwrap_err();
        assert_eq!(
            error.code(),
            Some(&launcher_core::hub_client::ErrorCode::InviteRequired)
        );
        assert!(!client.is_logged_in());
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn launcher_ne_voit_pas_discord_sur_un_hub_auto_heberge() {
    let state = AppState::with_services(
        test_store().await,
        HubConfig {
            discord_allowed: false,
            ..HubConfig::for_tests()
        },
        Arc::new(DisabledMailer),
        Some(Arc::new(FakeDiscord::new())),
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, hub::router(state)).await;
    });
    tokio::task::spawn_blocking(move || {
        let url = HubUrl::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let client = HubClient::new(url);
        assert!(!client.hub_info().unwrap().offers_discord());
        let error = client.discord_start(Some(1234)).unwrap_err();
        assert_eq!(
            error.code(),
            Some(&launcher_core::hub_client::ErrorCode::DiscordDisabled)
        );
    })
    .await
    .unwrap();
}
