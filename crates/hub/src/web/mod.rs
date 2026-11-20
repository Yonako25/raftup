//! Console d'admin web de Rafthub, sous `/admin` (ADR 0036).
//!
//! Pages HTML rendues par le serveur avec des gabarits askama
//! (`crates/hub/templates/admin/`), sans JavaScript : chaque action est un
//! formulaire `POST`. Une page = un fichier de ce dossier.
//!
//! | Méthode et chemin | Rôle |
//! | --- | --- |
//! | `GET /admin` | tableau de bord |
//! | `GET /admin/login` | formulaire de connexion |
//! | `POST /admin/login` | connexion (`email`, `password`, `csrf`) |
//! | `POST /admin/logout` | déconnexion (`csrf`) |
//! | `GET /admin/distribution` | distribution du launcher (exe en cache, liens) |
//! | `POST /admin/distribution/rafraichir` | retélécharge l'exe signé (`csrf`) |
//! | `GET /telecharger` | page publique de téléchargement (404 si désactivée) |
//! | `GET /admin/distribution/raftup-windows.zip` | zip relié au hub, pour un admin |
//! | `GET /telecharger/raftup-windows.zip` | le même zip, public (404 si la page est désactivée) |
//! | `GET /admin/joueurs`, `/admin/reglages`, `/admin/journaux` | page « Bientôt » |
//! | `GET /admin/static/…` | feuille de style et polices |
//!
//! **Connexion** : un cookie `rafthub_admin` porte un jeton opaque, dont la base ne
//! garde que l'empreinte (table `admin_sessions`). Les jetons Bearer de l'API ne
//! donnent pas accès : ils vivent dans une autre table et ne sont jamais lus ici.
//!
//! **Anti-CSRF** : chaque formulaire porte un champ caché `csrf`. Une fois connecté,
//! sa valeur est tirée du jeton de session (empreinte SHA-256 avec un préfixe), donc
//! propre à la session et impossible à deviner sans le cookie. Pour le formulaire
//! de connexion, qui vient avant toute session, un cookie à part
//! (`rafthub_login_csrf`) doit porter la même valeur que le champ.

mod assets;
mod dashboard;
mod distribution;
mod login;
mod soon;

use askama::Template;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use sha2::{Digest, Sha256};

use crate::error::HubError;
use crate::session::token_hash;
use crate::{now_unix, AppState};

/// Nom du cookie de session de la console.
pub const SESSION_COOKIE: &str = "rafthub_admin";
/// Nom du cookie anti-CSRF du formulaire de connexion.
pub const LOGIN_CSRF_COOKIE: &str = "rafthub_login_csrf";

/// Routes de la console, à fusionner dans le routeur du hub.
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin", get(dashboard::show))
        .route("/admin/", get(|| async { Redirect::to("/admin") }))
        .route("/admin/login", get(login::form).post(login::submit))
        .route("/admin/logout", post(login::logout))
        .route("/admin/joueurs", get(soon::players))
        .route("/admin/distribution", get(distribution::show))
        .route(
            "/admin/distribution/rafraichir",
            post(distribution::refresh),
        )
        .route("/telecharger", get(distribution::public_page))
        .route(distribution::ADMIN_ZIP_PATH, get(distribution::admin_zip))
        .route(distribution::ZIP_PATH, get(distribution::public_zip))
        .route("/admin/reglages", get(soon::settings))
        .route("/admin/journaux", get(soon::logs))
        .route("/admin/static/{*path}", get(assets::serve))
}

// ---------- Mise en page commune ----------

/// Ce que le gabarit de base (`admin/base.html`) affiche autour de chaque page :
/// le menu de gauche, le compte connecté et le formulaire de déconnexion.
pub struct Layout {
    /// Entrée du menu à mettre en avant : `dashboard`, `players`, `distribution`,
    /// `settings` ou `logs`.
    pub active: &'static str,
    pub username: String,
    /// Jeton anti-CSRF de la session, pour le bouton « Se déconnecter ».
    pub csrf: String,
    pub version: &'static str,
}

