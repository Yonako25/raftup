//! Changement de pseudo par le joueur : `PATCH /v1/me` (mêmes règles qu'à
//! l'inscription, une fois par 24 heures, ligne `username_changed` au journal).

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::http::{Method, StatusCode};
use axum::Router;
use hub::{now_unix, AppState, HubConfig, Store};
use serde_json::{json, Value};

use common::{account, call, error_code, friends, login, send_request, test_store};

const PASSWORD: &str = "mot de passe solide";

async fn hub() -> (Router, Store) {
    let store = test_store().await;
    let state = AppState::new(store.clone(), HubConfig::for_tests())
        .await
        .unwrap();
    (hub::router(state), store)
}

async fn rename(app: &Router, token: &str, username: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::PATCH,
        "/v1/me",
        Some(token),
        Some(json!({ "username": username })),
    )
    .await
}

async fn my_id(app: &Router, token: &str) -> i64 {
    let (_, me) = call(app, Method::GET, "/v1/me", Some(token), None).await;
    me["user_id"].as_i64().unwrap()
}

#[tokio::test]
async fn les_amis_voient_le_nouveau_pseudo() {
    let (app, store) = hub().await;
    let alice = account(&app, "Alice").await;
    let bob = account(&app, "Bob").await;
    send_request(&app, &alice, "Bob").await;
    let (status, _) = call(
        &app,
        Method::POST,
        "/v1/friends/requests/Alice/accept",
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = rename(&app, &alice, "Alicia").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["username"], "Alicia");
    let (_, me) = call(&app, Method::GET, "/v1/me", Some(&alice), None).await;
    assert_eq!(me["username"], "Alicia");

    // Bob voit le nouveau pseudo, sans rien refaire.
    let list = friends(&app, &bob).await;
    assert_eq!(list[0]["username"], "Alicia");

    // Connexion par le nouveau pseudo ; l'ancien ne marche plus (ADR 0038).
    assert_eq!(login(&app, "alicia", PASSWORD).await.0, StatusCode::OK);
    assert_eq!(
        login(&app, "Alice", PASSWORD).await.0,
        StatusCode::UNAUTHORIZED
    );
    // L'ancien pseudo est libre : un nouveau joueur peut le prendre.
    let (status, _) = common::register(&app, "autre@example.org", "Alice", PASSWORD).await;
    assert_eq!(status, StatusCode::CREATED);

    // Une ligne au journal : le joueur agit sur lui-même, ancien → nouveau.
    let events = store.list_events(10, 0).await.unwrap();
    let event = events
        .items
        .iter()
        .find(|e| e.kind == "username_changed")
        .unwrap();
    assert_eq!(event.detail, "Alice → Alicia");
    assert_eq!(event.actor_id, event.target_id);
    assert_eq!(event.target_name.as_deref(), Some("Alicia"));
}

#[tokio::test]
async fn une_fois_par_jour() {
    let (app, store) = hub().await;
    let token = account(&app, "Mireille").await;
    assert_eq!(rename(&app, &token, "Mimi").await.0, StatusCode::OK);

    let (status, body) = rename(&app, &token, "Mireille2").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error_code(&body), "username_change_too_soon");
    // Remettre le même pseudo ne fait rien et n'est pas refusé.
    let (status, body) = rename(&app, &token, "Mimi").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["username"], "Mimi");
    let changes = store
        .list_events(50, 0)
        .await
        .unwrap()
        .items
        .into_iter()
        .filter(|e| e.kind == "username_changed")
        .count();
    assert_eq!(changes, 1);
}

#[tokio::test]
async fn limite_de_24_heures() {
    let (app, store) = hub().await;
    let token = account(&app, "Kael").await;
    let id = my_id(&app, &token).await;
    let hour = 3600;

    // Un changement il y a 23 heures : encore trop tôt.
    store
        .insert_event(
            now_unix() - 23 * hour,
            Some(id),
            "username_changed",
            Some(id),
            "",
        )
        .await
        .unwrap();
    assert_eq!(
        rename(&app, &token, "Kaelis").await.0,
        StatusCode::TOO_MANY_REQUESTS
    );

    // Avec seulement un changement il y a 25 heures : permis.
    let (app, store) = hub().await;
    let token = account(&app, "Kael").await;
    let id = my_id(&app, &token).await;
    store
        .insert_event(
            now_unix() - 25 * hour,
            Some(id),
            "username_changed",
            Some(id),
            "",
        )
        .await
        .unwrap();
    // Un changement fait par un admin (autre acteur) ne compte pas.
    store
        .insert_event(now_unix(), None, "username_changed", Some(id), "")
        .await
        .unwrap();
    assert_eq!(rename(&app, &token, "Kaelis").await.0, StatusCode::OK);
}

#[tokio::test]
async fn memes_regles_qu_a_l_inscription() {
    let (app, _) = hub().await;
    let token = account(&app, "Alice").await;
    account(&app, "Bob").await;

    // Pris, même avec d'autres majuscules.
    let (status, body) = rename(&app, &token, "BOB").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error_code(&body), "username_taken");
    for bad in ["ab", "un pseudo", "admin", &"x".repeat(25)] {
        let (status, body) = rename(&app, &token, bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(error_code(&body), "invalid_username", "{bad}");
    }
    // Les refus ne comptent pas dans la limite ; changer la casse est permis.
    let (status, body) = rename(&app, &token, "ALICE").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["username"], "ALICE");

    // Sans jeton : 401.
    let (status, _) = call(
        &app,
        Method::PATCH,
        "/v1/me",
        None,
        Some(json!({ "username": "Zoe" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn suppression_du_compte_efface_les_anciens_pseudos() {
    let (app, store) = hub().await;
    let token = account(&app, "Alice").await;
    assert_eq!(rename(&app, &token, "Alicia").await.0, StatusCode::OK);
    let (status, _) = call(
        &app,
        Method::DELETE,
        "/v1/me",
        Some(&token),
        Some(json!({ "password": PASSWORD })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let events = store.list_events(50, 0).await.unwrap();
    let event = events
        .items
        .iter()
        .find(|e| e.kind == "username_changed")
        .unwrap();
    assert_eq!(event.detail, "");
    assert_eq!(event.target_id, None);
}
