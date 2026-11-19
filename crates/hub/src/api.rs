//! Routes HTTP de l'API v1 du hub.
//!
//! | Méthode et chemin | Rôle |
//! | --- | --- |
//! | `GET /v1` | version de l'API |
//! | `POST /v1/accounts` | inscription (`email`, `password`, `username`) |
//! | `POST /v1/sessions` | connexion (`email`, `password`) → jeton |
//! | `DELETE /v1/sessions/current` | déconnexion (révoque le jeton) |
//! | `GET /v1/me` | compte connecté (avec `email_verified`) |
//! | `GET /v1/me/export` | toutes les données du compte, en JSON (RGPD) |
//! | `DELETE /v1/me` | supprime le compte (`password` si le compte en a un) |
//! | `POST /v1/email/verify` | vérifie l'adresse (`token` reçu par e-mail) |
//! | `GET /v1/email/verify?token=…` | même chose, depuis le lien du message (réponse en texte) |
//! | `POST /v1/email/verify/resend` | renvoie le message de vérification |
//! | `POST /v1/password/forgot` | envoie un code de réinitialisation (`email`) ; répond toujours 202 |
//! | `POST /v1/password/reset` | nouveau mot de passe (`token`, `password`) ; ferme toutes les sessions |
//! | `POST /v1/oauth/discord/start` | commence une connexion Discord (`return_port` facultatif) ; avec un jeton : liaison au compte |
//! | `GET /v1/oauth/discord/callback` | retour de Discord (navigateur), renvoie vers le launcher |
//! | `POST /v1/oauth/discord/finish` | finit la connexion (`state`, `secret`, `ticket`) |
//! | `POST /v1/oauth/discord/register` | crée le compte d'un nouveau joueur Discord (`username` en plus) |
//! | `DELETE /v1/me/identities/discord` | délie Discord (s'il reste un autre moyen de connexion) |
//! | `PUT /v1/presence` | sa présence (`status`, `game`) |
//! | `GET /v1/friends` | amis et leur présence |
//! | `DELETE /v1/friends/{username}` | retirer un ami |
//! | `GET /v1/friends/requests` | demandes reçues et envoyées |
//! | `POST /v1/friends/requests` | envoyer une demande (`username`) |
//! | `DELETE /v1/friends/requests/{username}` | annuler sa demande |
//! | `POST /v1/friends/requests/{username}/accept` | accepter |
//! | `POST /v1/friends/requests/{username}/decline` | refuser |
//! | `GET /v1/blocks` | joueurs bloqués |
//! | `POST /v1/blocks` | bloquer (`username`) : retire l'amitié et les demandes |
//! | `DELETE /v1/blocks/{username}` | lever un blocage |
//!
//! Inscription (`POST /v1/accounts` et `/v1/oauth/discord/register`) : selon le
//! réglage `registration_mode` (ADR 0036), ouverte, sur code d'invitation
//! (`invite_code` dans le corps) ou fermée. Un compte suspendu ne peut plus se
//! connecter et n'apparaît plus chez les autres joueurs.
//!
//! Les routes sauf `/v1`, l'inscription, la connexion, la vérification d'adresse et
//! le mot de passe oublié, et la connexion Discord demandent l'en-tête
//! `Authorization: Bearer <jeton>`.

