//! Routes `/v1/games` : parties hébergées par le hub (ADR 0052, point 9).
//!
//! | Route | Qui | Effet |
//! | --- | --- | --- |
//! | `POST /v1/games` | compte | envoie une seed (et peut-être une sauvegarde) ; rend l'identifiant, l'adresse, l'empreinte `fp` et le mot de passe d'admin |
//! | `GET /v1/games` | compte | ses parties hébergées |
//! | `GET /v1/games/{id}` | qui connaît l'identifiant | état : joueurs connectés, avancement par slot |
//! | `POST /v1/games/{id}/wake` | qui connaît l'identifiant | réveille une partie en veille (ou relance une partie arrêtée) |
//! | `GET /v1/games/{id}/export` | hôte | zip de la seed et de la sauvegarde |
//! | `DELETE /v1/games/{id}` | hôte | arrête et supprime la partie |
//!
//! Toutes demandent un jeton de compte. L'identifiant (128 bits aléatoires) sert de
//! laissez-passer : un joueur à qui l'hôte a donné le lien d'invitation voit l'état
//! de la partie. Le salon (s'il est validé) précisera qui est « membre ».
//!
//! Le corps de `POST /v1/games` est en JSON, avec la seed et la sauvegarde en base64
//! (standard, avec `=`) : plus simple à envoyer qu'un formulaire `multipart`, au prix
//! d'un tiers de taille en plus. Cette route a sa propre limite de corps, assez
//! grande pour une seed et une sauvegarde de 32 Mio chacune.

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};

use super::{valid_id, Created, Upload};
use crate::api::{ApiJson, AuthUser};
use crate::error::HubError;
use crate::store::GameRow;
use crate::{now_unix, AppState};

/// Routes des parties, prêtes à fusionner dans le routeur public.
pub fn routes() -> Router<AppState> {
    let defaults = super::GamesConfig::default();
    Router::new()
        .route(
            "/v1/games",
            post(create_game)
                .get(list_games)
                // Limite propre à cette route : elle passe avant celle de 16 Kio du
                // reste de l'API (la couche la plus proche de la route gagne).
                .layer(DefaultBodyLimit::max(upload_body_limit(
                    defaults.max_seed_bytes,
                    defaults.max_save_bytes,
                ))),
        )
        .route("/v1/games/{id}", get(game_status).delete(delete_game))
        .route("/v1/games/{id}/export", get(export_game))
        .route("/v1/games/{id}/wake", post(wake_game))
}

/// Taille du corps de `POST /v1/games` : les deux fichiers en base64 (4 octets pour
/// 3), plus un peu de place pour le reste du JSON.
fn upload_body_limit(seed: usize, save: usize) -> usize {
    (seed + save).div_ceil(3) * 4 + 64 * 1024
}

/// Corps de `POST /v1/games`.
///
/// Pas de `Debug` dérivé : il afficherait le mot de passe.
#[derive(Deserialize)]
pub struct CreateGameBody {
    pub name: String,
    #[serde(default)]
    pub password: Option<String>,
    /// Le fichier `.archipelago`, en base64.
    pub seed: String,
    /// Une sauvegarde `.raftup` de cette seed, en base64, pour continuer une partie.
    #[serde(default)]
    pub save: Option<String>,
}

/// Réponse de `POST /v1/games`.
#[derive(Debug, Serialize)]
pub struct CreatedView {
    #[serde(flatten)]
    pub game: GameView,
    /// Mot de passe d'admin (`!admin login …`), tiré au hasard par le hub.
    pub admin_password: String,
    /// Vrai si la sauvegarde envoyée a repris.
    pub resumed: bool,
}

/// Une partie hébergée, vue par l'API.
#[derive(Debug, Serialize)]
pub struct GameView {
    pub id: String,
    pub name: String,
    pub seed_name: String,
    /// `active` (le serveur tourne), `sleeping` (en veille : la première connexion
    /// la réveille) ou `stopped` (elle n'a pas pu repartir).
    pub state: &'static str,
    /// Adresse à donner aux mods : `hôte:port`, sans schéma, pour qu'un client essaie
    /// `wss://` puis retombe sur `ws://` (ADR 0043).
    pub address: String,
    pub host: String,
    pub port: i64,
    /// Empreinte du certificat (`fp` des liens d'invitation) ; `None` si arrêtée.
    pub fingerprint: Option<String>,
    pub players: i64,
    pub has_password: bool,
    pub created_at: i64,
    /// Dernier relevé où une connexion était entrée dans un slot ; `None` : jamais.
    pub last_activity_at: Option<i64>,
}

/// État détaillé (`GET /v1/games/{id}`).
#[derive(Debug, Serialize)]
pub struct GameStatusView {
    #[serde(flatten)]
    pub game: GameView,
    /// Vrai si le compte qui demande est l'hôte.
    pub is_host: bool,
    /// Mot de passe d'admin, pour l'hôte seulement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_password: Option<String>,
    /// Connexions ouvertes (joueurs et outils de suivi).
    pub connections: usize,
    /// Avancement par slot, comme « Partie en cours » ; vide si la partie est arrêtée.
    pub slots: Vec<server::SlotStatus>,
}

#[derive(Debug, Serialize)]
pub struct GameList {
    pub games: Vec<GameView>,
}

