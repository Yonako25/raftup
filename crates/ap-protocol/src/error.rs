//! Erreurs de lecture des paquets.

/// Erreur rencontrée en lisant une trame ou un paquet.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// La trame n'est pas du JSON valide, ou pas un tableau de paquets
    /// (ou elle est trop profondément imbriquée : serde_json s'arrête à 128 niveaux).
    #[error("trame JSON invalide : {0}")]
    Json(#[from] serde_json::Error),

    /// Un élément de la trame n'est pas un objet JSON.
    #[error("un paquet doit être un objet JSON")]
    NotAnObject,

    /// Le paquet n'a pas de champ `cmd` de type texte.
    #[error("paquet sans champ « cmd » de type texte")]
    MissingCmd,

    /// Le paquet a une commande connue mais des champs incorrects.
    #[error("paquet {cmd} invalide : {source}")]
    InvalidPacket {
        cmd: String,
        #[source]
        source: serde_json::Error,
    },
}
