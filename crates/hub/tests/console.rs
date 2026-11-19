//! Couche données de la console Rafthub (ADR 0036) : suspension, réglages, mode
//! d'inscription et invitations, journal de la console, liste des joueurs.
//! Sur SQLite, et sur PostgreSQL avec `RAFTUP_HUB_TEST_POSTGRES` (voir `common`).

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use axum::Router;
use hub::store::PlayerFilter;
use hub::{AppState, EnvSettings, HubConfig, HubError, MemoryMailer, SettingKey, Store};
use serde_json::{json, Value};

mod common;

use common::*;

/// Routeur et état partagé (pour appeler les fonctions de la console).
async fn console_app_with(config: HubConfig) -> (Router, AppState, Arc<MemoryMailer>) {
    let store = test_store().await;
    let mailer = Arc::new(MemoryMailer::new());
    let state = AppState::with_mailer(store, config, mailer.clone())
        .await
        .unwrap();
    (hub::router(state.clone()), state, mailer)
}

async fn console_app() -> (Router, AppState, Arc<MemoryMailer>) {
    console_app_with(HubConfig::for_tests()).await
}

/// Identifiant d'un compte d'après son pseudo.
async fn user_id(state: &AppState, username: &str) -> i64 {
    state
        .store()
        .user_by_username(&username.to_lowercase())
        .await
        .unwrap()
        .unwrap()
        .id
}

async fn register_with_invite(
    app: &Router,
    username: &str,
    code: Option<&str>,
) -> (StatusCode, Value) {
    let mut body = json!({
        "email": format!("{}@example.org", username.to_lowercase()),
        "username": username,
        "password": "mot de passe solide",
    });
    if let Some(code) = code {
        body["invite_code"] = json!(code);
    }
    call(app, Method::POST, "/v1/accounts", None, Some(body)).await
}

// ---------- Suspension ----------

#[tokio::test]
async fn compte_suspendu_ne_se_connecte_plus_et_disparait_chez_les_autres() {
    let (app, state, _) = console_app().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;
    let carol = account(&app, "Carol").await;
    // Alice et Bob amis ; Carol a demandé Bob, Bob a demandé... personne d'autre.
    send_request(&app, &alice, "Bob").await;
    call(
        &app,
        Method::POST,
        "/v1/friends/requests/Alice/accept",
        Some(&bob),
        None,
    )
    .await;
    send_request(&app, &carol, "Bob").await;
    set_presence(&app, &bob, json!({ "status": "online" })).await;
    assert_eq!(
        friends(&app, &alice).await[0]["presence"]["status"],
        "online"
    );

    let bob_id = user_id(&state, "Bob").await;
    state.suspend_user(None, bob_id).await.unwrap();
    assert!(state.store().is_suspended(bob_id).await.unwrap());

    // Sessions révoquées, connexion refusée (après le bon mot de passe seulement).
    let (status, _) = call(&app, Method::GET, "/v1/me", Some(&bob), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = login(&app, "bob@example.org", "mot de passe solide").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&body), "account_suspended");
    let (status, body) = login(&app, "bob@example.org", "mauvais mot de passe").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(&body), "invalid_credentials");

    // Caché des amis, des demandes et de la présence des autres.
    assert_eq!(friends(&app, &alice).await, json!([]));
    let (_, requests) = call(
        &app,
        Method::GET,
        "/v1/friends/requests",
        Some(&carol),
        None,
    )
    .await;
    assert_eq!(requests["outgoing"], json!([]));
    let (status, body) = send_request(&app, &carol, "Bob").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "user_not_found");

    // Suspendre deux fois ne change rien.
    state.suspend_user(None, bob_id).await.unwrap();

    // Réactivé : il se reconnecte, et l'amitié revient.
    state.reactivate_user(None, bob_id).await.unwrap();
    let (status, _) = login(&app, "bob@example.org", "mot de passe solide").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(friends(&app, &alice).await[0]["username"], "Bob");
    let (_, requests) = call(
        &app,
        Method::GET,
        "/v1/friends/requests",
        Some(&carol),
        None,
    )
    .await;
    assert_eq!(requests["outgoing"], json!([{ "username": "Bob" }]));
}

#[tokio::test]
async fn suspension_refusee_pour_soi_meme_ou_compte_inconnu() {
    let (app, state, _) = console_app().await;
    account(&app, "Alice").await;
    let alice_id = user_id(&state, "Alice").await;
    assert!(matches!(
        state.suspend_user(Some(alice_id), alice_id).await,
        Err(HubError::SelfSuspend)
    ));
    assert!(matches!(
        state.suspend_user(Some(alice_id), 9999).await,
        Err(HubError::UserNotFound)
    ));
    assert!(matches!(
        state.reactivate_user(Some(alice_id), 9999).await,
        Err(HubError::UserNotFound)
    ));
}

