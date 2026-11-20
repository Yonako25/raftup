//! Connexion Discord (OAuth 2.0 + PKCE, ADR 0033), avec le faux Discord
//! [`FakeDiscord`] : aucun appel réseau.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use hub::{AppState, DisabledMailer, DiscordUser, FakeDiscord, HubConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

mod common;

use common::*;

async fn discord_app() -> (Router, Arc<FakeDiscord>) {
    let store = test_store().await;
    let fake = Arc::new(FakeDiscord::new());
    let state = AppState::with_services(
        store,
        HubConfig::for_tests(),
        Arc::new(DisabledMailer),
        Some(fake.clone()),
    )
    .await
    .unwrap();
    (hub::router(state), fake)
}

fn discord_user(id: &str, username: &str) -> DiscordUser {
    DiscordUser {
        id: id.to_string(),
        username: username.to_string(),
        global_name: None,
    }
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

struct Started {
    state: String,
    secret: String,
    authorize_url: String,
}

async fn start(app: &Router, token: Option<&str>, body: Value) -> Started {
    let (status, view) = call(
        app,
        Method::POST,
        "/v1/oauth/discord/start",
        token,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    Started {
        state: view["state"].as_str().unwrap().to_string(),
        secret: view["secret"].as_str().unwrap().to_string(),
        authorize_url: view["authorize_url"].as_str().unwrap().to_string(),
    }
}

/// Le navigateur revient de Discord ; renvoie le code HTTP et l'adresse de renvoi.
async fn callback(app: &Router, query: &str) -> (StatusCode, Option<String>) {
    let request = Request::get(format!("/v1/oauth/discord/callback?{query}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let location = response
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_string());
    (response.status(), location)
}

/// Aller-retour complet chez Discord avec un code accepté ; renvoie le ticket.
async fn authorize(
    app: &Router,
    fake: &FakeDiscord,
    started: &Started,
    user: DiscordUser,
) -> String {
    let code = format!("code-{}", user.id);
    fake.accept(&code, user);
    let (status, location) = callback(app, &format!("state={}&code={code}", started.state)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    param(&location.unwrap(), "ticket")
}

async fn finish(app: &Router, started: &Started, ticket: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/v1/oauth/discord/finish",
        None,
        Some(json!({ "state": started.state, "secret": started.secret, "ticket": ticket })),
    )
    .await
}

async fn register_discord(
    app: &Router,
    started: &Started,
    ticket: &str,
    username: &str,
) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/v1/oauth/discord/register",
        None,
        Some(json!({
            "state": started.state, "secret": started.secret,
            "ticket": ticket, "username": username
        })),
    )
    .await
}

#[tokio::test]
async fn nouveau_joueur_choisit_son_pseudo_puis_se_reconnecte() {
    let (app, fake) = discord_app().await;
    let started = start(&app, None, json!({ "return_port": 53682 })).await;
    assert!(started
        .authorize_url
        .starts_with("https://discord.com/oauth2/authorize?"));
    assert_eq!(param(&started.authorize_url, "state"), started.state);

    let ticket = authorize(&app, &fake, &started, discord_user("111", "zelda_fan")).await;
    // PKCE : le défi envoyé à Discord correspond au vérificateur gardé par le hub.
    let verifier = fake.last_verifier().unwrap();
    assert_eq!(
        param(&started.authorize_url, "code_challenge"),
        hub::discord::code_challenge(&verifier)
    );
    assert!(!started.authorize_url.contains(&verifier));

    let (status, body) = finish(&app, &started, &ticket).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "needs_username");
    assert_eq!(body["suggested_username"], "zelda_fan");

    let (status, body) = register_discord(&app, &started, &ticket, "Link").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["status"], "logged_in");
    assert_eq!(body["username"], "Link");
    let token = body["token"].as_str().unwrap().to_string();
    let (_, me) = call(&app, Method::GET, "/v1/me", Some(&token), None).await;
    assert_eq!(me["discord_linked"], true);
    assert_eq!(me["email"], Value::Null);

    // Connexion suivante : directement une session.
    let again = start(&app, None, json!({})).await;
    let ticket = authorize(&app, &fake, &again, discord_user("111", "zelda_fan")).await;
    let (status, body) = finish(&app, &again, &ticket).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "logged_in");
    assert_eq!(body["username"], "Link");
}

