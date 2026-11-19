//! Tests d'intégration de l'API du hub : requêtes HTTP envoyées au routeur,
//! sans ouvrir de port (`tower::ServiceExt::oneshot`), base SQLite en mémoire.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use hub::{AppState, HubConfig, Store};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn app_with(config: HubConfig) -> Router {
    let store = Store::in_memory().await.unwrap();
    let state = AppState::new(store, config).await.unwrap();
    hub::router(state)
}

async fn app() -> Router {
    app_with(HubConfig::for_tests()).await
}

/// Envoie une requête et renvoie le code HTTP et le corps JSON (`Null` si vide).
async fn call(
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

async fn register(
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

async fn login(app: &Router, email: &str, password: &str) -> (StatusCode, Value) {
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
async fn account(app: &Router, username: &str) -> String {
    let email = format!("{}@example.org", username.to_lowercase());
    let (status, _) = register(app, &email, username, "mot de passe solide").await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = login(app, &email, "mot de passe solide").await;
    assert_eq!(status, StatusCode::OK);
    body["token"].as_str().unwrap().to_string()
}

fn error_code(body: &Value) -> &str {
    body["error"].as_str().unwrap_or("")
}

#[tokio::test]
async fn version_de_l_api() {
    let app = app().await;
    let (status, body) = call(&app, Method::GET, "/v1", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["api_version"], 1);
}

#[tokio::test]
async fn inscription_connexion_et_compte() {
    let app = app().await;
    let (status, body) = register(&app, "Mimi@Example.org", "Mimi", "mot de passe solide").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["username"], "Mimi");

    // L'adresse est comparée sans tenir compte de la casse.
    let (status, body) = login(&app, "mimi@example.ORG", "mot de passe solide").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["username"], "Mimi");
    let token = body["token"].as_str().unwrap();
    assert_eq!(token.len(), 64);

    let (status, body) = call(&app, Method::GET, "/v1/me", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["username"], "Mimi");
    assert_eq!(body["email"], "mimi@example.org");
}

#[tokio::test]
async fn mauvais_mot_de_passe_et_adresse_inconnue_meme_reponse() {
    let app = app().await;
    register(&app, "bob@example.org", "Bob", "mot de passe solide").await;
    let (status_a, body_a) = login(&app, "bob@example.org", "mauvais mot de passe").await;
    let (status_b, body_b) = login(&app, "personne@example.org", "mot de passe solide").await;
    assert_eq!(status_a, StatusCode::UNAUTHORIZED);
    assert_eq!(body_a, body_b);
    assert_eq!(status_a, status_b);
    assert_eq!(error_code(&body_a), "invalid_credentials");
}

#[tokio::test]
async fn pseudo_et_adresse_uniques() {
    let app = app().await;
    let (status, _) = register(&app, "a@example.org", "Mimi", "mot de passe solide").await;
    assert_eq!(status, StatusCode::CREATED);

    // Même pseudo à la casse près, ou en lettres pleine chasse.
    for username in ["mimi", "MIMI", "Ｍｉｍｉ"] {
        let (status, body) = register(&app, "b@example.org", username, "mot de passe solide").await;
        assert_eq!(status, StatusCode::CONFLICT, "{username}");
        assert_eq!(error_code(&body), "username_taken");
    }

    let (status, body) = register(&app, "A@EXAMPLE.org", "Autre", "mot de passe solide").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error_code(&body), "email_taken");
}

#[tokio::test]
async fn entrees_invalides_refusees() {
    let app = app().await;
    for username in ["ab", "a b", "admin", "Raftup", "x".repeat(25).as_str()] {
        let (status, body) = register(&app, "c@example.org", username, "mot de passe solide").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{username}");
        assert_eq!(error_code(&body), "invalid_username");
    }
    let (status, body) = register(&app, "pas-une-adresse", "Carla", "mot de passe solide").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "invalid_email");
    let (status, body) = register(&app, "c@example.org", "Carla", "court").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "weak_password");
}

