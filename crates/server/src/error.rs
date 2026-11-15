//! Erreurs du serveur.

/// Erreur qui empêche le serveur de démarrer ou de continuer.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// Erreur réseau (port déjà pris, adresse invalide…).
    #[error("erreur réseau : {0}")]
    Io(#[from] std::io::Error),
}

/// Raison de la fermeture d'une connexion. Seule cette connexion est fermée.
#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    /// Erreur WebSocket (poignée de main ratée, connexion coupée…).
    ///
    /// `Box` : l'erreur de tungstenite est grosse ; la mettre sur le tas garde
    /// un `Result` léger.
    #[error("WebSocket : {0}")]
    WebSocket(Box<tokio_tungstenite::tungstenite::Error>),

    /// Paquet impossible à écrire en JSON.
    #[error("protocole : {0}")]
    Protocol(#[from] ap_protocol::ProtocolError),

    /// Trame illisible : le serveur officiel ferme aussi la connexion.
    #[error("trame invalide : {0}")]
    BadFrame(String),
}

// Écrit à la main (pas de `#[from]`) pour ranger l'erreur dans une `Box`.
impl From<tokio_tungstenite::tungstenite::Error> for ConnectionError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        ConnectionError::WebSocket(Box::new(error))
    }
}
