//! Types échangés avec l'API `/v1` du hub, **redéfinis côté launcher**.
//!
//! Le hub est en AGPL (ADR 0003) : le launcher (MIT / Apache) ne dépend jamais de sa
//! crate. Ces types recopient donc la forme du JSON de `crates/hub/src/api.rs`, sans
//! partager de code. Un champ ajouté plus tard par le hub est ignoré à la lecture
//! (serde ignore les champs inconnus par défaut) : un vieux launcher reste compatible.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Mot de passe tapé par le joueur. Il ne fait que passer : jamais écrit sur le disque,
/// jamais dans le journal (NF09).
///
/// `Debug` est écrit à la main pour afficher `Password(***)` ; il n'y a pas de `Display`.
#[derive(Clone)]
pub struct Password(String);

impl Password {
    pub fn new(text: impl Into<String>) -> Password {
        Password(text.into())
    }

    /// Le texte en clair, seulement pour le corps de la requête.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(***)")
    }
}

/// Jeton de session délivré par le hub à la connexion. C'est un secret : qui le connaît
/// agit au nom du joueur. `Debug` le masque.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken(String);

impl SessionToken {
    pub fn new(text: impl Into<String>) -> SessionToken {
        SessionToken(text.into())
    }

    /// Le jeton en clair, pour l'en-tête `Authorization` ou le trousseau. Ne pas logger.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionToken(***)")
    }
}

/// Réponse de `GET /v1`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ApiInfo {
    pub service: String,
    pub api_version: u32,
}

/// Compte créé (`POST /v1/accounts`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Account {
    pub user_id: i64,
    pub username: String,
}

/// Session ouverte (`POST /v1/sessions`). `Debug` vient de `derive`, mais le jeton s'y
/// affiche masqué grâce au `Debug` de [`SessionToken`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub token: SessionToken,
    pub user_id: i64,
    pub username: String,
    /// Fin de validité, en secondes Unix.
    pub expires_at: i64,
}

/// Forme JSON de la session : le jeton y est une simple chaîne.
#[derive(Deserialize)]
pub(crate) struct SessionJson {
    pub token: String,
    pub user_id: i64,
    pub username: String,
    pub expires_at: i64,
}

/// Compte connecté (`GET /v1/me`). L'adresse n'est jamais journalisée (RGPD).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Me {
    pub user_id: i64,
    pub username: String,
    #[serde(default)]
    pub email: Option<String>,
}

/// État de présence. `snake_case` : `in_game` dans le JSON, comme côté hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Online,
    InGame,
    Offline,
}

/// Présence d'un joueur : le nom du jeu n'existe qu'en partie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
}

impl Presence {
    pub fn online() -> Presence {
        Presence {
            status: Status::Online,
            game: None,
        }
    }

    pub fn offline() -> Presence {
        Presence {
            status: Status::Offline,
            game: None,
        }
    }

    pub fn in_game(game: impl Into<String>) -> Presence {
        Presence {
            status: Status::InGame,
            game: Some(game.into()),
        }
    }
}

/// Un ami et sa présence (`GET /v1/friends`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Friend {
    pub username: String,
    pub presence: Presence,
}

/// Un joueur dans une liste de demandes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Player {
    pub username: String,
}

/// Demandes d'ami reçues et envoyées (`GET /v1/friends/requests`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct FriendRequests {
    pub incoming: Vec<Player>,
    pub outgoing: Vec<Player>,
}

/// Résultat d'une demande d'ami.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestOutcome {
    /// La demande attend la réponse de l'autre joueur.
    Pending,
    /// L'autre joueur nous avait déjà demandé : vous êtes amis tout de suite.
    Accepted,
}

#[derive(Deserialize)]
pub(crate) struct RequestOutcomeJson {
    pub status: String,
}

/// Corps d'erreur du hub : `{"error": "<code>", "message": "<texte>"}`.
#[derive(Deserialize)]
pub(crate) struct ErrorJson {
    pub error: String,
    #[serde(default)]
    pub message: String,
}

/// Code d'erreur stable renvoyé par le hub (`error` dans le JSON).
///
/// Un code que ce launcher ne connaît pas encore arrive dans `Other` : un hub plus
/// récent ne casse pas un vieux launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    InvalidUsername,
    InvalidEmail,
    WeakPassword,
    UsernameTaken,
    /// Pseudo déjà changé il y a moins de 24 heures.
    UsernameChangeTooSoon,
    EmailTaken,
    InvalidCredentials,
    Unauthorized,
    TooManyAttempts,
    UserNotFound,
    RequestNotFound,
    FriendNotFound,
    SelfFriend,
    AlreadyFriends,
    RequestExists,
    InvalidPresence,
    /// Le hub n'accepte plus d'inscriptions (mode `closed`, ADR 0036).
    RegistrationClosed,
    /// Le hub demande un code d'invitation et aucun n'a été envoyé.
    InviteRequired,
    /// Code d'invitation inconnu, expiré ou déjà utilisé.
    InvalidInvite,
    BadRequest,
    Internal,
    Other(String),
}

impl ErrorCode {
    pub fn parse(code: &str) -> ErrorCode {
        match code {
            "invalid_username" => ErrorCode::InvalidUsername,
            "invalid_email" => ErrorCode::InvalidEmail,
            "weak_password" => ErrorCode::WeakPassword,
            "username_taken" => ErrorCode::UsernameTaken,
            "username_change_too_soon" => ErrorCode::UsernameChangeTooSoon,
            "email_taken" => ErrorCode::EmailTaken,
            "invalid_credentials" => ErrorCode::InvalidCredentials,
            "unauthorized" => ErrorCode::Unauthorized,
            "too_many_attempts" => ErrorCode::TooManyAttempts,
            "user_not_found" => ErrorCode::UserNotFound,
            "request_not_found" => ErrorCode::RequestNotFound,
            "friend_not_found" => ErrorCode::FriendNotFound,
            "self_friend" => ErrorCode::SelfFriend,
            "already_friends" => ErrorCode::AlreadyFriends,
            "request_exists" => ErrorCode::RequestExists,
            "invalid_presence" => ErrorCode::InvalidPresence,
            "registration_closed" => ErrorCode::RegistrationClosed,
            "invite_required" => ErrorCode::InviteRequired,
            "invalid_invite" => ErrorCode::InvalidInvite,
            "bad_request" => ErrorCode::BadRequest,
            "internal" => ErrorCode::Internal,
            other => ErrorCode::Other(other.to_string()),
        }
    }
}
