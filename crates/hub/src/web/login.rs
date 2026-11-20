//! Connexion et déconnexion de la console d'admin (ADR 0036).
//!
//! E-mail ou pseudo + mot de passe seulement (pas de Discord, ADR 0038), mêmes hachages argon2 que
//! l'API. Seul un compte au rôle `admin` entre ; pour tous les autres cas (adresse
//! inconnue, mauvais mot de passe, compte joueur), la page dit la même chose, pour
//! ne pas révéler quelles adresses existent ni lesquelles sont admin.

use askama::Template;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;

use super::{
    cookie, csrf_refused, internal_error, page, same_token, set_cookie, AdminSession,
    LOGIN_CSRF_COOKIE, SESSION_COOKIE,
};
use crate::error::HubError;
use crate::identity::LoginName;
use crate::password::{verify_password, Password};
use crate::session::{new_token, token_hash};
use crate::{blocking, now_unix, AppState};

/// Durée de vie du cookie anti-CSRF du formulaire de connexion (1 h).
const LOGIN_CSRF_SECONDS: u64 = 3600;

#[derive(Template)]
#[template(path = "admin/login.html")]
struct LoginPage {
    csrf: String,
    /// Adresse déjà tapée, remise dans le champ après un échec.
    email: String,
    error: Option<&'static str>,
}

/// Corps du formulaire de connexion. `#[serde(default)]` : un champ absent devient
/// une chaîne vide (refusée plus loin) au lieu d'une erreur 422 peu lisible.
/// `Debug` est sûr : celui de [`Password`] est masqué.
#[derive(Debug, Deserialize)]
pub struct LoginForm {
    #[serde(default)]
    email: String,
    password: Password,
    #[serde(default)]
    csrf: String,
}

/// Formulaire de la page de déconnexion (et, plus tard, des autres pages).
#[derive(Debug, Deserialize)]
pub struct CsrfForm {
    #[serde(default)]
    csrf: String,
}

/// Affiche le formulaire avec un jeton anti-CSRF neuf, posé aussi dans un cookie.
fn login_page(
    state: &AppState,
    status: StatusCode,
    email: String,
    error: Option<&'static str>,
) -> Response {
    let csrf = new_token();
    let mut response = page(
        status,
        &LoginPage {
            csrf: csrf.clone(),
            email,
            error,
        },
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(
            state,
            LOGIN_CSRF_COOKIE,
            &csrf,
            "/admin/login",
            LOGIN_CSRF_SECONDS,
        ),
    );
    response
}

/// `GET /admin/login` : déjà connecté, on va au tableau de bord.
pub async fn form(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match AdminSession::from_headers(&headers, &state).await {
        Ok(Some(_)) => Redirect::to("/admin").into_response(),
        Ok(None) => match state.store().has_admin().await {
            // Hub neuf, sans admin : on va d'abord à l'installation (ADR 0037).
            Ok(false) => Redirect::to("/admin/installation").into_response(),
            Ok(true) => login_page(&state, StatusCode::OK, String::new(), None),
            Err(error) => internal_error(error),
        },
        Err(error) => internal_error(error),
    }
}

/// `POST /admin/login`.
pub async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(body): Form<LoginForm>,
) -> Response {
    // 1. Le formulaire vient bien de notre page (cookie et champ identiques).
    let expected = cookie(&headers, LOGIN_CSRF_COOKIE).unwrap_or_default();
    if !same_token(&expected, &body.csrf) {
        return csrf_refused();
    }

    const BAD_LOGIN: &str =
        "E-mail, pseudo ou mot de passe incorrect, ou compte sans accès à la console.";
    let typed = body.email.trim().to_string();
    // E-mail ou pseudo (ADR 0038).
    let Some(name) = LoginName::parse(&body.email) else {
        return login_page(&state, StatusCode::UNAUTHORIZED, typed, Some(BAD_LOGIN));
    };
    let key = name.limiter_key();

    // 2. Trop d'échecs récents pour ce compte : on ne vérifie même pas.
    let limiter = &state.inner.admin_login_limiter;
    if limiter.is_blocked(&key) {
        return login_page(
            &state,
            StatusCode::TOO_MANY_REQUESTS,
            typed,
            Some("Trop d'essais ratés pour ce compte. Réessayez dans un quart d'heure."),
        );
    }

    // 3. Vérification du mot de passe, puis du rôle.
    match check_admin(&state, &name, body.password).await {
        Ok(Some(user_id)) => {
            limiter.reset(&key);
            match open_admin_session(&state, user_id).await {
                Ok(response) => response,
                Err(error) => internal_error(error),
            }
        }
        Ok(None) => {
            limiter.hit(&key);
            tracing::info!("échec de connexion à la console d'admin");
            login_page(&state, StatusCode::UNAUTHORIZED, typed, Some(BAD_LOGIN))
        }
        Err(error) => internal_error(error),
    }
}

/// Le compte admin qui correspond à ces identifiants, ou `None`.
async fn check_admin(
    state: &AppState,
    name: &LoginName,
    password: Password,
) -> Result<Option<i64>, HubError> {
    let identity = crate::api::password_identity(state, name).await?;
    // Adresse inconnue : on vérifie quand même contre un hash bidon, pour que la
    // réponse prenne le même temps (comme l'API).
    let (user_id, phc) = match identity {
        Some((user_id, phc)) => (Some(user_id), phc),
        None => (None, state.inner.dummy_hash.clone()),
    };
    let matches = blocking(move || verify_password(&password, &phc)).await?;
    match (user_id, matches) {
        (Some(user_id), true) if state.store().is_admin(user_id).await? => Ok(Some(user_id)),
        _ => Ok(None),
    }
}

/// Crée la session d'admin, pose le cookie et renvoie vers le tableau de bord.
pub(super) async fn open_admin_session(
    state: &AppState,
    user_id: i64,
) -> Result<Response, HubError> {
    let token = new_token();
    let now = now_unix();
    let lifetime = state.inner.config.admin_session_lifetime.as_secs();
    let expires_at = now.saturating_add(i64::try_from(lifetime).unwrap_or(i64::MAX));
    state
        .store()
        .insert_admin_session(&token_hash(&token), user_id, now, expires_at)
        .await?;
    tracing::info!(user_id, "session d'admin ouverte");

    let mut response = Redirect::to("/admin").into_response();
    let headers = response.headers_mut();
    headers.append(
        header::SET_COOKIE,
        set_cookie(state, SESSION_COOKIE, &token, "/admin", lifetime),
    );
    // Le jeton du formulaire de connexion a servi : on l'efface.
    headers.append(
        header::SET_COOKIE,
        set_cookie(state, LOGIN_CSRF_COOKIE, "", "/admin/login", 0),
    );
    Ok(response)
}

/// `POST /admin/logout` : ferme la session et efface le cookie.
pub async fn logout(
    State(state): State<AppState>,
    session: AdminSession,
    Form(body): Form<CsrfForm>,
) -> Response {
    if !session.csrf_ok(&body.csrf) {
        return csrf_refused();
    }
    if let Err(error) = state
        .store()
        .delete_admin_session(&session.token_hash)
        .await
    {
        return internal_error(error);
    }
    tracing::info!(user_id = session.user_id, "session d'admin fermée");
    let mut response = Redirect::to("/admin/login").into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(&state, SESSION_COOKIE, "", "/admin", 0),
    );
    response
}
