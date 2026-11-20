//! Installation : création du premier admin depuis le navigateur (ADR 0037).
//!
//! Tant qu'aucun compte n'a le rôle d'admin, le hub écrit au démarrage un code
//! d'installation dans son journal. Cette page demande ce code, puis crée un compte
//! (e-mail, pseudo, mot de passe) qui devient admin et ouvre sa session. Dès qu'un
//! admin existe, la page renvoie vers la connexion : elle ne sert qu'une fois.
//!
//! Le code prouve qu'on a accès au serveur (seul son journal le montre) : sans lui,
//! le premier venu pourrait prendre la console d'un hub fraîchement installé.
//!
//! Le navigateur qui vient de réussir l'étape 1 (choix de la base) a reçu le cookie
//! `rafthub_setup_pass` : son jeton, gardé en mémoire par le hub
//! (`HubConfig::setup_pass`), vaut le code, qui n'est donc pas redemandé. Sans ce
//! cookie (autre navigateur, hub relancé), le code est demandé comme avant.

use askama::Template;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;

use super::install::SETUP_PASS_COOKIE;
use super::login::open_admin_session;
use super::{cookie, csrf_refused, internal_error, page, same_token, set_cookie};
use crate::console::EventKind;
use crate::error::HubError;
use crate::identity::{normalize_email, parse_username};
use crate::password::{hash_password, Password};
use crate::session::{new_token, normalize_invite_code};
use crate::store::Role;
use crate::{blocking, now_unix, AppState};

/// Cookie anti-CSRF du formulaire d'installation, comme celui de la connexion.
const SETUP_CSRF_COOKIE: &str = "rafthub_setup_csrf";
const SETUP_PATH: &str = "/admin/installation";

#[derive(Template)]
#[template(path = "admin/setup.html")]
struct SetupPage {
    csrf: String,
    /// Faux si le hub n'a pas de code (lancé sans passer par `rafthub`) : on explique.
    open: bool,
    /// Faux quand le navigateur a déjà donné le code à l'étape 1 : champ caché.
    ask_code: bool,
    email: String,
    username: String,
    error: Option<String>,
}

/// Corps du formulaire. `Debug` est sûr : celui de [`Password`] est masqué.
#[derive(Debug, Deserialize)]
pub struct SetupForm {
    #[serde(default)]
    code: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    username: String,
    password: Password,
    /// Le mot de passe tapé une seconde fois, pour éviter une faute de frappe.
    password_confirm: Password,
    #[serde(default)]
    csrf: String,
}

/// Vrai si la requête porte le cookie de l'étape 1 avec le bon jeton.
fn has_setup_pass(state: &AppState, headers: &HeaderMap) -> bool {
    match (
        &state.inner.config.setup_pass,
        cookie(headers, SETUP_PASS_COOKIE),
    ) {
        (Some(expected), Some(given)) => same_token(expected, &given),
        _ => false,
    }
}

fn setup_page(
    state: &AppState,
    headers: &HeaderMap,
    status: StatusCode,
    email: String,
    username: String,
    error: Option<String>,
) -> Response {
    let csrf = new_token();
    let mut response = page(
        status,
        &SetupPage {
            csrf: csrf.clone(),
            open: state.inner.config.setup_code.is_some(),
            ask_code: !has_setup_pass(state, headers),
            email,
            username,
            error,
        },
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(state, SETUP_CSRF_COOKIE, &csrf, SETUP_PATH, 3600),
    );
    response
}

/// `GET /admin/installation`.
pub async fn form(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match state.store().has_admin().await {
        Ok(true) => Redirect::to("/admin/login").into_response(),
        Ok(false) => setup_page(
            &state,
            &headers,
            StatusCode::OK,
            String::new(),
            String::new(),
            None,
        ),
        Err(error) => internal_error(error),
    }
}