// ---------- Réglages ----------

#[tokio::test]
async fn reglages_base_puis_environnement_puis_defaut() {
    let mut config = HubConfig::for_tests();
    config.settings_env = EnvSettings::from_lookup(|name| match name {
        "RAFTUP_HUB_NAME" => Some("Hub de l'environnement".to_string()),
        "RAFTUP_HUB_SMTP_URL" => Some("smtps://moi:secret-smtp@smtp.exemple.org:465".to_string()),
        _ => None,
    });
    let (_, state, _) = console_app_with(config).await;

    let public = state.public_settings().await.unwrap();
    assert_eq!(public.hub_name, "Hub de l'environnement");
    assert_eq!(public.smtp_host.as_deref(), Some("smtp.exemple.org"));
    // Pas d'expéditeur : SMTP pas complet.
    assert!(!public.smtp_configured);

    state
        .update_setting(None, "hub_name", Some("  Le hub de la base "))
        .await
        .unwrap();
    state
        .update_setting(None, "discord_client_secret", Some("secret-discord"))
        .await
        .unwrap();
    let settings = state.settings().await.unwrap();
    assert_eq!(settings.hub_name, "Le hub de la base");
    assert_eq!(
        settings.discord_client_secret.as_deref(),
        Some("secret-discord")
    );

    // La vue publique ne rend jamais un secret.
    let json = serde_json::to_string(&state.public_settings().await.unwrap()).unwrap();
    assert!(!json.contains("secret-smtp"));
    assert!(!json.contains("secret-discord"));

    // Retirer la valeur de la base : retour à l'environnement.
    state.update_setting(None, "hub_name", None).await.unwrap();
    assert_eq!(
        state.settings().await.unwrap().hub_name,
        "Hub de l'environnement"
    );

    // Valeurs refusées.
    assert!(matches!(
        state.update_setting(None, "inconnu", Some("x")).await,
        Err(HubError::UnknownSetting)
    ));
    assert!(matches!(
        state
            .update_setting(None, "registration_mode", Some("ouvert"))
            .await,
        Err(HubError::InvalidSetting(_))
    ));

    // Le journal garde le nom du réglage, jamais la valeur.
    let events = state.events(50, 0).await.unwrap();
    assert!(events.items.iter().all(|e| e.kind == "setting_changed"));
    assert_eq!(events.total, 3);
    assert!(events
        .items
        .iter()
        .all(|e| !e.detail.contains("secret-discord") && !e.detail.contains("base")));
    assert_eq!(events.items[1].detail, "discord_client_secret");
    assert!(events.items[0].detail.contains("hub_name"));
}

#[tokio::test]
async fn adresse_publique_relue_pour_les_liens() {
    let (app, state, mailer) = console_app().await;
    state
        .update_setting(None, "public_url", Some("https://hub.exemple.org/"))
        .await
        .unwrap();
    let (status, _) = register(&app, "alice@example.org", "Alice", "mot de passe solide").await;
    assert_eq!(status, StatusCode::CREATED);
    let sent = mailer.wait_for(1).await;
    assert!(sent[0]
        .body
        .contains("https://hub.exemple.org/v1/email/verify?token="));
}

#[tokio::test]
async fn discord_relu_depuis_les_reglages() {
    let store = test_store().await;
    let state = AppState::from_settings(store, HubConfig::for_tests())
        .await
        .unwrap();
    let app = hub::router(state.clone());
    let start = || {
        call(
            &app,
            Method::POST,
            "/v1/oauth/discord/start",
            None,
            Some(json!({})),
        )
    };
    let (status, body) = start().await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(&body), "discord_disabled");

    // Réglé depuis la console : vaut tout de suite, sans redémarrage.
    state
        .update_setting(None, "discord_client_id", Some("424242"))
        .await
        .unwrap();
    state
        .update_setting(None, "discord_client_secret", Some("chut"))
        .await
        .unwrap();
    state
        .update_setting(None, "public_url", Some("https://hub.exemple.org"))
        .await
        .unwrap();
    let (status, body) = start().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let url = body["authorize_url"].as_str().unwrap();
    assert!(url.contains("client_id=424242"));
    assert!(url.contains("hub.exemple.org"));
    assert!(!url.contains("chut"));

    // SMTP non réglé : l'inscription marche quand même (aucun e-mail ne part).
    let (status, _) = register(&app, "alice@example.org", "Alice", "mot de passe solide").await;
    assert_eq!(status, StatusCode::CREATED);
}

// ---------- Mode d'inscription et invitations ----------