use axum::extract::{DefaultBodyLimit, FromRequest, FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::console::EventKind;
use crate::discord::{code_challenge, Outcome, Pending, ReturnTo};
use crate::error::HubError;
use crate::identity::{normalize_email, normalize_username, parse_username};
use crate::mail::{password_reset_email, verification_email};
use crate::memory::{PresenceView, Status};
use crate::password::{hash_password, verify_password, Password};
use crate::session::{new_token, normalize_invite_code, token_hash};
use crate::settings::{RegistrationMode, Settings};
use crate::store::{AccountExport, TokenPurpose, UserRow, DISCORD_PROVIDER};
use crate::{blocking, now_unix, AppState};

/// Version de l'API, renvoyée par `GET /v1`.
pub const API_VERSION: u32 = 1;
/// Taille maximale d'un corps de requête : tous nos messages sont petits.
const MAX_BODY_BYTES: usize = 16 * 1024;
/// Longueur maximale du nom de jeu dans la présence.
const MAX_GAME_CHARS: usize = 64;

/// Construit le routeur du hub.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1", get(api_info))
        .route("/v1/accounts", post(register))
        .route("/v1/sessions", post(login))
        .route("/v1/sessions/current", delete(logout))
        .route("/v1/me", get(me).delete(delete_account))
        .route("/v1/me/export", get(export_account))
        .route(
            "/v1/email/verify",
            post(verify_email).get(verify_email_link),
        )
        .route("/v1/email/verify/resend", post(resend_verification))
        .route("/v1/password/forgot", post(forgot_password))
        .route("/v1/password/reset", post(reset_password))
        .route("/v1/oauth/discord/start", post(discord_start))
        .route("/v1/oauth/discord/callback", get(discord_callback))
        .route("/v1/oauth/discord/finish", post(discord_finish))
        .route("/v1/oauth/discord/register", post(discord_register))
        .route("/v1/me/identities/discord", delete(discord_unlink))
        .route("/v1/presence", put(set_presence))
        .route("/v1/friends", get(list_friends))
        .route("/v1/friends/{username}", delete(remove_friend))
        .route(
            "/v1/friends/requests",
            get(list_requests).post(send_request),
        )
        .route("/v1/friends/requests/{username}", delete(cancel_request))
        .route(
            "/v1/friends/requests/{username}/accept",
            post(accept_request),
        )
        .route(
            "/v1/friends/requests/{username}/decline",
            post(decline_request),
        )
        .route("/v1/blocks", get(list_blocks).post(block_player))
        .route("/v1/blocks/{username}", delete(unblock_player))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

// ---------- Types échangés ----------

/// Corps de l'inscription. `Debug` est sûr : celui de [`Password`] est masqué.
#[derive(Debug, Deserialize)]
pub struct RegisterBody {
    pub email: String,
    pub password: Password,
    pub username: String,
    /// Code d'invitation, demandé seulement en mode `invite`.
    #[serde(default)]
    pub invite_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub email: String,
    pub password: Password,
}

/// Confirmation de la suppression du compte : le mot de passe, si le compte en a un.
#[derive(Debug, Deserialize)]
pub struct DeleteAccountBody {
    #[serde(default)]
    pub password: Option<Password>,
}

/// Export RGPD : les données du compte, plus la date et la version du format.
#[derive(Debug, Serialize)]
pub struct ExportView {
    pub format_version: u32,
    pub exported_at: i64,
    pub service: &'static str,
    pub account: AccountExport,
    /// La présence n'est jamais écrite en base ; elle est rappelée ici pour être complet.
    pub presence: PresenceView,
}

/// Version du format de l'export RGPD.
pub const EXPORT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
pub struct UsernameBody {
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct PresenceBody {
    pub status: Status,
    #[serde(default)]
    pub game: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AccountView {
    pub user_id: i64,
    pub username: String,
}

#[derive(Debug, Serialize)]
pub struct SessionView {
    pub token: String,
    pub user_id: i64,
    pub username: String,
    /// Fin de validité, en secondes Unix.
    pub expires_at: i64,
}

#[derive(Debug, Serialize)]
pub struct MeView {
    pub user_id: i64,
    pub username: String,
    pub email: Option<String>,
    /// Vrai quand l'adresse a été confirmée par le lien du message.
    pub email_verified: bool,
    /// Vrai quand un compte Discord est lié.
    pub discord_linked: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct DiscordStartBody {
    /// Port du petit serveur local du launcher ; sans lui, retour par `raftup://`.
    #[serde(default)]
    pub return_port: Option<u16>,
}

#[derive(Debug, Serialize)]
pub struct DiscordStartView {
    /// Adresse à ouvrir dans le navigateur du système.
    pub authorize_url: String,
    pub state: String,
    /// Secret à garder par le launcher, redemandé à la fin.
    pub secret: String,
}

/// Fin de la connexion Discord. `Debug` masqué : secrets.
#[derive(Deserialize)]
pub struct DiscordFinishBody {
    pub state: String,
    pub secret: String,
    pub ticket: String,
    /// Seulement pour `/register` : le pseudo choisi.
    #[serde(default)]
    pub username: Option<String>,
    /// Seulement pour `/register`, en mode `invite` : le code d'invitation.
    #[serde(default)]
    pub invite_code: Option<String>,
}

impl std::fmt::Debug for DiscordFinishBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiscordFinishBody(***)")
    }
}

/// Résultat de `finish` / `register` : `logged_in` (avec la session), `linked`,
/// ou `needs_username` (avec un pseudo proposé, s'il est libre).
#[derive(Debug, Serialize)]
pub struct DiscordFinishView {
    pub status: &'static str,
    /// `flatten` : les champs de la session sont au même niveau que `status`.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_username: Option<String>,
    /// Avec `needs_username` : le hub demande aussi un code d'invitation.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub invite_required: bool,
}

/// Jeton reçu par e-mail. `Debug` masqué : le jeton est un secret.
#[derive(Deserialize)]
pub struct TokenBody {
    pub token: String,
}

impl std::fmt::Debug for TokenBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenBody(***)")
    }
}

#[derive(Debug, Deserialize)]
pub struct EmailBody {
    pub email: String,
}

/// Nouveau mot de passe après « mot de passe oublié ».
#[derive(Deserialize)]
pub struct ResetBody {
    pub token: String,
    pub password: Password,
}

impl std::fmt::Debug for ResetBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResetBody(***)")
    }
}

#[derive(Debug, Serialize)]
pub struct FriendView {
    pub username: String,
    pub presence: PresenceView,
}

#[derive(Debug, Serialize)]
pub struct PlayerView {
    pub username: String,
}

#[derive(Debug, Serialize)]
pub struct RequestsView {
    pub incoming: Vec<PlayerView>,
    pub outgoing: Vec<PlayerView>,
}

/// Résultat d'une demande d'ami : `pending`, ou `accepted` si l'autre joueur nous
/// avait déjà envoyé une demande (les deux demandes se croisent).
#[derive(Debug, Serialize)]
pub struct FriendRequestResult {
    pub status: &'static str,
}

fn players(rows: Vec<UserRow>) -> Vec<PlayerView> {
    rows.into_iter()
        .map(|row| PlayerView {
            username: row.username,
        })
        .collect()
}

// ---------- Lecture du corps JSON ----------

/// Comme `Json<T>` d'axum, mais une erreur de lecture répond dans le format JSON
/// du hub (`{"error": "bad_request", ...}`) au lieu d'un texte brut.
pub struct ApiJson<T>(pub T);

