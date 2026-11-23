//! Erreurs du hub et leur traduction en réponse HTTP.
//!
//! Chaque erreur a un code stable (`error` dans le JSON), que le launcher peut tester,
//! et un message lisible (`message`). Aucun message ne contient de mot de passe (NF09).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Toutes les erreurs que peut renvoyer l'API du hub.
#[derive(Debug, thiserror::Error)]
pub enum HubError {
    #[error("pseudo invalide : {0}")]
    InvalidUsername(&'static str),
    #[error("adresse e-mail invalide")]
    InvalidEmail,
    #[error("mot de passe refusé : {0}")]
    WeakPassword(&'static str),
    #[error("ce pseudo est déjà pris")]
    UsernameTaken,
    #[error("pseudo déjà changé il y a moins de 24 heures, réessayer plus tard")]
    UsernameChangeTooSoon,
    #[error("cette adresse e-mail a déjà un compte")]
    EmailTaken,
    #[error("e-mail ou mot de passe incorrect")]
    InvalidCredentials,
    #[error("session absente, expirée ou révoquée")]
    Unauthorized,
    #[error("trop de tentatives, réessayer plus tard")]
    TooManyAttempts,
    #[error("aucun joueur ne porte ce pseudo")]
    UserNotFound,
    #[error("aucune demande d'ami de ce joueur")]
    RequestNotFound,
    #[error("ce joueur n'est pas dans la liste d'amis")]
    FriendNotFound,
    #[error("impossible de s'ajouter soi-même en ami")]
    SelfFriend,
    #[error("vous êtes déjà amis")]
    AlreadyFriends,
    #[error("demande d'ami déjà envoyée")]
    RequestExists,
    #[error("impossible de se bloquer soi-même")]
    SelfBlock,
    #[error("vous avez bloqué ce joueur : levez le blocage d'abord")]
    YouBlocked,
    #[error("ce joueur n'est pas bloqué")]
    BlockNotFound,
    #[error("lien ou code invalide, expiré ou déjà utilisé")]
    InvalidToken,
    #[error("adresse e-mail déjà vérifiée")]
    AlreadyVerified,
    #[error("connexion Discord non configurée sur ce hub")]
    DiscordDisabled,
    #[error("connexion Discord refusée")]
    DiscordRefused,
    #[error("Discord ne répond pas, réessayer plus tard")]
    DiscordUnavailable,
    #[error("ce compte Discord est déjà lié à un autre compte")]
    DiscordTaken,
    #[error("ce compte a déjà un compte Discord lié")]
    AlreadyLinked,
    #[error("aucun moyen de connexion de ce type sur le compte")]
    IdentityNotFound,
    #[error("c'est le dernier moyen de connexion du compte")]
    LastIdentity,
    #[error("présence invalide : {0}")]
    InvalidPresence(&'static str),
    /// Compte suspendu par l'admin du hub (ADR 0036).
    #[error("ce compte est suspendu")]
    AccountSuspended,
    #[error("impossible de suspendre son propre compte")]
    SelfSuspend,
    #[error("les inscriptions sont fermées sur ce hub")]
    RegistrationClosed,
    #[error("ce hub demande un code d'invitation pour s'inscrire")]
    InviteRequired,
    #[error("code d'invitation invalide, expiré ou déjà utilisé")]
    InvalidInvite,
    #[error("aucun code d'invitation avec ce numéro")]
    InviteNotFound,
    #[error("réglage inconnu")]
    UnknownSetting,
    #[error("réglage refusé : {0}")]
    InvalidSetting(&'static str),
    /// Illustrations des jeux (ADR 0047).
    #[error("illustrations non configurées sur ce hub")]
    ArtDisabled,
    #[error("aucune illustration pour ce jeu")]
    ArtNotFound,
    #[error("IGDB ne répond pas, réessayer plus tard")]
    ArtUnavailable,
    #[error("trop de demandes d'illustrations, réessayer plus tard")]
    ArtRateLimited,
    /// Parties hébergées (ADR 0052).
    #[error("ce hub n'héberge pas de parties")]
    HostingDisabled,
    #[error("aucune partie hébergée avec cet identifiant")]
    GameNotFound,
    #[error("seul l'hôte de la partie peut faire cela")]
    NotGameHost,
    #[error("nom de partie : 1 à {0} caractères, sans caractère de contrôle")]
    InvalidGameName(usize),
    /// Le texte vient de la lecture sécurisée de `multidata` (jamais de mot de passe).
    #[error("seed refusée : {0}")]
    InvalidSeed(String),
    #[error("sauvegarde refusée : {0}")]
    InvalidSave(String),
    #[error("seed ou sauvegarde trop grosse")]
    SeedTooLarge,
    #[error("trop de joueurs dans la seed : {0} au plus")]
    TooManyPlayers(usize),
    #[error("déjà {0} parties hébergées sur ce compte : en supprimer une d'abord")]
    GameLimitReached(i64),
    #[error("ce hub héberge déjà autant de parties qu'il peut, réessayer plus tard")]
    HubFull,
    #[error("aucun port libre pour une nouvelle partie, réessayer plus tard")]
    NoFreePort,
    /// Corps de requête illisible (JSON mal formé, champ manquant…).
    #[error("requête invalide : {0}")]
    BadRequest(String),
    /// Erreur de la base. `#[from]` permet d'écrire `?` sur une `sqlx::Error`.
    #[error("erreur de la base de données : {0}")]
    Database(#[from] sqlx::Error),
    #[error("erreur interne : {0}")]
    Internal(String),
}

impl HubError {
    /// Code HTTP et code d'erreur stable.
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            HubError::InvalidUsername(_) => (StatusCode::BAD_REQUEST, "invalid_username"),
            HubError::InvalidEmail => (StatusCode::BAD_REQUEST, "invalid_email"),
            HubError::WeakPassword(_) => (StatusCode::BAD_REQUEST, "weak_password"),
            HubError::UsernameTaken => (StatusCode::CONFLICT, "username_taken"),
            HubError::UsernameChangeTooSoon => {
                (StatusCode::TOO_MANY_REQUESTS, "username_change_too_soon")
            }
            HubError::EmailTaken => (StatusCode::CONFLICT, "email_taken"),
            HubError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid_credentials"),
            HubError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            HubError::TooManyAttempts => (StatusCode::TOO_MANY_REQUESTS, "too_many_attempts"),
            HubError::UserNotFound => (StatusCode::NOT_FOUND, "user_not_found"),
            HubError::RequestNotFound => (StatusCode::NOT_FOUND, "request_not_found"),
            HubError::FriendNotFound => (StatusCode::NOT_FOUND, "friend_not_found"),
            HubError::SelfFriend => (StatusCode::BAD_REQUEST, "self_friend"),
            HubError::AlreadyFriends => (StatusCode::CONFLICT, "already_friends"),
            HubError::RequestExists => (StatusCode::CONFLICT, "request_exists"),
            HubError::SelfBlock => (StatusCode::BAD_REQUEST, "self_block"),
            HubError::YouBlocked => (StatusCode::CONFLICT, "you_blocked"),
            HubError::BlockNotFound => (StatusCode::NOT_FOUND, "block_not_found"),
            HubError::InvalidToken => (StatusCode::BAD_REQUEST, "invalid_token"),
            HubError::AlreadyVerified => (StatusCode::CONFLICT, "already_verified"),
            HubError::DiscordDisabled => (StatusCode::SERVICE_UNAVAILABLE, "discord_disabled"),
            HubError::DiscordRefused => (StatusCode::BAD_REQUEST, "discord_refused"),
            HubError::DiscordUnavailable => (StatusCode::BAD_GATEWAY, "discord_unavailable"),
            HubError::DiscordTaken => (StatusCode::CONFLICT, "discord_taken"),
            HubError::AlreadyLinked => (StatusCode::CONFLICT, "already_linked"),
            HubError::IdentityNotFound => (StatusCode::NOT_FOUND, "identity_not_found"),
            HubError::LastIdentity => (StatusCode::CONFLICT, "last_identity"),
            HubError::InvalidPresence(_) => (StatusCode::BAD_REQUEST, "invalid_presence"),
            HubError::AccountSuspended => (StatusCode::FORBIDDEN, "account_suspended"),
            HubError::SelfSuspend => (StatusCode::BAD_REQUEST, "self_suspend"),
            HubError::RegistrationClosed => (StatusCode::FORBIDDEN, "registration_closed"),
            HubError::InviteRequired => (StatusCode::FORBIDDEN, "invite_required"),
            HubError::InvalidInvite => (StatusCode::FORBIDDEN, "invalid_invite"),
            HubError::InviteNotFound => (StatusCode::NOT_FOUND, "invite_not_found"),
            HubError::UnknownSetting => (StatusCode::BAD_REQUEST, "unknown_setting"),
            HubError::InvalidSetting(_) => (StatusCode::BAD_REQUEST, "invalid_setting"),
            HubError::ArtDisabled => (StatusCode::SERVICE_UNAVAILABLE, "art_disabled"),
            HubError::ArtNotFound => (StatusCode::NOT_FOUND, "art_not_found"),
            HubError::ArtUnavailable => (StatusCode::BAD_GATEWAY, "art_unavailable"),
            HubError::ArtRateLimited => (StatusCode::TOO_MANY_REQUESTS, "art_rate_limited"),
            HubError::HostingDisabled => (StatusCode::SERVICE_UNAVAILABLE, "hosting_disabled"),
            HubError::GameNotFound => (StatusCode::NOT_FOUND, "game_not_found"),
            HubError::NotGameHost => (StatusCode::FORBIDDEN, "not_game_host"),
            HubError::InvalidGameName(_) => (StatusCode::BAD_REQUEST, "invalid_game_name"),
            HubError::InvalidSeed(_) => (StatusCode::BAD_REQUEST, "invalid_seed"),
            HubError::InvalidSave(_) => (StatusCode::BAD_REQUEST, "invalid_save"),
            HubError::SeedTooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "seed_too_large"),
            HubError::TooManyPlayers(_) => (StatusCode::BAD_REQUEST, "too_many_players"),
            HubError::GameLimitReached(_) => (StatusCode::CONFLICT, "game_limit_reached"),
            HubError::HubFull => (StatusCode::SERVICE_UNAVAILABLE, "hub_full"),
            HubError::NoFreePort => (StatusCode::SERVICE_UNAVAILABLE, "no_free_port"),
            HubError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            HubError::Database(_) | HubError::Internal(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        }
    }
}

/// `IntoResponse` : axum sait transformer une `HubError` en réponse HTTP,
/// ce qui permet aux routes de renvoyer `Result<_, HubError>`.
impl IntoResponse for HubError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();
        let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
            // Le détail reste dans le journal du serveur, pas chez le client.
            // Les erreurs sqlx ne contiennent pas les valeurs liées aux requêtes.
            tracing::error!(error = %self, "erreur interne du hub");
            "erreur interne".to_string()
        } else {
            self.to_string()
        };
        (status, Json(json!({ "error": code, "message": message }))).into_response()
    }
}

/// Erreur des illustrations vue par l'API. Le détail d'une panne d'IGDB reste au
/// journal du hub.
impl From<crate::art::ArtError> for HubError {
    fn from(error: crate::art::ArtError) -> HubError {
        use crate::art::ArtError;
        match error {
            ArtError::Disabled => HubError::ArtDisabled,
            ArtError::NotFound => HubError::ArtNotFound,
            ArtError::RateLimited => HubError::ArtRateLimited,
            ArtError::BadId => {
                HubError::BadRequest("identifiant d'illustration refusé".to_string())
            }
            ArtError::Unavailable(_) | ArtError::TooLarge | ArtError::NotAnImage => {
                tracing::warn!(%error, "illustration indisponible");
                HubError::ArtUnavailable
            }
        }
    }
}
