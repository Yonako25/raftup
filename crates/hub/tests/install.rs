//! Assistant de premier lancement : choix de la base (ADR 0037). Tout passe par le
//! routeur en `oneshot`, sans ouvrir de port.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use hub::server_config::ServerConfig;
use hub::web::install::{install_router, InstallState};
use tokio::sync::oneshot;
use tower::ServiceExt;

const CODE: &str = "K7QX-2M9P-AB12-CD34";

struct Reply {
    status: StatusCode,
    set_cookie: Vec<String>,
    location: String,
    body: String,
}

impl Reply {
    fn csrf(&self) -> String {
        let marker = "name=\"csrf\" value=\"";
        let start = self.body.find(marker).expect("champ csrf") + marker.len();
        let end = self.body[start..].find('"').unwrap();
        self.body[start..start + end].to_string()
    }
}

async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    Reply {
        status,
        set_cookie: headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect(),
        location: headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string(),
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

async fn get(app: &Router, uri: &str) -> Reply {
    send(
        app,
        Request::builder().uri(uri).body(Body::empty()).unwrap(),
    )
    .await
}

/// Ouvre la page puis envoie le formulaire avec ces champs (le CSRF en plus).
async fn submit(app: &Router, fields: &[(&str, &str)]) -> Reply {
    let page = get(app, "/admin/installation").await;
    assert_eq!(page.status, StatusCode::OK);
    let csrf = page.csrf();
    let mut body = url::form_urlencoded::Serializer::new(String::new());
    body.append_pair("csrf", &csrf);
    for (key, value) in fields {
        body.append_pair(key, value);
    }
    send(
        app,
        Request::builder()
            .method(Method::POST)
            .uri("/admin/installation")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::COOKIE, format!("rafthub_install_csrf={csrf}"))
            .body(Body::from(body.finish()))
            .unwrap(),
    )
    .await
}

fn wizard(dir: &std::path::Path) -> (Router, oneshot::Receiver<String>) {
    let (tx, rx) = oneshot::channel();
    let state = InstallState::new(CODE.to_string(), dir.to_path_buf(), false, tx);
    (install_router(state), rx)
}

#[tokio::test]
async fn tout_mene_a_l_assistant_et_l_api_attend() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _rx) = wizard(dir.path());
    for uri in ["/", "/admin", "/admin/login", "/telecharger"] {
        let reply = get(&app, uri).await;
        assert_eq!(reply.location, "/admin/installation", "{uri}");
    }
    assert_eq!(
        get(&app, "/v1/me").await.status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let page = get(&app, "/admin/installation").await;
    assert!(page.body.contains("Base intégrée (SQLite)"));
    assert!(page.body.contains("PostgreSQL"));
    assert!(page.body.contains("MySQL") && page.body.contains("MariaDB"));
    assert!(!page.body.contains("disabled"));
    assert!(page
        .set_cookie
        .iter()
        .any(|c| c.starts_with("rafthub_install_csrf=")));
    assert_eq!(
        get(&app, "/admin/static/rafthub.css").await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn base_integree_essayee_puis_gardee() {
    let dir = tempfile::tempdir().unwrap();
    let (app, rx) = wizard(dir.path());
    let reply = submit(
        &app,
        &[("code", "k7qx2m9pab12cd34"), ("database", "sqlite")],
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert!(reply.body.contains("Base de données prête"));
    // `main` reçoit l'URL, et le choix est gardé pour les prochains lancements.
    let url = rx.await.unwrap();
    assert!(url.starts_with("sqlite://"), "{url}");
    let saved = ServerConfig::load(dir.path()).unwrap().unwrap();
    assert_eq!(saved.database_url, hub::server_config::EMBEDDED_URL);
    assert_eq!(saved.database_url_in(dir.path()), url);
    assert!(dir.path().join("rafthub.db").exists());
}

#[tokio::test]
async fn mauvais_code_refuse_rien_n_est_garde() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _rx) = wizard(dir.path());
    let reply = submit(
        &app,
        &[("code", "AAAA-BBBB-CCCC-DDDD"), ("database", "sqlite")],
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert!(reply.body.contains("installation incorrect"));
    assert_eq!(ServerConfig::load(dir.path()).unwrap(), None);
}

#[tokio::test]
async fn base_externe_incomplete_ou_injoignable() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _rx) = wizard(dir.path());
    // Sans adresse : message clair, et les champs tapés sont remis.
    let reply = submit(
        &app,
        &[
            ("code", CODE),
            ("database", "external"),
            ("engine", "postgres"),
            ("db_host", ""),
            ("db_name", "mabase"),
            ("db_user", "moi"),
            ("db_password", "motdepasse-secret"),
        ],
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(reply.body.contains("adresse du serveur"));
    assert!(reply.body.contains("mabase"));
    assert!(!reply.body.contains("motdepasse-secret"));

    // Serveur qui refuse la connexion (port fermé) : erreur sans le mot de passe.
    let reply = submit(
        &app,
        &[
            ("code", CODE),
            ("database", "external"),
            ("engine", "postgres"),
            ("db_host", "127.0.0.1"),
            ("db_port", "1"),
            ("db_name", "mabase"),
            ("db_user", "moi"),
            ("db_password", "motdepasse-secret"),
        ],
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
    assert!(
        reply.body.contains("Connexion à la base impossible") || reply.body.contains("pas répondu")
    );
    assert!(!reply.body.contains("motdepasse-secret"));
    assert_eq!(ServerConfig::load(dir.path()).unwrap(), None);
}

#[tokio::test]
async fn formulaire_sans_csrf_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _rx) = wizard(dir.path());
    let reply = send(
        &app,
        Request::builder()
            .method(Method::POST)
            .uri("/admin/installation")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(format!("code={CODE}&database=sqlite&csrf=x")))
            .unwrap(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(ServerConfig::load(dir.path()).unwrap(), None);
}