impl Layout {
    fn new(session: &AdminSession, active: &'static str) -> Layout {
        Layout {
            active,
            username: session.username.clone(),
            csrf: session.csrf.clone(),
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}

/// Rend une page HTML avec les en-têtes de sécurité de la console.
///
/// - `Content-Security-Policy` : rien d'autre que ce hub (pas de script, de police
///   ni d'image venus d'ailleurs), pas d'affichage dans un cadre ;
/// - `Cache-Control: no-store` : une page d'admin ne reste pas dans un cache.
fn page(status: StatusCode, template: &impl Template) -> Response {
    let html = match template.render() {
        Ok(html) => html,
        Err(error) => {
            tracing::error!(%error, "gabarit de la console non rendu");
            return (StatusCode::INTERNAL_SERVER_ERROR, "erreur interne").into_response();
        }
    };
    let mut response = (status, Html(html)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; \
             img-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Page d'erreur simple (refus CSRF, erreur interne…).
#[derive(Template)]
#[template(path = "admin/error.html")]
struct ErrorPage {
    title: &'static str,
    message: &'static str,
}

fn error_page(status: StatusCode, title: &'static str, message: &'static str) -> Response {
    page(status, &ErrorPage { title, message })
}

/// Une erreur du hub (base indisponible…) vue depuis une page HTML : le détail
/// reste dans le journal du serveur, la page ne dit que « erreur interne ».
fn internal_error(error: HubError) -> Response {
    tracing::error!(%error, "erreur dans la console d'admin");
    error_page(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Erreur interne",
        "Le hub n'a pas pu traiter la demande. Le détail est dans son journal.",
    )
}

// ---------- Cookies ----------

/// Valeur d'un cookie de la requête, s'il est là.
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    // Un navigateur peut envoyer plusieurs en-têtes `Cookie`, chacun avec
    // plusieurs paires `nom=valeur` séparées par `;`.
    for value in headers.get_all(header::COOKIE) {
        let Ok(text) = value.to_str() else { continue };
        for pair in text.split(';') {
            if let Some((key, value)) = pair.trim().split_once('=') {
                if key == name && !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Vrai si le hub est joint en HTTP sur la machine elle-même : c'est le seul cas où
/// le cookie n'a pas l'attribut `Secure` (un navigateur refuse un cookie `Secure`
/// reçu en HTTP). Partout ailleurs, la console n'est ouverte qu'en HTTPS.
fn is_local_http(public_url: &str) -> bool {
    ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|prefix| {
            public_url
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
        })
}

/// En-tête `Set-Cookie` d'un cookie de la console. `max_age = 0` efface le cookie.
fn set_cookie(state: &AppState, name: &str, value: &str, path: &str, max_age: u64) -> HeaderValue {
    let secure = if is_local_http(&state.inner.config.public_url) {
        ""
    } else {
        "; Secure"
    };
    let text = format!(
        "{name}={value}; Path={path}; Max-Age={max_age}; HttpOnly; SameSite=Strict{secure}"
    );
    // Le texte ne contient que des caractères ASCII imprimables (jeton hexadécimal) ;
    // en cas de surprise, on envoie un cookie vide plutôt que de planter.
    HeaderValue::from_str(&text).unwrap_or_else(|_| HeaderValue::from_static(""))
}

// ---------- Anti-CSRF ----------

/// Jeton anti-CSRF d'une session : empreinte du jeton de session avec un préfixe
/// propre à cet usage (il ne révèle donc pas le jeton lui-même).
fn session_csrf(session_token: &str) -> String {
    let digest = Sha256::digest(format!("rafthub-csrf:{session_token}").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Comparaison en temps constant : le temps de réponse ne dit pas combien de
/// caractères du jeton envoyé étaient justes.
fn same_token(a: &str, b: &str) -> bool {
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn csrf_refused() -> Response {
    error_page(
        StatusCode::FORBIDDEN,
        "Formulaire refusé",
        "Le formulaire a expiré ou ne vient pas de cette console. Rechargez la page et recommencez.",
    )
}

// ---------- Session d'admin ----------

/// Admin connecté, tiré du cookie `rafthub_admin`.
///
/// Comme `AuthUser` pour l'API : mettre `session: AdminSession` dans les paramètres
/// d'une route suffit à la protéger. Sans session valide, axum renvoie vers la page
/// de connexion au lieu d'appeler la route.
pub struct AdminSession {
    pub user_id: i64,
    pub username: String,
    /// Empreinte du jeton, pour la déconnexion.
    token_hash: String,
    /// Jeton anti-CSRF attendu dans les formulaires de cette session.
    csrf: String,
}

impl AdminSession {
    /// Vrai si le champ `csrf` envoyé par un formulaire est celui de la session.
    fn csrf_ok(&self, sent: &str) -> bool {
        same_token(&self.csrf, sent)
    }

    /// Session de la requête, ou `None` (pas de cookie, jeton inconnu ou expiré,
    /// compte qui n'est plus admin).
    async fn from_headers(
        headers: &HeaderMap,
        state: &AppState,
    ) -> Result<Option<AdminSession>, HubError> {
        let Some(token) = cookie(headers, SESSION_COOKIE) else {
            return Ok(None);
        };
        let hash = token_hash(&token);
        let found = state.store().admin_session_user(&hash, now_unix()).await?;
        Ok(found.map(|(user_id, username)| AdminSession {
            user_id,
            username,
            token_hash: hash,
            csrf: session_csrf(&token),
        }))
    }
}

impl FromRequestParts<AppState> for AdminSession {
    // `Response` : en cas d'échec, axum renvoie directement cette réponse
    // (redirection vers la connexion, ou page d'erreur).
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        match AdminSession::from_headers(&parts.headers, state).await {
            Ok(Some(session)) => Ok(session),
            Ok(None) => Err(Redirect::to("/admin/login").into_response()),
            Err(error) => Err(internal_error(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_local_reconnu() {
        assert!(is_local_http("http://127.0.0.1:8787"));
        assert!(is_local_http("http://localhost"));
        assert!(is_local_http("http://[::1]:80/"));
        assert!(!is_local_http("https://127.0.0.1:8787"));
        assert!(!is_local_http("http://127.0.0.1.exemple.org"));
        assert!(!is_local_http("http://hub.exemple.org"));
    }

    #[test]
    fn comparaison_des_jetons() {
        assert!(same_token("abc", "abc"));
        assert!(!same_token("abc", "abd"));
        assert!(!same_token("abc", "ab"));
        assert!(!same_token("", ""));
    }

    #[test]
    fn cookie_lu_parmi_plusieurs() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; rafthub_admin=abc; b=2"),
        );
        assert_eq!(cookie(&headers, SESSION_COOKIE).as_deref(), Some("abc"));
        assert_eq!(cookie(&headers, "c"), None);
    }

    #[test]
    fn csrf_propre_a_la_session() {
        assert_ne!(session_csrf("a"), session_csrf("b"));
        assert_ne!(session_csrf("a"), "a");
        assert_eq!(session_csrf("a").len(), 64);
    }
}