/// Hôte des adresses de partie : celui de l'adresse publique du hub.
async fn public_host(state: &AppState) -> Result<String, HubError> {
    let settings = state.settings().await?;
    let host = url::Url::parse(&settings.public_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "127.0.0.1".to_string());
    Ok(host)
}

async fn view(state: &AppState, row: &GameRow, host: &str) -> GameView {
    let (game_state, fingerprint) = state.games().state(&row.id).await;
    GameView {
        id: row.id.clone(),
        name: row.name.clone(),
        seed_name: row.seed_name.clone(),
        state: game_state.as_str(),
        address: format!("{host}:{}", row.port),
        host: host.to_string(),
        port: row.port,
        fingerprint,
        players: row.player_count,
        has_password: row.password_enc.is_some(),
        created_at: row.created_at,
        last_activity_at: row.last_activity_at,
    }
}

/// Lit un fichier envoyé en base64, sans dépasser `max` octets une fois décodé.
fn decode(field: &str, text: &str, max: usize) -> Result<Vec<u8>, HubError> {
    // 4 caractères base64 donnent au plus 3 octets : on refuse avant de décoder.
    if text.len().div_ceil(4) * 3 > max + 3 {
        return Err(HubError::SeedTooLarge);
    }
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .map_err(|_| HubError::BadRequest(format!("{field} : base64 illisible")))
}

async fn create_game(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<CreateGameBody>,
) -> Result<(StatusCode, Json<CreatedView>), HubError> {
    if !state.settings().await?.hosting {
        return Err(HubError::HostingDisabled);
    }
    let config = state.games().config();
    let seed = decode("seed", &body.seed, config.max_seed_bytes)?;
    let save = body
        .save
        .as_deref()
        .map(|text| decode("save", text, config.max_save_bytes))
        .transpose()?;
    let upload = Upload {
        name: body.name,
        password: body.password,
        seed,
        save,
    };
    let Created {
        row,
        fingerprint: _,
        admin_password,
        resumed,
    } = state
        .games()
        .create(state.store(), user.user_id, upload, now_unix())
        .await?;
    let host = public_host(&state).await?;
    let game = view(&state, &row, &host).await;
    Ok((
        StatusCode::CREATED,
        Json(CreatedView {
            game,
            admin_password,
            resumed,
        }),
    ))
}

async fn list_games(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<GameList>, HubError> {
    let host = public_host(&state).await?;
    let mut games = Vec::new();
    for row in state.store().games_of(user.user_id).await? {
        games.push(view(&state, &row, &host).await);
    }
    Ok(Json(GameList { games }))
}

/// La partie `id`, ou `game_not_found` (identifiant mal formé compris).
async fn find(state: &AppState, id: &str) -> Result<GameRow, HubError> {
    if !valid_id(id) {
        return Err(HubError::GameNotFound);
    }
    state.store().game(id).await?.ok_or(HubError::GameNotFound)
}

/// La partie `id`, si le compte en est l'hôte.
async fn find_hosted_by(state: &AppState, id: &str, user_id: i64) -> Result<GameRow, HubError> {
    let row = find(state, id).await?;
    if row.owner_id != user_id {
        return Err(HubError::NotGameHost);
    }
    Ok(row)
}

async fn game_status(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<GameStatusView>, HubError> {
    let row = find(&state, &id).await?;
    let host = public_host(&state).await?;
    let game = view(&state, &row, &host).await;
    let status = state.games().status(&row.id).await;
    let is_host = row.owner_id == user.user_id;
    let admin_password = if is_host {
        Some(state.games().admin_password(&row)?)
    } else {
        None
    };
    let (connections, slots) = match status {
        Some(status) => (status.connections, status.slots),
        None => (0, Vec::new()),
    };
    Ok(Json(GameStatusView {
        game,
        is_host,
        admin_password,
        connections,
        slots,
    }))
}

/// Réveille la partie et rend son nouvel état. Une partie déjà éveillée reste comme
/// elle est ; une partie qui ne repart pas répond par une erreur.
async fn wake_game(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<GameView>, HubError> {
    let row = find(&state, &id).await?;
    state.games().wake(state.store(), &row.id).await?;
    let host = public_host(&state).await?;
    // Relue : `asleep` vient de changer en base.
    let row = find(&state, &id).await?;
    Ok(Json(view(&state, &row, &host).await))
}

async fn export_game(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Response, HubError> {
    let row = find_hosted_by(&state, &id, user.user_id).await?;
    let zip = state.games().export(&row.id).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                // L'identifiant n'a que des chiffres hexadécimaux : rien à échapper.
                format!("attachment; filename=\"partie-{}.zip\"", row.id),
            ),
        ],
        zip,
    )
        .into_response())
}

async fn delete_game(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, HubError> {
    let row = find_hosted_by(&state, &id, user.user_id).await?;
    state.games().delete(state.store(), &row.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limite_du_corps() {
        // 64 Mio de fichiers en base64 : un peu plus de 85 Mio.
        let limit = upload_body_limit(32 << 20, 32 << 20);
        assert!(limit > (64 << 20) * 4 / 3);
        assert!(limit < 90 << 20);
    }

    #[test]
    fn base64_trop_long_refuse_avant_decodage() {
        assert!(matches!(
            decode("seed", &"A".repeat(40), 10),
            Err(HubError::SeedTooLarge)
        ));
        assert_eq!(decode("seed", "AAEC", 10).ok(), Some(vec![0, 1, 2]));
        assert!(decode("seed", "@@@@", 10).is_err());
    }
}
