//! Console d'admin servie à part (`--admin-listen`, ADR 0040) : le routeur public
//! ne connaît pas `/admin`, le routeur de la console ne connaît pas l'API.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use hub::store::Role;
use hub::{AppState, HubConfig};
use tower::ServiceExt;

/// Code HTTP, en-têtes `Set-Cookie` et corps d'un `GET`.
async fn get(app: &Router, uri: &str) -> (StatusCode, Vec<String>, String) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 22)
        .await
        .unwrap();
    (
        status,
        cookies,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// État d'un hub d'essai, avec une adresse publique en HTTPS (cas du serveur officiel)
/// et un admin déjà créé (sinon `/admin/login` renvoie vers l'installation).
async fn state(admin_separate: bool) -> AppState {
    let mut config = HubConfig::for_tests();
    config.public_url = "https://hub.exemple.org".to_string();
    config.admin_separate = admin_separate;
    let store = common::test_store().await;
    let state = AppState::new(store.clone(), config).await.unwrap();
    let app = hub::router(state.clone());
    let (status, _) =
        common::register(&app, "chef@example.org", "Chef", "mot de passe solide 42").await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(store
        .set_role_by_email("chef@example.org", Role::Admin)
        .await
        .unwrap());
    state
}

#[tokio::test]
async fn la_console_est_inconnue_du_routeur_public() {
    let public = hub::public_router(state(true).await);
    for uri in [
        "/admin",
        "/admin/",
        "/admin/login",
        "/admin/installation",
        "/admin/joueurs",
        "/admin/static/rafthub.css",
        "/admin/distribution/raftup-windows.zip",
    ] {
        let (status, _, body) = get(&public, uri).await;
        // 404 sans corps, comme n'importe quelle adresse qui n'existe pas.
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert!(body.is_empty(), "{uri} : {body}");
    }
    // Une adresse inventée répond exactement pareil : rien ne distingue `/admin`.
    let (status, _, body) = get(&public, "/nimporte-quoi").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.is_empty());

    // L'API et les fichiers statiques publics restent servis.
    let (status, _, _) = get(&public, "/v1").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = get(&public, "/static/rafthub.css").await;
    assert_eq!(status, StatusCode::OK);
    // La sonde de santé de Docker vise l'adresse publique : elle doit y trouver 200,
    // sinon le conteneur passe « malade » et Traefik ne le publie pas.
    let (status, _, _) = get(&public, hub::health::PROBE_PATH).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn le_routeur_de_la_console_ne_sert_pas_l_api() {
    let admin = hub::admin_router(state(true).await);
    let (status, _, body) = get(&admin, "/admin/login").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<form"), "{body}");
    let (status, _, _) = get(&admin, "/admin/static/rafthub.css").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = get(&admin, "/v1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn la_page_publique_ne_mentionne_pas_la_console() {
    let mut config = HubConfig::for_tests();
    config.admin_separate = true;
    let app = common::app_with(config).await;
    let (status, _, body) = get(&app, "/telecharger").await;
    // Page activée ou non selon les réglages : dans les deux cas, pas de `/admin`.
    assert!(
        status == StatusCode::OK || status == StatusCode::NOT_FOUND,
        "{status}"
    );
    assert!(!body.contains("/admin"), "{body}");
}

#[tokio::test]
async fn cookies_sans_secure_quand_la_console_est_a_part() {
    // Console à part, jointe en HTTP clair par le VPN : pas d'attribut `Secure`.
    let admin = hub::admin_router(state(true).await);
    let (_, cookies, _) = get(&admin, "/admin/login").await;
    assert!(!cookies.is_empty());
    assert!(cookies.iter().all(|c| !c.contains("Secure")), "{cookies:?}");

    // Console avec l'API, adresse publique en HTTPS : `Secure`, comme avant.
    let joined = hub::router(state(false).await);
    let (_, cookies, _) = get(&joined, "/admin/login").await;
    assert!(!cookies.is_empty());
    assert!(cookies.iter().all(|c| c.contains("Secure")), "{cookies:?}");
}
