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

    /// Certificat généré par Raftup impossible à créer ou à écrire (ADR 0043).
    #[error("chiffrement : {0}")]
    Certificate(#[from] crate::certificate::CertificateError),

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

    // Erreurs des modules du launcher : `transparent` reprend leur message tel quel,
    // et `#[from]` permet d'écrire `?` pour convertir l'erreur du module en LauncherError.
    #[error(transparent)]
    Paths(#[from] crate::paths::PathsError),

    #[error(transparent)]
    Template(#[from] crate::template::TemplateError),

    #[error(transparent)]
    Profile(#[from] crate::profile::ProfileError),

    #[error(transparent)]
    Download(#[from] crate::download::DownloadError),

    #[error(transparent)]
    Archive(#[from] crate::archive::ArchiveError),

    #[error(transparent)]
    Record(#[from] crate::record::RecordError),

    #[error(transparent)]
    Install(#[from] crate::install::InstallError),

    #[error(transparent)]
    Lock(#[from] crate::lockfile::LockError),

    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),

    #[error(transparent)]
    Launch(#[from] crate::launch::LaunchError),
}
