//! Erreurs de lecture.

/// Erreur du lecteur pickle, avec la position de l'opcode fautif.
#[derive(Debug, thiserror::Error)]
#[error("pickle invalide à l'octet {at} : {kind}")]
pub struct PickleError {
    /// Position (en octets) de l'opcode en cours de lecture.
    pub at: usize,
    pub kind: PickleErrorKind,
}

/// Ce qui ne va pas dans le pickle.
#[derive(Debug, thiserror::Error)]
pub enum PickleErrorKind {
    #[error("fin des données inattendue")]
    UnexpectedEof,
    #[error("opcode {0:#04x} non pris en charge")]
    UnknownOpcode(u8),
    #[error("protocole pickle {0} non pris en charge")]
    UnsupportedProtocol(u8),
    /// Classe hors de la liste blanche : c'est ainsi qu'un pickle piégé
    /// essaie d'exécuter du code (par exemple `os.system`).
    #[error("classe interdite : {module}.{name}")]
    ForbiddenGlobal { module: String, name: String },
    #[error("pile vide")]
    StackUnderflow,
    #[error("marque absente")]
    MissingMark,
    #[error("entrée {0} absente du mémo")]
    BadMemo(u32),
    #[error("type inattendu : {0} attendu")]
    WrongType(&'static str),
    #[error("texte qui n'est pas de l'UTF-8 valide")]
    InvalidUtf8,
    #[error("entier trop grand ({0} octets)")]
    IntegerTooLarge(usize),
    #[error("trop d'éléments (limite : {0})")]
    TooManyNodes(usize),
    #[error("données après la fin du pickle")]
    TrailingData,
    #[error("le pickle ne produit pas exactement une valeur")]
    BadFinalStack,
}

/// Erreur de lecture d'un fichier `.archipelago`.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("fichier vide")]
    Empty,
    #[error("format de fichier {0} non pris en charge (3 au maximum)")]
    UnsupportedFormat(u8),
    #[error("décompression impossible : {0}")]
    Decompress(#[from] std::io::Error),
    #[error("données trop grosses une fois décompressées (limite : {limit} octets)")]
    TooLarge { limit: usize },
    #[error(transparent)]
    Pickle(#[from] PickleError),
}

/// Champ de la seed absent ou de mauvais type.
#[derive(Debug, thiserror::Error)]
#[error("seed invalide, champ {path} : {problem}")]
pub struct ModelError {
    /// Chemin du champ, par exemple `locations[1][81000]`.
    pub path: String,
    pub problem: String,
}

/// Erreur de chargement complet d'une seed.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    #[error(transparent)]
    Model(#[from] ModelError),
}
