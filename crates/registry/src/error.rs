//! Erreurs du registre.

/// Ce qui fait refuser un index, une signature ou un paquet.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// Le texte n'est pas un index JSON du bon schéma.
    #[error("index du registre illisible : {0}")]
    Parse(String),
    /// L'index est d'un format plus récent (ou inconnu).
    #[error("format d'index {found} non pris en charge (ce programme lit le format {supported})")]
    UnsupportedFormat { found: u32, supported: u32 },
    /// Un champ d'un paquet ne respecte pas les règles de l'ADR 0032.
    #[error("paquet « {package} » : {reason}")]
    InvalidPackage { package: String, reason: String },
    /// L'index lui-même est faux (nom, doublon, trop de paquets…).
    #[error("index du registre invalide : {0}")]
    InvalidIndex(String),
    /// L'index ou sa signature dépasse la taille permise.
    #[error("{what} trop gros (plus de {max} octets)")]
    TooLarge { what: &'static str, max: u64 },
    /// Aucune clé publique n'est configurée : on refuse tout plutôt que d'accepter n'importe quoi.
    #[error("aucune clé publique du registre n'est configurée : index refusé")]
    PublicKeyNotConfigured,
    /// La clé publique n'est pas une clé minisign valide.
    #[error("clé publique minisign invalide : {0}")]
    BadPublicKey(String),
    /// La signature n'est pas au format minisign.
    #[error("signature minisign illisible : {0}")]
    BadSignature(String),
    /// La signature ne correspond pas à l'index (ou à la clé).
    #[error("signature du registre incorrecte : index rejeté")]
    SignatureMismatch,
    /// Les octets téléchargés n'ont pas la taille annoncée.
    #[error("paquet « {package} » : {found} octets au lieu de {expected}")]
    SizeMismatch {
        package: String,
        expected: u64,
        found: u64,
    },
    /// Les octets téléchargés n'ont pas le sha256 annoncé.
    #[error("paquet « {package} » : sha256 {found} au lieu de {expected}")]
    HashMismatch {
        package: String,
        expected: String,
        found: String,
    },
    /// Erreur de lecture ou d'écriture d'un fichier (publication).
    #[error("{path} : {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    /// Erreur de la clé secrète ou de la signature à la publication.
    #[error("signature impossible : {0}")]
    Signing(String),
}