#[tokio::test]
async fn retour_vers_le_port_local_ou_le_lien_profond() {
    let (app, fake) = discord_app().await;
    let started = start(&app, None, json!({ "return_port": 53682 })).await;
    fake.accept("c", discord_user("1", "a"));
    let (_, location) = callback(&app, &format!("state={}&code=c", started.state)).await;
    assert!(location
        .unwrap()
        .starts_with("http://127.0.0.1:53682/discord?state="));

    let started = start(&app, None, json!({})).await;
    fake.accept("d", discord_user("2", "b"));
    let (_, location) = callback(&app, &format!("state={}&code=d", started.state)).await;
    assert!(location
        .unwrap()
        .starts_with("raftup://oauth/discord?state="));
}

#[tokio::test]
async fn secret_et_ticket_obligatoires() {
    let (app, fake) = discord_app().await;
    let started = start(&app, None, json!({})).await;
    // Avant le retour de Discord : rien à finir.
    let (status, _) = finish(&app, &started, "pas-de-ticket").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let ticket = authorize(&app, &fake, &started, discord_user("111", "zelda")).await;
    // Quelqu'un qui connaît le lien (donc `state`) mais pas le secret : refusé.
    let thief = Started {
        state: started.state.clone(),
        secret: "0".repeat(64),
        authorize_url: String::new(),
    };
    let (status, body) = finish(&app, &thief, &ticket).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "invalid_token");
    // Le secret sans le ticket (arrivé dans le navigateur) : refusé aussi.
    let (status, _) = finish(&app, &started, &"1".repeat(64)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Les deux : accepté.
    let (status, _) = finish(&app, &started, &ticket).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn retour_de_discord_une_seule_fois() {
    let (app, fake) = discord_app().await;
    let started = start(&app, None, json!({})).await;
    authorize(&app, &fake, &started, discord_user("1", "a")).await;
    fake.accept("autre", discord_user("2", "b"));
    let (status, _) = callback(&app, &format!("state={}&code=autre", started.state)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = callback(&app, "state=inconnu&code=x").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn refus_chez_discord() {
    let (app, _) = discord_app().await;
    let started = start(&app, None, json!({})).await;
    let (status, location) = callback(
        &app,
        &format!("state={}&error=access_denied", started.state),
    )
    .await;
    // Le navigateur revient quand même au launcher, qui apprend le refus.
    assert_eq!(status, StatusCode::SEE_OTHER);
    let ticket = param(&location.unwrap(), "ticket");
    let (status, body) = finish(&app, &started, &ticket).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "discord_refused");

    // Code inconnu de Discord : même chose.
    let started = start(&app, None, json!({})).await;
    let (_, location) = callback(&app, &format!("state={}&code=faux", started.state)).await;
    let ticket = param(&location.unwrap(), "ticket");
    let (_, body) = finish(&app, &started, &ticket).await;
    assert_eq!(error_code(&body), "discord_refused");
}

#[tokio::test]
async fn lier_puis_delier_discord() {
    let (app, fake) = discord_app().await;
    let alice = account(&app, "Alice").await;

    let started = start(&app, Some(&alice), json!({})).await;
    let ticket = authorize(&app, &fake, &started, discord_user("222", "alice_d")).await;
    let (status, body) = finish(&app, &started, &ticket).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "linked");
    let (_, me) = call(&app, Method::GET, "/v1/me", Some(&alice), None).await;
    assert_eq!(me["discord_linked"], true);

    // Connexion par Discord : le compte d'Alice.
    let started = start(&app, None, json!({})).await;
    let ticket = authorize(&app, &fake, &started, discord_user("222", "alice_d")).await;
    let (_, body) = finish(&app, &started, &ticket).await;
    assert_eq!(body["username"], "Alice");

    // Un second compte Discord sur le même compte : refusé.
    let started = start(&app, Some(&alice), json!({})).await;
    let ticket = authorize(&app, &fake, &started, discord_user("333", "autre")).await;
    let (status, body) = finish(&app, &started, &ticket).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error_code(&body), "already_linked");

    // L'export RGPD montre l'identité Discord.
    let (_, export) = call(&app, Method::GET, "/v1/me/export", Some(&alice), None).await;
    assert_eq!(export["account"]["identities"][1]["provider"], "discord");
    assert_eq!(export["account"]["identities"][1]["subject"], "222");

    let (status, _) = call(
        &app,
        Method::DELETE,
        "/v1/me/identities/discord",
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = call(
        &app,
        Method::DELETE,
        "/v1/me/identities/discord",
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "identity_not_found");
}

#[tokio::test]
async fn discord_deja_pris_et_dernier_moyen_de_connexion() {
    let (app, fake) = discord_app().await;
    // Bob se crée un compte par Discord seulement.
    let started = start(&app, None, json!({})).await;
    let ticket = authorize(&app, &fake, &started, discord_user("444", "bob")).await;
    finish(&app, &started, &ticket).await;
    let (_, body) = register_discord(&app, &started, &ticket, "Bob").await;
    let bob = body["token"].as_str().unwrap().to_string();

    // Alice ne peut pas lier le Discord de Bob.
    let alice = account(&app, "Alice").await;
    let started = start(&app, Some(&alice), json!({})).await;
    let ticket = authorize(&app, &fake, &started, discord_user("444", "bob")).await;
    let (status, body) = finish(&app, &started, &ticket).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error_code(&body), "discord_taken");

    // Bob ne peut pas délier son seul moyen de connexion…
    let (status, body) = call(
        &app,
        Method::DELETE,
        "/v1/me/identities/discord",
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error_code(&body), "last_identity");
    // … et supprime son compte sans mot de passe (il n'en a pas).
    let (status, _) = call(&app, Method::DELETE, "/v1/me", Some(&bob), Some(json!({}))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn pseudo_propose_seulement_s_il_est_libre_et_valide() {
    let (app, fake) = discord_app().await;
    account(&app, "Pris").await;
    let started = start(&app, None, json!({})).await;
    let user = DiscordUser {
        id: "555".to_string(),
        username: "pris".to_string(),
        global_name: Some("Nom affiché avec espaces".to_string()),
    };
    let ticket = authorize(&app, &fake, &started, user).await;
    let (_, body) = finish(&app, &started, &ticket).await;
    assert_eq!(body["status"], "needs_username");
    assert!(body.get("suggested_username").is_none(), "{body}");

    // Pseudo pris : erreur, mais la connexion reste ouverte pour réessayer.
    let (status, body) = register_discord(&app, &started, &ticket, "Pris").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error_code(&body), "username_taken");
    let (status, _) = register_discord(&app, &started, &ticket, "Libre").await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn sans_application_discord_le_hub_refuse_proprement() {
    let app = app().await;
    let (status, body) = call(
        &app,
        Method::POST,
        "/v1/oauth/discord/start",
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(&body), "discord_disabled");
}

#[tokio::test]
async fn jeton_faux_au_depart_refuse() {
    let (app, _) = discord_app().await;
    let (status, _) = call(
        &app,
        Method::POST,
        "/v1/oauth/discord/start",
        Some("faux"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------- Console Rafthub (ADR 0036) ----------

/// Comme [`discord_app`], avec l'état du hub pour appeler la console.
async fn discord_app_with_state() -> (Router, Arc<FakeDiscord>, AppState) {
    let store = test_store().await;
    let fake = Arc::new(FakeDiscord::new());
    let state = AppState::with_services(
        store,
        HubConfig::for_tests(),
        Arc::new(DisabledMailer),
        Some(fake.clone()),
    )
    .await
    .unwrap();
    (hub::router(state.clone()), fake, state)
}

#[tokio::test]
async fn compte_suspendu_refuse_par_discord() {
    let (app, fake, state) = discord_app_with_state().await;
    let started = start(&app, None, json!({})).await;
    let ticket = authorize(&app, &fake, &started, discord_user("111", "zelda")).await;
    finish(&app, &started, &ticket).await;
    let (status, body) = register_discord(&app, &started, &ticket, "Link").await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = body["user_id"].as_i64().unwrap();

    state.suspend_user(None, user_id).await.unwrap();
    let again = start(&app, None, json!({})).await;
    let ticket = authorize(&app, &fake, &again, discord_user("111", "zelda")).await;
    let (status, body) = finish(&app, &again, &ticket).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&body), "account_suspended");
}

#[tokio::test]
async fn inscription_discord_selon_le_mode() {
    let (app, fake, state) = discord_app_with_state().await;

    // Fermé : la connexion d'un nouveau joueur s'arrête au retour de Discord.
    state
        .update_setting(None, "registration_mode", Some("closed"))
        .await
        .unwrap();
    let started = start(&app, None, json!({})).await;
    let ticket = authorize(&app, &fake, &started, discord_user("1", "un")).await;
    let (status, body) = finish(&app, &started, &ticket).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&body), "registration_closed");

    // Sur invitation : pseudo et code demandés.
    state
        .update_setting(None, "registration_mode", Some("invite"))
        .await
        .unwrap();
    let started = start(&app, None, json!({})).await;
    let ticket = authorize(&app, &fake, &started, discord_user("2", "deux")).await;
    let (status, body) = finish(&app, &started, &ticket).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "needs_username");
    assert_eq!(body["invite_required"], true);
    let (status, body) = register_discord(&app, &started, &ticket, "Deux").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&body), "invite_required");

    // La connexion reste ouverte : on réessaie avec un bon code.
    let invite = state
        .create_invite(None, std::time::Duration::from_secs(600))
        .await
        .unwrap();
    let (status, body) = call(
        &app,
        Method::POST,
        "/v1/oauth/discord/register",
        None,
        Some(json!({
            "state": started.state, "secret": started.secret, "ticket": ticket,
            "username": "Deux", "invite_code": invite.code
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let events = state.events(10, 0).await.unwrap();
    assert_eq!(events.items[0].kind, "account_created");
    assert_eq!(events.items[0].detail, "Discord, avec invitation");

    // Mode ouvert : pas de champ `invite_required` dans la réponse.
    state
        .update_setting(None, "registration_mode", Some("open"))
        .await
        .unwrap();
    let started = start(&app, None, json!({})).await;
    let ticket = authorize(&app, &fake, &started, discord_user("3", "trois")).await;
    let (_, body) = finish(&app, &started, &ticket).await;
    assert!(body.get("invite_required").is_none());
}

/// Rafthub auto-hébergé (sans l'option `officiel`) : Discord reste éteint, même
/// avec un client Discord prêt (ADR 0038).
#[tokio::test]
async fn hub_auto_heberge_sans_discord() {
    let store = test_store().await;
    let fake = Arc::new(FakeDiscord::new());
    let state = AppState::with_services(
        store,
        HubConfig {
            discord_allowed: false,
            ..HubConfig::for_tests()
        },
        Arc::new(DisabledMailer),
        Some(fake),
    )
    .await
    .unwrap();
    let app = hub::router(state);
    let (status, body) = call(
        &app,
        Method::POST,
        "/v1/oauth/discord/start",
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(&body), "discord_disabled");
}
