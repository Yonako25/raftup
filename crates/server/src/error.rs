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

    /// La tâche salle est arrêtée : le serveur s'éteint.
    #[error("la salle est fermée")]
    RoomClosed,

    /// Trame illisible : le serveur officiel ferme aussi la connexion.
    #[error("trame invalide : {0}")]
    BadFrame(String),

    /// Message plus gros que la limite (NF03).
    #[error("message trop gros : {size} octets, {max_size} au plus")]
    TooLarge { size: usize, max_size: usize },

    /// Plus de paquets par seconde que la limite (NF04).
    #[error("trop de paquets par seconde")]
    TooManyPackets,

    /// Le client ne lit pas assez vite : sa file d'envoi est pleine.
    #[error("client trop lent, file d'envoi pleine")]
    TooSlow,

    /// Rien reçu du client depuis trop longtemps (NF05).
    #[error("aucune réponse du client")]
    Silent,

    /// Poignée de main TLS ratée (client en `ws://`, certificat refusé…).
    #[error("TLS : {0}")]
    Tls(std::io::Error),

    /// Poignée de main TLS ou WebSocket trop longue.
    #[error("poignée de main trop longue")]
    HandshakeTimeout,
}

/// Configuration TLS impossible à charger (ADR 0009).
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// Fichier absent ou illisible.
    #[error("lecture de {path} : {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },

    /// Pas de certificat PEM (`BEGIN CERTIFICATE`) dans le fichier.
    #[error("aucun certificat PEM dans {path}")]
    NoCertificate { path: String },

    /// Pas de clé privée PEM (`BEGIN PRIVATE KEY`…) dans le fichier.
    #[error("aucune clé privée PEM dans {path}")]
    NoKey { path: String },

    /// PEM mal formé.
    #[error("PEM illisible dans {path} : {message}")]
    BadPem { path: String, message: String },

    /// rustls refuse la paire : clé qui ne va pas avec le certificat, type de
    /// clé non géré…
    #[error("certificat ou clé refusés : {0}")]
    Rejected(#[from] rustls::Error),
}

// Écrit à la main (pas de `#[from]`) pour ranger l'erreur dans une `Box`.
impl From<tokio_tungstenite::tungstenite::Error> for ConnectionError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        ConnectionError::WebSocket(Box::new(error))
    }
}