/// `POST /admin/installation`.
pub async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(body): Form<SetupForm>,
) -> Response {
    let expected = cookie(&headers, SETUP_CSRF_COOKIE).unwrap_or_default();
    if !same_token(&expected, &body.csrf) {
        return csrf_refused();
    }
    let email_typed = body.email.trim().to_string();
    let username_typed = body.username.trim().to_string();
    let passed = has_setup_pass(&state, &headers);
    match create_first_admin(&state, body, passed).await {
        Ok(Ok(user_id)) => match open_admin_session(&state, user_id).await {
            Ok(mut response) => {
                // Le jeton du formulaire et celui de l'étape 1 ont servi : on les efface.
                for name in [SETUP_CSRF_COOKIE, SETUP_PASS_COOKIE] {
                    response.headers_mut().append(
                        header::SET_COOKIE,
                        set_cookie(&state, name, "", SETUP_PATH, 0),
                    );
                }
                response
            }
            Err(error) => internal_error(error),
        },
        // Erreur que la personne peut corriger : on remontre le formulaire.
        Ok(Err((status, message))) => setup_page(
            &state,
            &headers,
            status,
            email_typed,
            username_typed,
            Some(message),
        ),
        Err(error) => internal_error(error),
    }
}

/// Vérifie le code et les champs, crée le compte et lui donne le rôle d'admin.
/// Le `Result` imbriqué sépare les erreurs à montrer dans la page (intérieur) des
/// pannes du serveur (extérieur). `passed` : le cookie de l'étape 1 remplace le code.
async fn create_first_admin(
    state: &AppState,
    body: SetupForm,
    passed: bool,
) -> Result<Result<i64, (StatusCode, String)>, HubError> {
    let store = state.store();
    if store.has_admin().await? {
        return Ok(Err((
            StatusCode::CONFLICT,
            "Ce hub a déjà un admin : connectez-vous depuis la page de connexion.".to_string(),
        )));
    }
    let Some(expected) = &state.inner.config.setup_code else {
        return Ok(Err((
            StatusCode::FORBIDDEN,
            "Installation fermée.".to_string(),
        )));
    };
    // Même tolérance que les invitations : casse, tirets et espaces ignorés.
    if !passed
        && !same_token(
            &normalize_invite_code(expected),
            &normalize_invite_code(&body.code),
        )
    {
        tracing::info!("code d'installation refusé");
        return Ok(Err((
            StatusCode::FORBIDDEN,
            "Code d'installation incorrect. Il est écrit dans le journal du serveur au démarrage."
                .to_string(),
        )));
    }

    // Les erreurs de saisie (pseudo, e-mail, mot de passe) ont déjà un message
    // en français : on le montre tel quel.
    let checked = parse_username(&body.username).and_then(|username| {
        let email = normalize_email(&body.email)?;
        body.password.validate()?;
        Ok((username, email))
    });
    if !body.password.same_as(&body.password_confirm) {
        return Ok(Err((
            StatusCode::BAD_REQUEST,
            "Les deux mots de passe ne sont pas identiques.".to_string(),
        )));
    }
    let (username, email) = match checked {
        Ok(values) => values,
        Err(error) => return Ok(Err((StatusCode::BAD_REQUEST, error.to_string()))),
    };
    if store.email_identity(&email).await?.is_some() {
        return Ok(Err((
            StatusCode::CONFLICT,
            "Un compte existe déjà avec cette adresse. Donnez-lui le rôle d'admin sur le serveur : rafthub admin create <e-mail>."
                .to_string(),
        )));
    }
    if store
        .user_by_username(&username.normalized)
        .await?
        .is_some()
    {
        return Ok(Err((
            StatusCode::CONFLICT,
            "Ce pseudo est déjà pris.".to_string(),
        )));
    }

    let params = state.inner.config.hash_params;
    let password = body.password;
    let hash = blocking(move || hash_password(&password, &params)).await?;
    let user_id = store
        .create_account(
            &username.display,
            &username.normalized,
            &email,
            &hash,
            None,
            now_unix(),
        )
        .await?;
    store.set_role_by_email(&email, Role::Admin).await?;
    tracing::info!(user_id, "premier admin créé depuis la page d'installation");
    state
        .record_event_quietly(
            Some(user_id),
            EventKind::AccountCreated,
            Some(user_id),
            "premier admin, page d'installation",
        )
        .await;
    Ok(Ok(user_id))
}
