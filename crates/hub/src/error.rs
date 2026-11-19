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
    #[error("présence invalide : {0}")]
    InvalidPresence(&'static str),
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
            HubError::InvalidPresence(_) => (StatusCode::BAD_REQUEST, "invalid_presence"),
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
