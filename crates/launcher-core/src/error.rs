//! Erreurs du launcher, avec un message que l'interface peut montrer tel quel.

/// Ce qui empêche une action du launcher.
#[derive(Debug, thiserror::Error)]
pub enum LauncherError {
    /// Fichier illisible (seed, certificat…).
    #[error("lecture de {path} : {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },

    /// Le fichier n'est pas une seed `.archipelago` valide.
    #[error("seed invalide : {0}")]
    Seed(#[from] multidata::LoadError),

    /// Certificat ou clé refusés.
    #[error("chiffrement : {0}")]
    Tls(#[from] server::TlsError),

    /// Sauvegarde `.raftup` illisible (autre seed, corrompue…) ou impossible à créer.
    #[error("sauvegarde : {0}")]
    Save(#[from] server::save::SaveSetupError),

    /// Le serveur ne démarre pas (port déjà pris…).
    #[error("serveur : {0}")]
    Server(#[from] server::ServerError),

    /// Une partie est déjà hébergée : il faut l'arrêter d'abord.
    #[error("une partie est déjà hébergée")]
    AlreadyRunning,

    /// Aucune partie hébergée.
    #[error("aucune partie hébergée")]
    NotRunning,
}