// `S: Send + Sync` : axum exige que l'état puisse passer d'un thread à l'autre.
// `DeserializeOwned` : `T` se lit depuis le JSON sans emprunter le corps de la requête.
impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = HubError;

    async fn from_request(request: axum::extract::Request, state: &S) -> Result<Self, HubError> {
        match Json::<T>::from_request(request, state).await {
            Ok(Json(value)) => Ok(ApiJson(value)),
            // Le texte de serde ne recopie pas les chaînes valides, donc jamais
            // un mot de passe.
            Err(rejection) => Err(HubError::BadRequest(rejection.body_text())),
        }
    }
}

// ---------- Authentification ----------

/// Compte connecté, tiré de l'en-tête `Authorization: Bearer <jeton>`.
///
/// Mettre `user: AuthUser` dans les paramètres d'une route suffit à la protéger :
/// axum appelle `from_request_parts` avant la route et répond 401 en cas d'échec.
pub struct AuthUser {
    pub user_id: i64,
    token_hash: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = HubError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, HubError> {
        optional_auth(&parts.headers, state)
            .await?
            .ok_or(HubError::Unauthorized)
    }
}

/// Session facultative : `Ok(None)` sans en-tête `Authorization`, une erreur si
/// l'en-tête est là mais le jeton faux ou expiré.
async fn optional_auth(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<Option<AuthUser>, HubError> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    let token = value
        .to_str()
        .ok()
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or(HubError::Unauthorized)?;
    let hash = token_hash(token);
    let user_id = state
        .store()
        .session_user(&hash, now_unix())
        .await?
        .ok_or(HubError::Unauthorized)?;
    Ok(Some(AuthUser {
        user_id,
        token_hash: hash,
    }))
}

// ---------- Routes ----------

async fn api_info() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "service": "raftup-hub", "api_version": API_VERSION }))
}

async fn register(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<RegisterBody>,
) -> Result<(StatusCode, Json<AccountView>), HubError> {
    // Mode d'inscription d'abord : sur un hub fermé, inutile de lire le reste.
    let settings = state.settings().await?;
    let invite = registration_invite(&state, &settings, body.invite_code.as_deref()).await?;
    let username = parse_username(&body.username)?;
    let email = normalize_email(&body.email)?;
    body.password.validate()?;

    // Vérifications rapides avant le hachage, qui est lent. La base garde le dernier
    // mot grâce à ses contraintes d'unicité (deux inscriptions simultanées).
    let store = state.store();
    if store
        .user_by_username(&username.normalized)
        .await?
        .is_some()
    {
        return Err(HubError::UsernameTaken);
    }
    if store.email_identity(&email).await?.is_some() {
        return Err(HubError::EmailTaken);
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
            invite.as_deref(),
            now_unix(),
        )
        .await?;
    // Ni l'adresse ni le mot de passe dans le journal (NF09, RGPD).
    tracing::info!(user_id, "compte créé");
    let detail = if invite.is_some() {
        "e-mail, avec invitation"
    } else {
        "e-mail"
    };
    state
        .record_event_quietly(
            Some(user_id),
            EventKind::AccountCreated,
            Some(user_id),
            detail,
        )
        .await;
    send_verification(&state, &settings, user_id, &email, &username.display).await?;
    Ok((
        StatusCode::CREATED,
        Json(AccountView {
            user_id,
            username: username.display,
        }),
    ))
}

async fn login(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<LoginBody>,
) -> Result<Json<SessionView>, HubError> {
    // Adresse mal formée : même réponse qu'un mauvais mot de passe.
    let email = normalize_email(&body.email).map_err(|_| HubError::InvalidCredentials)?;
    let limiter = &state.inner.login_limiter;
    if limiter.is_blocked(&email) {
        return Err(HubError::TooManyAttempts);
    }

    let identity = state.store().email_identity(&email).await?;
    // Adresse inconnue : on vérifie quand même contre un hash bidon, pour que la
    // réponse prenne le même temps.
    let (user_id, phc) = match identity {
        Some((user_id, phc)) => (Some(user_id), phc),
        None => (None, state.inner.dummy_hash.clone()),
    };
    let password = body.password;
    let matches = blocking(move || verify_password(&password, &phc)).await?;

    let user_id = match (user_id, matches) {
        (Some(user_id), true) => user_id,
        _ => {
            limiter.hit(&email);
            tracing::info!("échec de connexion");
            return Err(HubError::InvalidCredentials);
        }
    };
    limiter.reset(&email);

    Ok(Json(open_session(&state, user_id).await?))
}

/// Vérifie qu'une inscription est permise par le mode d'inscription du hub, et
/// renvoie l'empreinte du code d'invitation à consommer (mode `invite`).
///
/// Le code est seulement vérifié ici (réponse rapide) ; il est consommé dans la
/// transaction qui crée le compte.
async fn registration_invite(
    state: &AppState,
    settings: &Settings,
    code: Option<&str>,
) -> Result<Option<String>, HubError> {
    match settings.registration_mode {
        RegistrationMode::Open => Ok(None),
        RegistrationMode::Closed => Err(HubError::RegistrationClosed),
        RegistrationMode::Invite => {
            let code = code
                .map(normalize_invite_code)
                .filter(|c| !c.is_empty())
                .ok_or(HubError::InviteRequired)?;
            let hash = token_hash(&code);
            if !state.store().invite_usable(&hash, now_unix()).await? {
                return Err(HubError::InvalidInvite);
            }
            Ok(Some(hash))
        }
    }
}

