//! Routes HTTP de l'API v1 du hub.
//!
//! | Méthode et chemin | Rôle |
//! | --- | --- |
//! | `GET /v1` | version de l'API |
//! | `POST /v1/accounts` | inscription (`email`, `password`, `username`) |
//! | `POST /v1/sessions` | connexion (`email`, `password`) → jeton |
//! | `DELETE /v1/sessions/current` | déconnexion (révoque le jeton) |
//! | `GET /v1/me` | compte connecté |
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
//! Les routes sauf `/v1`, l'inscription et la connexion demandent l'en-tête
//! `Authorization: Bearer <jeton>`.

use axum::extract::{DefaultBodyLimit, FromRequest, FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{header, StatusCode};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::HubError;
use crate::identity::{normalize_email, normalize_username, parse_username};
use crate::memory::{PresenceView, Status};
use crate::password::{hash_password, verify_password, Password};
use crate::session::{new_token, token_hash};
use crate::store::UserRow;
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
        .route("/v1/me", get(me))
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
}

#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub email: String,
    pub password: Password,
}

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
        let header_value = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(HubError::Unauthorized)?;
        let token = header_value
            .strip_prefix("Bearer ")
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or(HubError::Unauthorized)?;
        let hash = token_hash(token);
        let user_id = state
            .store()
            .session_user(&hash, now_unix())
            .await?
            .ok_or(HubError::Unauthorized)?;
        Ok(AuthUser {
            user_id,
            token_hash: hash,
        })
    }
}

// ---------- Routes ----------

async fn api_info() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "service": "raftup-hub", "api_version": API_VERSION }))
}

async fn register(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<RegisterBody>,
) -> Result<(StatusCode, Json<AccountView>), HubError> {
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
            now_unix(),
        )
        .await?;
    // Ni l'adresse ni le mot de passe dans le journal (NF09, RGPD).
    tracing::info!(user_id, "compte créé");
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

    let user = state
        .store()
        .user_by_id(user_id)
        .await?
        .ok_or(HubError::InvalidCredentials)?;
    let token = new_token();
    let now = now_unix();
    let lifetime = i64::try_from(state.inner.config.session_lifetime.as_secs()).unwrap_or(i64::MAX);
    let expires_at = now.saturating_add(lifetime);
    state
        .store()
        .insert_session(&token_hash(&token), user_id, now, expires_at)
        .await?;
    tracing::info!(user_id, "session ouverte");
    Ok(Json(SessionView {
        token,
        user_id,
        username: user.username,
        expires_at,
    }))
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
    let email = state.store().email_of(user.user_id).await?;
    Ok(Json(MeView {
        user_id: row.id,
        username: row.username,
        email,
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

/// Cherche un autre joueur par son pseudo.
async fn other_player(state: &AppState, me: i64, username: &str) -> Result<UserRow, HubError> {
    let other = state
        .store()
        .user_by_username(&normalize_username(username))
        .await?
        .ok_or(HubError::UserNotFound)?;
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
