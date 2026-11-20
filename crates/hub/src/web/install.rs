//! Assistant de premier lancement : choix de la base de données (ADR 0037).
//!
//! Quand le hub démarre sans base choisie (ni `--db`, ni `RAFTUP_HUB_DB`, ni
//! fichier `rafthub-data/rafthub.toml`), il ne peut pas encore servir l'API ni la
//! console : elles ont besoin de la base. Il sert alors **seulement** cet assistant,
//! qui demande :
//! - la base intégrée (SQLite, un fichier dans `rafthub-data/`), pour un petit groupe ;
//! - ou une base externe choisie dans une liste (PostgreSQL), avec ses informations
//!   de connexion : PostgreSQL, MySQL ou MariaDB.
//!
//! Le choix est essayé (connexion et création des tables) avant d'être gardé dans
//! `rafthub.toml`. Ensuite l'assistant prévient `main`, qui arrête ce petit serveur
//! et lance le vrai hub sur la base choisie ; la page suivante est la création du
//! premier admin (`web/setup.rs`), avec le même code d'installation.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use askama::Template;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Form, Router};
use serde::Deserialize;
use tokio::sync::oneshot;

use super::{assets, cookie, csrf_refused, page, same_token};
use crate::server_config::{sqlite_url, Engine, ExternalDb, ServerConfig, EMBEDDED_URL};
use crate::session::{new_token, normalize_invite_code};
use crate::store::Store;

const CSRF_COOKIE: &str = "rafthub_install_csrf";
const PATH: &str = "/admin/installation";
/// Temps laissé à une base externe pour répondre pendant l'essai.
const DB_TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// État de l'assistant. Le `Sender` est dans un `Mutex<Option<…>>` : il ne peut
/// servir qu'une fois (`take()`), et plusieurs requêtes partagent l'état (`Arc`).
#[derive(Clone)]
pub struct InstallState {
    setup_code: String,
    data_dir: PathBuf,
    secure_cookies: bool,
    done: Arc<Mutex<Option<oneshot::Sender<String>>>>,
}

impl InstallState {
    /// `done` reçoit l'URL de la base choisie, une fois essayée et enregistrée.
    /// `secure_cookies` : faux seulement quand le hub est joint en `http://127.0.0.1`.
    pub fn new(
        setup_code: String,
        data_dir: PathBuf,
        secure_cookies: bool,
        done: oneshot::Sender<String>,
    ) -> InstallState {
        InstallState {
            setup_code,
            data_dir,
            secure_cookies,
            done: Arc::new(Mutex::new(Some(done))),
        }
    }
}

/// Routeur de l'assistant : sa page, les fichiers statiques, et tout le reste
/// renvoie vers la page (l'API répond 503 : le hub n'est pas encore prêt).
pub fn install_router(state: InstallState) -> Router {
    Router::new()
        .route(PATH, get(form).post(submit))
        .route("/admin/static/{*path}", get(assets::serve))
        .route("/v1/{*rest}", axum::routing::any(not_ready))
        .fallback(|| async { Redirect::to(PATH) })
        .with_state(state)
}

async fn not_ready() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "hub en cours d'installation",
    )
        .into_response()
}

#[derive(Template)]
#[template(path = "admin/install.html")]
struct InstallPage {
    csrf: String,
    /// `sqlite` ou `external` : le bouton coché.
    choice: String,
    /// Base externe choisie dans la liste : `postgres`, `mysql` ou `mariadb`.
    engine: String,
    db_host: String,
    /// Vide : port habituel de la base choisie (5432 ou 3306).
    db_port: String,
    db_name: String,
    db_user: String,
    error: Option<String>,
    /// Vrai une fois la base enregistrée : la page dit d'attendre puis continue.
    finished: bool,
}

impl InstallPage {
    fn empty(csrf: String) -> InstallPage {
        InstallPage {
            csrf,
            choice: "sqlite".to_string(),
            engine: Engine::Postgres.as_str().to_string(),
            db_host: "localhost".to_string(),
            db_port: String::new(),
            db_name: "rafthub".to_string(),
            db_user: "rafthub".to_string(),
            error: None,
            finished: false,
        }
    }
}

/// Corps du formulaire. Pas de `Debug` : il contient le mot de passe de la base.
#[derive(Deserialize)]
pub struct InstallForm {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    database: String,
    #[serde(default)]
    engine: String,
    #[serde(default)]
    db_host: String,
    #[serde(default)]
    db_port: String,
    #[serde(default)]
    db_name: String,
    #[serde(default)]
    db_user: String,
    #[serde(default)]
    db_password: String,
}