/// Ouvre une session pour un compte (connexion par mot de passe ou Discord).
/// Un compte suspendu est refusé : l'appelant a déjà vérifié le mot de passe (ou
/// Discord), donc la réponse n'apprend rien à quelqu'un qui ne le connaît pas.
async fn open_session(state: &AppState, user_id: i64) -> Result<SessionView, HubError> {
    let user = state
        .store()
        .user_by_id(user_id)
        .await?
        .ok_or(HubError::InvalidCredentials)?;
    if state.store().is_suspended(user_id).await? {
        tracing::info!(user_id, "connexion refusée : compte suspendu");
        return Err(HubError::AccountSuspended);
    }
    let token = new_token();
    let now = now_unix();
    let lifetime = i64::try_from(state.inner.config.session_lifetime.as_secs()).unwrap_or(i64::MAX);
    let expires_at = now.saturating_add(lifetime);
    state
        .store()
        .insert_session(&token_hash(&token), user_id, now, expires_at)
        .await?;
    tracing::info!(user_id, "session ouverte");
    Ok(SessionView {
        token,
        user_id,
        username: user.username,
        expires_at,
    })
}

async fn logout(State(state): State<AppState>, user: AuthUser) -> Result<StatusCode, HubError> {
    state.store().delete_session(&user.token_hash).await?;
    state.inner.presence.clear(user.user_id);
    tracing::info!(user_id = user.user_id, "session révoquée");
    Ok(StatusCode::NO_CONTENT)
}

async fn me(State(state): State<AppState>, user: AuthUser) -> Result<Json<MeView>, HubError> {
    let row = state
        .store()
        .user_by_id(user.user_id)
        .await?
        .ok_or(HubError::Unauthorized)?;
    let status = state.store().email_status(user.user_id).await?;
    let email_verified = status.as_ref().is_some_and(|(_, verified)| *verified);
    let providers = state.store().identity_providers(user.user_id).await?;
    Ok(Json(MeView {
        user_id: row.id,
        username: row.username,
        email: status.map(|(email, _)| email),
        email_verified,
        discord_linked: providers.iter().any(|p| p == DISCORD_PROVIDER),
    }))
}