#[tokio::test]
async fn inscriptions_fermees() {
    let (app, state, _) = console_app().await;
    state
        .update_setting(None, SettingKey::RegistrationMode.name(), Some("closed"))
        .await
        .unwrap();
    let (status, body) = register_with_invite(&app, "Alice", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&body), "registration_closed");
    // Les comptes existants se connectent toujours.
    state
        .update_setting(None, "registration_mode", Some("open"))
        .await
        .unwrap();
    account(&app, "Bob").await;
    state
        .update_setting(None, "registration_mode", Some("closed"))
        .await
        .unwrap();
    let (status, _) = login(&app, "bob@example.org", "mot de passe solide").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn inscription_sur_invitation() {
    let (app, state, _) = console_app().await;
    state
        .update_setting(None, "registration_mode", Some("invite"))
        .await
        .unwrap();

    let (status, body) = register_with_invite(&app, "Alice", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&body), "invite_required");
    let (status, body) = register_with_invite(&app, "Alice", Some("FAUX-CODE")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&body), "invalid_invite");

    let invite = state
        .create_invite(None, Duration::from_secs(3600))
        .await
        .unwrap();
    // Le code se tape aussi en minuscules, sans tirets.
    let typed = invite.code.to_lowercase().replace('-', "");
    let (status, body) = register_with_invite(&app, "Alice", Some(&typed)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // Un code ne sert qu'une fois.
    let (status, body) = register_with_invite(&app, "Bob", Some(&invite.code)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&body), "invalid_invite");

    // Liste : utilisé par Alice ; le code en clair n'est pas gardé.
    let invites = state.invites(10, 0).await.unwrap();
    assert_eq!(invites.total, 1);
    assert_eq!(invites.items[0].used_by.as_deref(), Some("Alice"));
    assert!(invites.items[0].used_at.is_some());
    assert!(!serde_json::to_string(&invites)
        .unwrap()
        .contains(&invite.code));

    // Code expiré ; code révoqué.
    let expired = state.create_invite(None, Duration::ZERO).await.unwrap();
    let (_, body) = register_with_invite(&app, "Bob", Some(&expired.code)).await;
    assert_eq!(error_code(&body), "invalid_invite");
    let revoked = state
        .create_invite(None, Duration::from_secs(3600))
        .await
        .unwrap();
    state.revoke_invite(None, revoked.id).await.unwrap();
    let (_, body) = register_with_invite(&app, "Bob", Some(&revoked.code)).await;
    assert_eq!(error_code(&body), "invalid_invite");
    assert!(matches!(
        state.revoke_invite(None, revoked.id).await,
        Err(HubError::InviteNotFound)
    ));
}

// ---------- Journal de la console ----------