#[tokio::test]
async fn jeton_absent_faux_ou_revoque() {
    let app = app().await;
    let (status, _) = call(&app, Method::GET, "/v1/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&app, Method::GET, "/v1/me", Some("faux"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let token = account(&app, "Dora").await;
    let (status, _) = call(
        &app,
        Method::DELETE,
        "/v1/sessions/current",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = call(&app, Method::GET, "/v1/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(&body), "unauthorized");
}

#[tokio::test]
async fn session_expiree_refusee() {
    let config = HubConfig {
        session_lifetime: Duration::ZERO,
        ..HubConfig::for_tests()
    };
    let app = app_with(config).await;
    register(&app, "e@example.org", "Emma", "mot de passe solide").await;
    let (_, body) = login(&app, "e@example.org", "mot de passe solide").await;
    let token = body["token"].as_str().unwrap();
    let (status, _) = call(&app, Method::GET, "/v1/me", Some(token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn trop_d_echecs_bloque_la_connexion() {
    let config = HubConfig {
        login_max_failures: 3,
        ..HubConfig::for_tests()
    };
    let app = app_with(config).await;
    register(&app, "f@example.org", "Fanny", "mot de passe solide").await;
    for _ in 0..3 {
        let (status, _) = login(&app, "f@example.org", "mauvais mot de passe").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    // Même le bon mot de passe est refusé pendant la fenêtre.
    let (status, body) = login(&app, "F@example.org", "mot de passe solide").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error_code(&body), "too_many_attempts");
}

async fn send_request(app: &Router, token: &str, username: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/v1/friends/requests",
        Some(token),
        Some(json!({ "username": username })),
    )
    .await
}

async fn friends(app: &Router, token: &str) -> Value {
    let (status, body) = call(app, Method::GET, "/v1/friends", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
    body
}

#[tokio::test]
async fn demande_d_ami_acceptee() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;

    // Le pseudo se tape sans tenir compte de la casse.
    let (status, body) = send_request(&app, &alice, "bob").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["status"], "pending");
    let (status, body) = send_request(&app, &alice, "Bob").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error_code(&body), "request_exists");

    let (_, requests) = call(&app, Method::GET, "/v1/friends/requests", Some(&bob), None).await;
    assert_eq!(
        requests,
        json!({ "incoming": [{ "username": "Alice" }], "outgoing": [] })
    );
    let (_, requests) = call(
        &app,
        Method::GET,
        "/v1/friends/requests",
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(
        requests,
        json!({ "incoming": [], "outgoing": [{ "username": "Bob" }] })
    );

    let (status, _) = call(
        &app,
        Method::POST,
        "/v1/friends/requests/alice/accept",
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert_eq!(friends(&app, &alice).await[0]["username"], "Bob");
    assert_eq!(friends(&app, &bob).await[0]["username"], "Alice");
    let (_, requests) = call(&app, Method::GET, "/v1/friends/requests", Some(&bob), None).await;
    assert_eq!(requests, json!({ "incoming": [], "outgoing": [] }));

    let (status, body) = send_request(&app, &bob, "Alice").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error_code(&body), "already_friends");
}

#[tokio::test]
async fn demandes_croisees_deviennent_amitie() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;
    send_request(&app, &alice, "Bob").await;
    let (status, body) = send_request(&app, &bob, "Alice").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "accepted");
    assert_eq!(friends(&app, &alice).await.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn refus_annulation_et_erreurs() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;

    let (status, body) = send_request(&app, &alice, "Alice").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "self_friend");
    let (status, body) = send_request(&app, &alice, "Personne").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "user_not_found");

    // Refus par Bob.
    send_request(&app, &alice, "Bob").await;
    let uri = "/v1/friends/requests/Alice/decline";
    let (status, _) = call(&app, Method::POST, uri, Some(&bob), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = call(&app, Method::POST, uri, Some(&bob), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "request_not_found");

    // Annulation par Alice.
    send_request(&app, &alice, "Bob").await;
    let (status, _) = call(
        &app,
        Method::DELETE,
        "/v1/friends/requests/Bob",
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, requests) = call(&app, Method::GET, "/v1/friends/requests", Some(&bob), None).await;
    assert_eq!(requests["incoming"], json!([]));

    // Accepter une demande qui n'existe pas.
    let (status, _) = call(
        &app,
        Method::POST,
        "/v1/friends/requests/Alice/accept",
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(friends(&app, &bob).await.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn retirer_un_ami() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;
    send_request(&app, &alice, "Bob").await;
    send_request(&app, &bob, "Alice").await;
    let (status, _) = call(&app, Method::DELETE, "/v1/friends/Bob", Some(&alice), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(friends(&app, &bob).await.as_array().unwrap().is_empty());
    let (status, body) = call(&app, Method::DELETE, "/v1/friends/Bob", Some(&alice), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "friend_not_found");
}

#[tokio::test]
async fn amis_ranges_par_pseudo() {
    let app = app().await;
    let me = account(&app, "Moi").await;
    for name in ["zoe", "Bruno", "anna"] {
        let other = account(&app, name).await;
        send_request(&app, &other, "Moi").await;
        send_request(&app, &me, name).await;
    }
    let names: Vec<String> = friends(&app, &me)
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["username"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["anna", "Bruno", "zoe"]);
}

async fn set_presence(app: &Router, token: &str, presence: Value) -> StatusCode {
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

#[tokio::test]
async fn presence_visible_par_les_amis() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;
    send_request(&app, &alice, "Bob").await;
    send_request(&app, &bob, "Alice").await;

    // Par défaut : hors ligne.
    assert_eq!(
        friends(&app, &alice).await[0]["presence"],
        json!({ "status": "offline" })
    );

    let status = set_presence(
        &app,
        &bob,
        json!({ "status": "in_game", "game": "Slime Rancher" }),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        friends(&app, &alice).await[0]["presence"],
        json!({ "status": "in_game", "game": "Slime Rancher" })
    );

    // « En ligne » efface le nom du jeu ; « hors ligne » efface la présence.
    set_presence(&app, &bob, json!({ "status": "online", "game": "ignoré" })).await;
    assert_eq!(
        friends(&app, &alice).await[0]["presence"],
        json!({ "status": "online" })
    );
    set_presence(&app, &bob, json!({ "status": "offline" })).await;
    assert_eq!(
        friends(&app, &alice).await[0]["presence"]["status"],
        "offline"
    );

    // La déconnexion efface aussi la présence.
    set_presence(&app, &bob, json!({ "status": "online" })).await;
    call(
        &app,
        Method::DELETE,
        "/v1/sessions/current",
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(
        friends(&app, &alice).await[0]["presence"]["status"],
        "offline"
    );
}

#[tokio::test]
async fn presence_invalide_refusee() {
    let app = app().await;
    let alice = account(&app, "Alice").await;
    let long = "x".repeat(65);
    for body in [
        json!({ "status": "in_game", "game": "" }),
        json!({ "status": "in_game", "game": long }),
        json!({ "status": "in_game", "game": "a\nb" }),
    ] {
        assert_eq!(
            set_presence(&app, &alice, body).await,
            StatusCode::BAD_REQUEST
        );
    }
    // Statut inconnu : refusé par la lecture du JSON, réponse au format du hub.
    let (status, body) = call(
        &app,
        Method::PUT,
        "/v1/presence",
        Some(&alice),
        Some(json!({ "status": "busy" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "bad_request");
}

#[tokio::test]
async fn presence_expire_sans_renouvellement() {
    let config = HubConfig {
        presence_ttl: Duration::from_millis(50),
        ..HubConfig::for_tests()
    };
    let app = app_with(config).await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;
    send_request(&app, &alice, "Bob").await;
    send_request(&app, &bob, "Alice").await;
    set_presence(&app, &bob, json!({ "status": "online" })).await;
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(
        friends(&app, &alice).await[0]["presence"]["status"],
        "offline"
    );
}

#[tokio::test]
async fn trop_de_demandes_d_ami() {
    let config = HubConfig {
        friend_requests_per_minute: 2,
        ..HubConfig::for_tests()
    };
    let app = app_with(config).await;
    let alice = account(&app, "Alice").await;
    for name in ["Bob", "Carla"] {
        account(&app, name).await;
        let (status, _) = send_request(&app, &alice, name).await;
        assert_eq!(status, StatusCode::CREATED);
    }
    account(&app, "Dora").await;
    let (status, _) = send_request(&app, &alice, "Dora").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn menage_des_sessions_expirees() {
    let store = Store::in_memory().await.unwrap();
    let state = AppState::new(store.clone(), HubConfig::for_tests())
        .await
        .unwrap();
    let app = hub::router(state);
    account(&app, "Gaby").await;
    let now = hub::now_unix();
    // La session de Gaby vit 30 jours : rien à retirer maintenant, tout dans 31 jours.
    assert_eq!(store.purge_expired_sessions(now).await.unwrap(), 0);
    let later = now + 31 * 24 * 3600;
    assert_eq!(store.purge_expired_sessions(later).await.unwrap(), 1);
}

#[tokio::test]
async fn base_fichier_rouverte_garde_les_comptes() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("hub.db").display());
    {
        let store = Store::open(&url).await.unwrap();
        let app = hub::router(AppState::new(store, HubConfig::for_tests()).await.unwrap());
        account(&app, "Hugo").await;
    }
    // Deuxième ouverture : le schéma existe déjà, sa version est vérifiée.
    let store = Store::open(&url).await.unwrap();
    let app = hub::router(AppState::new(store, HubConfig::for_tests()).await.unwrap());
    let (status, _) = login(&app, "hugo@example.org", "mot de passe solide").await;
    assert_eq!(status, StatusCode::OK);
}