async fn set_presence(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<PresenceBody>,
) -> Result<StatusCode, HubError> {
    let game = match (body.status, body.game) {
        (Status::InGame, Some(game)) => {
            let game = game.trim().to_string();
            if game.is_empty() || game.chars().count() > MAX_GAME_CHARS {
                return Err(HubError::InvalidPresence("nom de jeu vide ou trop long"));
            }
            if game.chars().any(char::is_control) {
                return Err(HubError::InvalidPresence("caractère de contrôle"));
            }
            Some(game)
        }
        // Le nom du jeu n'a de sens qu'en partie ; ailleurs il est ignoré.
        _ => None,
    };
    state.inner.presence.set(
        user.user_id,
        PresenceView {
            status: body.status,
            game,
        },
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn list_friends(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<FriendView>>, HubError> {
    let friends = state.store().friends_of(user.user_id).await?;
    // La présence n'est visible que par les amis : on ne la montre qu'ici.
    let views = friends
        .into_iter()
        .map(|friend| FriendView {
            presence: state.inner.presence.get(friend.id),
            username: friend.username,
        })
        .collect();
    Ok(Json(views))
}

/// Cherche un autre joueur par son pseudo. Un compte suspendu répond comme un
/// pseudo inconnu : il n'existe plus pour les autres joueurs (ADR 0036).
async fn other_player(state: &AppState, me: i64, username: &str) -> Result<UserRow, HubError> {
    let other = state
        .store()
        .user_by_username(&normalize_username(username))
        .await?
        .ok_or(HubError::UserNotFound)?;
    if state.store().is_suspended(other.id).await? {
        return Err(HubError::UserNotFound);
    }
    if other.id == me {
        return Err(HubError::SelfFriend);
    }
    Ok(other)
}

async fn remove_friend(
    State(state): State<AppState>,
    user: AuthUser,
    Path(username): Path<String>,
) -> Result<StatusCode, HubError> {
    let other = other_player(&state, user.user_id, &username).await?;
    if !state
        .store()
        .remove_friendship(user.user_id, other.id)
        .await?
    {
        return Err(HubError::FriendNotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_requests(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<RequestsView>, HubError> {
    let incoming = state.store().incoming_requests(user.user_id).await?;
    let outgoing = state.store().outgoing_requests(user.user_id).await?;
    Ok(Json(RequestsView {
        incoming: players(incoming),
        outgoing: players(outgoing),
    }))
}

async fn send_request(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<UsernameBody>,
) -> Result<(StatusCode, Json<FriendRequestResult>), HubError> {
    // Limite par compte : les pseudos se devinent facilement (ADR 0002).
    if !state.inner.friend_limiter.hit(&user.user_id.to_string()) {
        return Err(HubError::TooManyAttempts);
    }
    let other = other_player(&state, user.user_id, &body.username).await?;
    let store = state.store();
    // Blocages (ADR 0033) : celui qui a bloqué le sait ; celui qui est bloqué reçoit
    // la même réponse qu'un pseudo inconnu, pour ne pas apprendre qu'il l'est.
    if store.has_blocked(user.user_id, other.id).await? {
        return Err(HubError::YouBlocked);
    }
    if store.has_blocked(other.id, user.user_id).await? {
        return Err(HubError::UserNotFound);
    }
    if store.are_friends(user.user_id, other.id).await? {
        return Err(HubError::AlreadyFriends);
    }
    if store.request_exists(user.user_id, other.id).await? {
        return Err(HubError::RequestExists);
    }
    let now = now_unix();
    if store.request_exists(other.id, user.user_id).await? {
        // L'autre joueur nous avait déjà demandé : les deux sont d'accord.
        store.make_friends(user.user_id, other.id, now).await?;
        return Ok((
            StatusCode::OK,
            Json(FriendRequestResult { status: "accepted" }),
        ));
    }
    store.insert_request(user.user_id, other.id, now).await?;
    Ok((
        StatusCode::CREATED,
        Json(FriendRequestResult { status: "pending" }),
    ))
}

async fn cancel_request(
    State(state): State<AppState>,
    user: AuthUser,
    Path(username): Path<String>,
) -> Result<StatusCode, HubError> {
    let other = other_player(&state, user.user_id, &username).await?;
    if !state.store().delete_request(user.user_id, other.id).await? {
        return Err(HubError::RequestNotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn accept_request(
    State(state): State<AppState>,
    user: AuthUser,
    Path(username): Path<String>,
) -> Result<StatusCode, HubError> {
    let other = other_player(&state, user.user_id, &username).await?;
    if !state.store().request_exists(other.id, user.user_id).await? {
        return Err(HubError::RequestNotFound);
    }
    state
        .store()
        .make_friends(user.user_id, other.id, now_unix())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn decline_request(
    State(state): State<AppState>,
    user: AuthUser,
    Path(username): Path<String>,
) -> Result<StatusCode, HubError> {
    let other = other_player(&state, user.user_id, &username).await?;
    if !state.store().delete_request(other.id, user.user_id).await? {
        return Err(HubError::RequestNotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Blocages ----------

async fn list_blocks(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<PlayerView>>, HubError> {
    let rows = state.store().blocked_by(user.user_id).await?;
    Ok(Json(players(rows)))
}

/// Bloque un joueur. Effets : plus d'amitié (donc plus de présence visible dans
/// les deux sens), plus de demandes entre les deux, et plus de nouvelle demande
/// tant que le blocage dure.
async fn block_player(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<UsernameBody>,
) -> Result<StatusCode, HubError> {
    let other = other_player(&state, user.user_id, &body.username)
        .await
        .map_err(|e| match e {
            HubError::SelfFriend => HubError::SelfBlock,
            other => other,
        })?;
    state
        .store()
        .block(user.user_id, other.id, now_unix())
        .await?;
    tracing::info!(user_id = user.user_id, "joueur bloqué");
    Ok(StatusCode::NO_CONTENT)
}

async fn unblock_player(
    State(state): State<AppState>,
    user: AuthUser,
    Path(username): Path<String>,
) -> Result<StatusCode, HubError> {
    let other = other_player(&state, user.user_id, &username)
        .await
        .map_err(|e| match e {
            HubError::SelfFriend => HubError::BlockNotFound,
            other => other,
        })?;
    if !state.store().unblock(user.user_id, other.id).await? {
        return Err(HubError::BlockNotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------- RGPD ----------

/// Droit d'accès : tout ce que le hub garde sur le compte, en un document JSON.
/// Ni hash de mot de passe ni empreinte de session : ils ne disent rien au joueur
/// et ne doivent pas circuler.
async fn export_account(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<ExportView>, HubError> {
    let account = state
        .store()
        .export_account(user.user_id)
        .await?
        .ok_or(HubError::Unauthorized)?;
    tracing::info!(user_id = user.user_id, "export des données du compte");
    state
        .record_event_quietly(
            Some(user.user_id),
            EventKind::GdprExport,
            Some(user.user_id),
            "",
        )
        .await;
    Ok(Json(ExportView {
        format_version: EXPORT_FORMAT_VERSION,
        exported_at: now_unix(),
        service: "raftup-hub",
        account,
        presence: state.inner.presence.get(user.user_id),
    }))
}

/// Droit à l'effacement : supprime le compte et tout ce qui s'y rattache.
///
/// Un compte avec mot de passe doit le redonner : une session volée ne suffit pas
/// à effacer un compte. Les échecs comptent dans la même limite que la connexion.
async fn delete_account(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<DeleteAccountBody>,
) -> Result<StatusCode, HubError> {
    let store = state.store();
    if let Some(phc) = store.password_hash_of(user.user_id).await? {
        let limiter = &state.inner.login_limiter;
        let key = format!("delete:{}", user.user_id);
        if limiter.is_blocked(&key) {
            return Err(HubError::TooManyAttempts);
        }
        let password = body.password.ok_or(HubError::InvalidCredentials)?;
        let matches = blocking(move || verify_password(&password, &phc)).await?;
        if !matches {
            limiter.hit(&key);
            return Err(HubError::InvalidCredentials);
        }
        limiter.reset(&key);
    }
    store.delete_account(user.user_id).await?;
    state.inner.presence.clear(user.user_id);
    tracing::info!(user_id = user.user_id, "compte supprimé");
    // Le compte n'existe plus : ni acteur ni cible, pour qu'aucune ligne du journal
    // ne le désigne encore (droit à l'effacement).
    state
        .record_event_quietly(
            None,
            EventKind::GdprDelete,
            None,
            "compte supprimé par son joueur",
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- E-mails : vérification d'adresse, mot de passe oublié ----------

/// Durée en secondes, bornée pour tenir dans un `i64`.
fn seconds(duration: std::time::Duration) -> i64 {
    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
}

/// Crée un jeton à usage unique, garde son empreinte en base et renvoie le jeton
/// en clair (il ne part que dans l'e-mail).
async fn new_email_token(
    state: &AppState,
    user_id: i64,
    purpose: TokenPurpose,
    lifetime: std::time::Duration,
) -> Result<String, HubError> {
    let token = new_token();
    let now = now_unix();
    state
        .store()
        .insert_email_token(
            &token_hash(&token),
            user_id,
            purpose,
            now,
            now.saturating_add(seconds(lifetime)),
        )
        .await?;
    Ok(token)
}

/// Envoie le message de vérification d'adresse. Le lien part de l'adresse publique
/// des réglages (ADR 0036).
async fn send_verification(
    state: &AppState,
    settings: &Settings,
    user_id: i64,
    email: &str,
    username: &str,
) -> Result<(), HubError> {
    let lifetime = state.inner.config.email_verify_lifetime;
    let token = new_email_token(state, user_id, TokenPurpose::VerifyEmail, lifetime).await?;
    let link = format!("{}/v1/email/verify?token={token}", settings.public_url);
    state
        .send_mail(verification_email(
            email,
            username,
            &link,
            lifetime.as_secs() / 3600,
        ))
        .await
}

/// Vérifie l'adresse avec le jeton du message.
async fn check_verification_token(state: &AppState, token: &str) -> Result<i64, HubError> {
    let user_id = state
        .store()
        .consume_email_token(
            &token_hash(token.trim()),
            TokenPurpose::VerifyEmail,
            now_unix(),
        )
        .await?
        .ok_or(HubError::InvalidToken)?;
    state.store().mark_email_verified(user_id).await?;
    tracing::info!(user_id, "adresse e-mail vérifiée");
    Ok(user_id)
}

async fn verify_email(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<TokenBody>,
) -> Result<StatusCode, HubError> {
    check_verification_token(&state, &body.token).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Lien cliqué dans le message : le navigateur affiche une phrase, pas du JSON.
async fn verify_email_link(
    State(state): State<AppState>,
    Query(query): Query<std::collections::BTreeMap<String, String>>,
) -> (StatusCode, &'static str) {
    let Some(token) = query.get("token") else {
        return (StatusCode::BAD_REQUEST, "Lien incomplet.");
    };
    match check_verification_token(&state, token).await {
        Ok(_) => (
            StatusCode::OK,
            "Adresse vérifiée. Vous pouvez fermer cette page et revenir au launcher Raftup.",
        ),
        Err(HubError::InvalidToken) => (
            StatusCode::BAD_REQUEST,
            "Lien invalide, expiré ou déjà utilisé. Demandez un nouveau message depuis le launcher.",
        ),
        Err(error) => {
            tracing::error!(%error, "vérification d'adresse");
            (StatusCode::INTERNAL_SERVER_ERROR, "Erreur interne du hub.")
        }
    }
}

async fn resend_verification(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<StatusCode, HubError> {
    let (email, verified) =
        state
            .store()
            .email_status(user.user_id)
            .await?
            .ok_or(HubError::BadRequest(
                "ce compte n'a pas d'adresse e-mail".to_string(),
            ))?;
    if verified {
        return Err(HubError::AlreadyVerified);
    }
    if !state
        .inner
        .mail_limiter
        .hit(&format!("verify:{}", user.user_id))
    {
        return Err(HubError::TooManyAttempts);
    }
    let username = state
        .store()
        .user_by_id(user.user_id)
        .await?
        .ok_or(HubError::Unauthorized)?
        .username;
    let settings = state.settings().await?;
    send_verification(&state, &settings, user.user_id, &email, &username).await?;
    Ok(StatusCode::ACCEPTED)
}

/// « Mot de passe oublié ». Répond toujours `202 Accepted`, que l'adresse ait un
/// compte ou non, et même au-delà de la limite d'envoi : la réponse ne dit pas
/// quelles adresses sont inscrites.
async fn forgot_password(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<EmailBody>,
) -> Result<StatusCode, HubError> {
    let email = normalize_email(&body.email)?;
    if !state.inner.mail_limiter.hit(&format!("reset:{email}")) {
        tracing::info!("mot de passe oublié : limite d'envoi atteinte");
        return Ok(StatusCode::ACCEPTED);
    }
    if let Some((user_id, _)) = state.store().email_identity(&email).await? {
        let username = state
            .store()
            .user_by_id(user_id)
            .await?
            .map(|row| row.username)
            .unwrap_or_default();
        let lifetime = state.inner.config.password_reset_lifetime;
        let token = new_email_token(&state, user_id, TokenPurpose::ResetPassword, lifetime).await?;
        state
            .send_mail(password_reset_email(
                &email,
                &username,
                &token,
                lifetime.as_secs() / 60,
            ))
            .await?;
        tracing::info!(user_id, "code de réinitialisation envoyé");
    }
    Ok(StatusCode::ACCEPTED)
}

async fn reset_password(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ResetBody>,
) -> Result<StatusCode, HubError> {
    // Mot de passe vérifié d'abord : un mot de passe trop court ne gâche pas le code.
    body.password.validate()?;
    let user_id = state
        .store()
        .consume_email_token(
            &token_hash(body.token.trim()),
            TokenPurpose::ResetPassword,
            now_unix(),
        )
        .await?
        .ok_or(HubError::InvalidToken)?;
    let params = state.inner.config.hash_params;
    let password = body.password;
    let hash = blocking(move || hash_password(&password, &params)).await?;
    state.store().reset_password(user_id, &hash).await?;
    state.inner.presence.clear(user_id);
    tracing::info!(user_id, "mot de passe réinitialisé, sessions fermées");
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Connexion Discord (OAuth 2.0 + PKCE) ----------

/// Client Discord du moment (relu depuis les réglages, ADR 0036).
async fn discord_api(state: &AppState) -> Result<std::sync::Arc<dyn crate::DiscordApi>, HubError> {
    state.discord().await?.ok_or(HubError::DiscordDisabled)
}

/// Étape 1 : commence une connexion. Avec une session : liaison de Discord à ce compte.
async fn discord_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<DiscordStartBody>,
) -> Result<Json<DiscordStartView>, HubError> {
    let api = discord_api(&state).await?;
    let link_user = optional_auth(&headers, &state).await?.map(|u| u.user_id);
    let return_to = match body.return_port {
        Some(0) => return Err(HubError::BadRequest("return_port invalide".to_string())),
        Some(port) => ReturnTo::Loopback(port),
        None => ReturnTo::DeepLink,
    };
    // Trois valeurs aléatoires : `state` (dans les adresses), `secret` (gardé par
    // le launcher) et le vérificateur PKCE (gardé par le hub).
    let oauth_state = new_token();
    let secret = new_token();
    let code_verifier = new_token();
    let authorize_url = api.authorize_url(&oauth_state, &code_challenge(&code_verifier));
    let pending = Pending {
        secret_hash: token_hash(&secret),
        code_verifier,
        link_user,
        return_to,
        created: std::time::Instant::now(),
        outcome: None,
    };
    if !state
        .inner
        .oauth_pending
        .insert(oauth_state.clone(), pending)
    {
        return Err(HubError::TooManyAttempts);
    }
    Ok(Json(DiscordStartView {
        authorize_url,
        state: oauth_state,
        secret,
    }))
}

/// Étape 2 : retour de Discord dans le navigateur : échange du code, puis renvoi vers le
/// launcher avec un `ticket` (même en cas de refus, pour que le launcher l'apprenne).
async fn discord_callback(
    State(state): State<AppState>,
    Query(query): Query<std::collections::BTreeMap<String, String>>,
) -> Response {
    let text_error = |message: &'static str| (StatusCode::BAD_REQUEST, message).into_response();
    let Some(oauth_state) = query.get("state") else {
        return text_error("Lien incomplet.");
    };
    let Some(pending) = state.inner.oauth_pending.get(oauth_state) else {
        return text_error("Connexion inconnue ou expirée : recommencez depuis le launcher.");
    };
    if pending.outcome.is_some() {
        return text_error("Cette connexion est déjà revenue de Discord.");
    }
    let outcome = match (query.get("code"), discord_api(&state).await) {
        (Some(code), Ok(api)) if !query.contains_key("error") => {
            let code = code.clone();
            let verifier = pending.code_verifier.clone();
            match blocking(move || Ok(api.identify(&code, &verifier))).await {
                Ok(Ok(user)) => Outcome::Identified(user),
                Ok(Err(crate::discord::DiscordError::Refused)) => Outcome::Refused,
                _ => Outcome::Unavailable,
            }
        }
        // `error=access_denied` : le joueur a dit non chez Discord.
        _ => Outcome::Refused,
    };
    let ticket = new_token();
    if !state
        .inner
        .oauth_pending
        .set_outcome(oauth_state, outcome, token_hash(&ticket))
    {
        return text_error("Cette connexion est déjà revenue de Discord.");
    }
    Redirect::to(&pending.return_to.url(oauth_state, &ticket)).into_response()
}

/// Vérifie `state`, `secret` et `ticket`, et renvoie l'identité Discord obtenue.
/// Un refus ou une panne de Discord termine la connexion.
fn checked_discord_user(
    state: &AppState,
    body: &DiscordFinishBody,
) -> Result<(Pending, crate::DiscordUser), HubError> {
    let pending = state
        .inner
        .oauth_pending
        .get(&body.state)
        .ok_or(HubError::InvalidToken)?;
    if token_hash(&body.secret) != pending.secret_hash {
        return Err(HubError::InvalidToken);
    }
    let Some((outcome, ticket_hash)) = pending.outcome.clone() else {
        return Err(HubError::InvalidToken);
    };
    if token_hash(&body.ticket) != ticket_hash {
        return Err(HubError::InvalidToken);
    }
    match outcome {
        Outcome::Identified(user) => Ok((pending, user)),
        Outcome::Refused => {
            state.inner.oauth_pending.remove(&body.state);
            Err(HubError::DiscordRefused)
        }
        Outcome::Unavailable => {
            state.inner.oauth_pending.remove(&body.state);
            Err(HubError::DiscordUnavailable)
        }
    }
}

/// Pseudo proposé à partir du nom Discord, s'il respecte les règles et est libre.
async fn suggested_username(
    state: &AppState,
    user: &crate::DiscordUser,
) -> Result<Option<String>, HubError> {
    for candidate in [user.global_name.as_deref(), Some(user.username.as_str())]
        .into_iter()
        .flatten()
    {
        if let Ok(name) = parse_username(candidate) {
            if state
                .store()
                .user_by_username(&name.normalized)
                .await?
                .is_none()
            {
                return Ok(Some(name.display));
            }
        }
    }
    Ok(None)
}

/// Étape 3 : fin de la connexion : session, liaison, ou pseudo à choisir.
async fn discord_finish(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<DiscordFinishBody>,
) -> Result<Json<DiscordFinishView>, HubError> {
    let (pending, discord) = checked_discord_user(&state, &body)?;
    let store = state.store();
    let owner = store.identity_user(DISCORD_PROVIDER, &discord.id).await?;

    if let Some(user_id) = pending.link_user {
        // Liaison à un compte déjà connecté. La connexion en attente est retirée
        // quoi qu'il arrive : une liaison ne se retente pas avec le même ticket.
        state.inner.oauth_pending.remove(&body.state);
        match owner {
            // Déjà lié à ce compte : rien à faire.
            Some(owner) if owner == user_id => {}
            Some(_) => return Err(HubError::DiscordTaken),
            None => {
                store
                    .link_identity(user_id, DISCORD_PROVIDER, &discord.id, now_unix())
                    .await?
            }
        }
        tracing::info!(user_id, "compte Discord lié");
        return Ok(Json(DiscordFinishView {
            status: "linked",
            session: None,
            suggested_username: None,
            invite_required: false,
        }));
    }

    match owner {
        Some(user_id) => {
            state.inner.oauth_pending.remove(&body.state);
            Ok(Json(DiscordFinishView {
                status: "logged_in",
                session: Some(open_session(&state, user_id).await?),
                suggested_username: None,
                invite_required: false,
            }))
        }
        // Nouveau joueur : la connexion reste ouverte le temps de choisir un pseudo
        // (et le code d'invitation en mode `invite`). Hub fermé : on s'arrête là.
        None => {
            let mode = state.settings().await?.registration_mode;
            if mode == RegistrationMode::Closed {
                state.inner.oauth_pending.remove(&body.state);
                return Err(HubError::RegistrationClosed);
            }
            Ok(Json(DiscordFinishView {
                status: "needs_username",
                session: None,
                suggested_username: suggested_username(&state, &discord).await?,
                invite_required: mode == RegistrationMode::Invite,
            }))
        }
    }
}

/// Crée le compte d'un nouveau joueur venu par Discord, avec le pseudo choisi.
async fn discord_register(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<DiscordFinishBody>,
) -> Result<(StatusCode, Json<DiscordFinishView>), HubError> {
    let (pending, discord) = checked_discord_user(&state, &body)?;
    if pending.link_user.is_some() {
        return Err(HubError::BadRequest(
            "connexion commencée pour une liaison : utiliser /finish".to_string(),
        ));
    }
    let store = state.store();
    // Compte créé entre-temps (deux onglets) : simple connexion.
    if let Some(user_id) = store.identity_user(DISCORD_PROVIDER, &discord.id).await? {
        state.inner.oauth_pending.remove(&body.state);
        let session = open_session(&state, user_id).await?;
        return Ok((
            StatusCode::OK,
            Json(DiscordFinishView {
                status: "logged_in",
                session: Some(session),
                suggested_username: None,
                invite_required: false,
            }),
        ));
    }
    // Code d'invitation faux ou manquant : la connexion reste ouverte, le joueur
    // peut corriger et réessayer.
    let settings = state.settings().await?;
    let invite = registration_invite(&state, &settings, body.invite_code.as_deref()).await?;
    let wanted = body
        .username
        .as_deref()
        .ok_or(HubError::InvalidUsername("pseudo manquant"))?;
    let username = parse_username(wanted)?;
    // Pseudo pris : la connexion reste ouverte, le joueur en essaie un autre.
    let user_id = store
        .create_account_with_identity(
            &username.display,
            &username.normalized,
            DISCORD_PROVIDER,
            &discord.id,
            invite.as_deref(),
            now_unix(),
        )
        .await?;
    state.inner.oauth_pending.remove(&body.state);
    tracing::info!(user_id, "compte créé par Discord");
    let detail = if invite.is_some() {
        "Discord, avec invitation"
    } else {
        "Discord"
    };
    state
        .record_event_quietly(
            Some(user_id),
            EventKind::AccountCreated,
            Some(user_id),
            detail,
        )
        .await;
    let session = open_session(&state, user_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(DiscordFinishView {
            status: "logged_in",
            session: Some(session),
            suggested_username: None,
            invite_required: false,
        }),
    ))
}

/// Délie Discord, sauf si c'est le dernier moyen de se connecter au compte.
async fn discord_unlink(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<StatusCode, HubError> {
    let providers = state.store().identity_providers(user.user_id).await?;
    if !providers.iter().any(|p| p == DISCORD_PROVIDER) {
        return Err(HubError::IdentityNotFound);
    }
    if providers.len() == 1 {
        return Err(HubError::LastIdentity);
    }
    state
        .store()
        .unlink_identity(user.user_id, DISCORD_PROVIDER)
        .await?;
    tracing::info!(user_id = user.user_id, "compte Discord délié");
    Ok(StatusCode::NO_CONTENT)
}
