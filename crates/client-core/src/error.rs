//! Erreurs du client.

use ap_protocol::ProtocolError;
use tokio_tungstenite::tungstenite;

/// Erreur rencontrée par le client.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// L'adresse saisie n'est pas utilisable.
    #[error("adresse invalide : {0}")]
    InvalidAddress(String),

    /// Erreur réseau ou WebSocket.
    ///
    /// `Box` : l'erreur de tungstenite est grosse en mémoire ; la ranger dans une
    /// boîte garde `ClientError` (et donc chaque `Result`) petit.
    #[error("erreur WebSocket : {0}")]
    WebSocket(Box<tungstenite::Error>),

    /// Le serveur a envoyé un paquet illisible.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    /// Le serveur a refusé la connexion (`ConnectionRefused`).
    #[error("connexion refusée par le serveur : {}", .0.join(", "))]
    Refused(Vec<String>),

    /// Paquet inattendu pendant l'ouverture de la session.
    #[error("paquet inattendu pendant la connexion : {0}")]
    UnexpectedPacket(String),

    /// Le serveur a fermé la connexion pendant l'ouverture de la session.
    #[error("le serveur a fermé la connexion")]
    Closed,

    /// Le serveur n'a pas répondu à temps pendant l'ouverture de la session.
    #[error("le serveur ne répond pas")]
    Timeout,
}

// `#[from]` ne sait pas mettre en boîte : on écrit la conversion à la main.
// Elle permet d'utiliser `?` sur les erreurs de tungstenite.
impl From<tungstenite::Error> for ClientError {
    fn from(error: tungstenite::Error) -> Self {
        ClientError::WebSocket(Box::new(error))
    }
}