#[tokio::test]
async fn journal_des_actions_du_plus_recent_au_plus_ancien() {
    let (app, state, _) = console_app().await;
    let alice = account(&app, "Alice").await;
    account(&app, "Bob").await;
    let bob_id = user_id(&state, "Bob").await;
    let alice_id = user_id(&state, "Alice").await;
    state.suspend_user(Some(alice_id), bob_id).await.unwrap();
    state.reactivate_user(Some(alice_id), bob_id).await.unwrap();
    let (status, _) = call(&app, Method::GET, "/v1/me/export", Some(&alice), None).await;
    assert_eq!(status, StatusCode::OK);

    let page = state.events(100, 0).await.unwrap();
    let kinds: Vec<&str> = page.items.iter().map(|e| e.kind.as_str()).collect();
    // Même seconde possible : l'ordre suit alors l'identifiant, le plus récent d'abord.
    assert_eq!(
        kinds,
        [
            "gdpr_export",
            "account_reactivated",
            "account_suspended",
            "account_created",
            "account_created"
        ]
    );
    let suspended = &page.items[2];
    assert_eq!(suspended.actor_name.as_deref(), Some("Alice"));
    assert_eq!(suspended.target_name.as_deref(), Some("Bob"));
    // Jamais d'adresse e-mail dans le journal.
    let json = serde_json::to_string(&page).unwrap();
    assert!(!json.contains("example.org"));

    // Pagination.
    let first = state.events(2, 0).await.unwrap();
    let second = state.events(2, 2).await.unwrap();
    assert_eq!(first.total, 5);
    assert_eq!(first.items.len(), 2);
    assert_eq!(second.items[0].kind, "account_suspended");

    // Suppression RGPD : plus aucune ligne ne désigne le compte supprimé.
    let (status, _) = call(
        &app,
        Method::DELETE,
        "/v1/me",
        Some(&alice),
        Some(json!({ "password": "mot de passe solide" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let page = state.events(100, 0).await.unwrap();
    assert_eq!(page.items[0].kind, "gdpr_delete");
    assert!(page
        .items
        .iter()
        .all(|e| e.actor_id != Some(alice_id) && e.target_id != Some(alice_id)));
    assert!(!serde_json::to_string(&page).unwrap().contains("Alice"));
    assert_eq!(state.store().rows_of_user(alice_id).await.unwrap(), 0);
}

// ---------- Page « Joueurs » et tableau de bord ----------

#[tokio::test]
async fn liste_des_joueurs_recherche_filtres_et_compteurs() {
    let (app, state, mailer) = console_app().await;
    for name in ["Alice", "Bob", "Carol", "dave_100%"] {
        let (status, body) = register(
            &app,
            &format!("{}@example.org", name.to_lowercase().replace('%', "")),
            &name.replace('%', ""),
            "mot de passe solide",
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    // Alice vérifie son adresse ; Carol est suspendue.
    let sent = mailer.wait_for(4).await;
    let alice_mail = sent.iter().find(|m| m.to == "alice@example.org").unwrap();
    let token = alice_mail
        .body
        .split("token=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    let (status, _) = call(
        &app,
        Method::POST,
        "/v1/email/verify",
        None,
        Some(json!({ "token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let carol_id = user_id(&state, "Carol").await;
    state.suspend_user(None, carol_id).await.unwrap();
    login(&app, "bob@example.org", "mot de passe solide").await;

    let all = state.players("", PlayerFilter::All, 50, 0).await.unwrap();
    assert_eq!(all.total, 4);
    // Le plus récent d'abord.
    assert_eq!(all.items[0].username, "dave_100");
    assert_eq!(all.items[3].username, "Alice");
    assert!(all.items[3].email_verified);

    // Recherche par pseudo (sans tenir compte des majuscules) ou par e-mail.
    let found = state
        .players("ALI", PlayerFilter::All, 50, 0)
        .await
        .unwrap();
    assert_eq!(found.total, 1);
    assert_eq!(found.items[0].email.as_deref(), Some("alice@example.org"));
    let found = state
        .players("bob@example", PlayerFilter::All, 50, 0)
        .await
        .unwrap();
    assert_eq!(found.items.len(), 1);
    // `%` et `_` sont cherchés tels quels.
    assert_eq!(
        state
            .players("%", PlayerFilter::All, 50, 0)
            .await
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        state
            .players("e_1", PlayerFilter::All, 50, 0)
            .await
            .unwrap()
            .total,
        1
    );

    let unverified = state
        .players("", PlayerFilter::Unverified, 50, 0)
        .await
        .unwrap();
    assert_eq!(unverified.total, 3);
    let suspended = state
        .players("", PlayerFilter::Suspended, 50, 0)
        .await
        .unwrap();
    assert_eq!(suspended.total, 1);
    assert_eq!(suspended.items[0].username, "Carol");
    assert!(suspended.items[0].suspended_at.is_some());

    // Pagination : total inchangé, deux lignes.
    let page = state.players("", PlayerFilter::All, 2, 2).await.unwrap();
    assert_eq!((page.total, page.items.len()), (4, 2));
    assert_eq!(page.items[0].username, "Bob");

    let summary = state
        .store()
        .player_summary(carol_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.username, "Carol");
    assert!(state.store().player_summary(9999).await.unwrap().is_none());

    let bob = login(&app, "bob@example.org", "mot de passe solide")
        .await
        .1;
    set_presence(
        &app,
        bob["token"].as_str().unwrap(),
        json!({ "status": "online" }),
    )
    .await;
    let dashboard = state.dashboard().await.unwrap();
    assert_eq!(dashboard.counts.accounts, 4);
    assert_eq!(dashboard.counts.unverified, 3);
    assert_eq!(dashboard.counts.suspended, 1);
    assert_eq!(dashboard.counts.created_last_7_days, 4);
    assert_eq!(dashboard.counts.with_active_session, 1);
    assert_eq!(dashboard.online, 1);
}

// ---------- Migration ----------

/// Une base en version 4 (avant la console) est migrée : colonne `suspended_at`
/// ajoutée, nouvelles tables créées.
#[tokio::test]
async fn base_en_version_4_migree() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hub.db");
    let url = format!("sqlite://{}", path.display());
    drop(Store::open(&url).await.unwrap());
    let raw = sqlx::AnyPool::connect(&format!("sqlite:{}", path.display()))
        .await
        .unwrap();
    for statement in [
        "DROP TABLE admin_events",
        "DROP TABLE invites",
        "DROP TABLE settings",
        "ALTER TABLE users DROP COLUMN suspended_at",
        "UPDATE schema_version SET version = 4",
    ] {
        sqlx::query(statement).execute(&raw).await.unwrap();
    }
    raw.close().await;

    let store = Store::open(&url).await.unwrap();
    let state = AppState::new(store, HubConfig::for_tests()).await.unwrap();
    let app = hub::router(state.clone());
    account(&app, "Alice").await;
    let alice_id = user_id(&state, "Alice").await;
    state.suspend_user(None, alice_id).await.unwrap();
    let (status, _) = login(&app, "alice@example.org", "mot de passe solide").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