/// Rend la page avec un jeton anti-CSRF neuf, posé aussi dans un cookie.
fn render(state: &InstallState, status: StatusCode, mut content: InstallPage) -> Response {
    let csrf = new_token();
    content.csrf = csrf.clone();
    let mut response = page(status, &content);
    let secure = if state.secure_cookies { "; Secure" } else { "" };
    let text = format!(
        "{CSRF_COOKIE}={csrf}; Path={PATH}; Max-Age=3600; HttpOnly; SameSite=Strict{secure}"
    );
    if let Ok(value) = HeaderValue::from_str(&text) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

async fn form(State(state): State<InstallState>) -> Response {
    render(&state, StatusCode::OK, InstallPage::empty(String::new()))
}

async fn submit(
    State(state): State<InstallState>,
    headers: HeaderMap,
    Form(body): Form<InstallForm>,
) -> Response {
    let expected = cookie(&headers, CSRF_COOKIE).unwrap_or_default();
    if !same_token(&expected, &body.csrf) {
        return csrf_refused();
    }
    // La page remontrée garde ce qui a été tapé, sauf le mot de passe et le code.
    let mut again = InstallPage::empty(String::new());
    again.choice = if body.database == "external" {
        "external".to_string()
    } else {
        "sqlite".to_string()
    };
    if let Some(engine) = Engine::parse(&body.engine) {
        again.engine = engine.as_str().to_string();
    }
    again.db_host = body.db_host.clone();
    again.db_port = body.db_port.clone();
    again.db_name = body.db_name.clone();
    again.db_user = body.db_user.clone();

    if !same_token(
        &normalize_invite_code(&state.setup_code),
        &normalize_invite_code(&body.code),
    ) {
        tracing::info!("code d'installation refusé");
        again.error = Some(
            "Code d'installation incorrect. Il est écrit dans le journal du serveur au démarrage."
                .to_string(),
        );
        return render(&state, StatusCode::FORBIDDEN, again);
    }

    // `stored` : ce qui est écrit dans `rafthub.toml` ; `url` : ce qui est ouvert.
    let (stored, url) = match body.database.as_str() {
        "sqlite" => (EMBEDDED_URL.to_string(), sqlite_url(&state.data_dir)),
        "external" => {
            let Some(engine) = Engine::parse(&body.engine) else {
                again.error = Some("Choisissez le type de base dans la liste.".to_string());
                return render(&state, StatusCode::BAD_REQUEST, again);
            };
            let info = ExternalDb {
                engine,
                host: body.db_host,
                port: body.db_port,
                database: body.db_name,
                user: body.db_user,
                password: body.db_password,
            };
            match info.to_url() {
                Ok(url) => (url.clone(), url),
                Err(message) => {
                    again.error = Some(message);
                    return render(&state, StatusCode::BAD_REQUEST, again);
                }
            }
        }
        _ => {
            again.error = Some("Choisissez une base de données.".to_string());
            return render(&state, StatusCode::BAD_REQUEST, again);
        }
    };

    // Essai : connexion et création des tables. Le message d'erreur ne cite jamais
    // l'URL (elle contient le mot de passe).
    match tokio::time::timeout(DB_TEST_TIMEOUT, Store::open(&url)).await {
        Ok(Ok(_store)) => {}
        Ok(Err(error)) => {
            tracing::warn!(%error, "base refusée par l'assistant d'installation");
            again.error = Some(format!(
                "Connexion à la base impossible : {error}. Vérifiez l'adresse, le nom de la base, l'utilisateur et le mot de passe."
            ));
            return render(&state, StatusCode::BAD_REQUEST, again);
        }
        Err(_) => {
            again.error = Some(
                "La base n'a pas répondu en 10 secondes. Vérifiez l'adresse et le port, et qu'un pare-feu ne bloque pas la connexion."
                    .to_string(),
            );
            return render(&state, StatusCode::BAD_REQUEST, again);
        }
    }

    if let Err(error) = ServerConfig::new(stored).save(&state.data_dir) {
        tracing::error!(%error, "config du serveur non écrite");
        again.error = Some(format!("Le choix n'a pas pu être enregistré : {error}."));
        return render(&state, StatusCode::INTERNAL_SERVER_ERROR, again);
    }
    tracing::info!("base choisie dans l'assistant d'installation");

    // On prévient `main` ; un second envoi (double clic) ne fait rien de plus.
    let sender = state.done.lock().ok().and_then(|mut slot| slot.take());
    if let Some(sender) = sender {
        let _ = sender.send(url);
    }
    let mut finished = InstallPage::empty(String::new());
    finished.finished = true;
    render(&state, StatusCode::OK, finished)
}
