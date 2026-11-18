//! Génération de seeds avec le générateur officiel d'Archipelago (ADR 0017).
//!
//! - [`pins`] : le fichier d'épinglage `generator-pins.toml` (Python, Archipelago, wheels),
//!   lu et validé.

pub mod pins;
mod run;
pub mod runtime;

pub use runtime::{
    ensure_runtime, ensure_runtime_cancellable, runtime_status, RuntimeStatus, SetupStep,
};

pub use pins::{parse_pins, pins, GeneratorPins, PinnedArchive, PinnedSource, PinnedWheel};
pub use run::{
    generate, GenerateOutput, GenerateRequest, GeneratorBackend, PythonBackend, Runtime,
    DEFAULT_SPOILER, DEFAULT_TIMEOUT,
};

/// Ce qui peut échouer pendant la préparation ou le lancement d'une génération.
///
/// L'énumération grossira avec les autres étapes (téléchargement, installation, worker).
#[derive(Debug, thiserror::Error)]
pub enum GenerationError {
    /// Le fichier d'épinglage est mal formé ou contient une valeur interdite.
    #[error("épinglage du générateur : {0}")]
    Pins(String),

    /// Le pont Python a échoué (lancement, flux, délai).
    #[error(transparent)]
    Bridge(#[from] py_bridge::BridgeError),

    /// Téléchargement refusé ou échoué (réseau, taille, hash).
    #[error(transparent)]
    Download(#[from] crate::download::DownloadError),

    /// Extraction d'une archive refusée ou impossible.
    #[error(transparent)]
    Archive(#[from] crate::archive::ArchiveError),

    /// Lecture ou écriture impossible sur ce chemin.
    #[error("accès à {path} : {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },

    /// Aucun fichier joueur fourni.
    #[error("aucun fichier joueur : il en faut au moins un")]
    NoPlayers,

    /// Fichier joueur refusé (meta.yaml, trop gros, pas un fichier…).
    #[error("fichier joueur refusé ({}) : {reason}", path.display())]
    InvalidPlayerFile {
        path: std::path::PathBuf,
        reason: String,
    },

    /// Apworld refusé (mauvaise extension, trop récent pour cet Archipelago…).
    #[error("apworld refusé ({}) : {reason}", path.display())]
    IncompatibleApworld {
        path: std::path::PathBuf,
        reason: String,
    },

    /// Le générateur a échoué ; `traceback` aide à comprendre (peut être vide).
    #[error("le générateur a échoué ({kind}) : {message}")]
    GeneratorFailed {
        kind: String,
        message: String,
        traceback: String,
    },

    /// Génération annulée par le joueur.
    #[error("génération annulée")]
    Cancelled,

    /// Une génération est déjà en cours.
    #[error("une génération est déjà en cours")]
    Busy,

    /// Le dossier d'Archipelago n'est pas sous les données de Raftup : on refuse d'y vider
    /// `custom_worlds` ou d'y copier des apworlds (on ne touche pas au dossier de quelqu'un d'autre).
    #[error("dossier d'Archipelago hors des données de Raftup, apworlds refusés : {}", .0.display())]
    UnmanagedArchipelago(std::path::PathBuf),

    /// Une wheel ne peut pas être installée telle quelle (ex. dossier `.data`).
    #[error("wheel {wheel} refusée : {reason}")]
    UnsupportedWheel { wheel: String, reason: String },

    /// Deux wheels (ou deux entrées) veulent écrire le même fichier dans `site-packages`.
    #[error("la wheel {wheel} réécrit le fichier {path}, déjà fourni par une autre")]
    DuplicateWheelFile { wheel: String, path: String },

    /// Le zip produit ne contient aucun fichier `.archipelago`.
    #[error("le générateur n'a produit aucun fichier .archipelago")]
    NoOutput,

    /// Le zip produit est dangereux ou sort du dossier de sortie.
    #[error("sortie du générateur refusée : {}", .0.display())]
    UnsafeOutput(std::path::PathBuf),

    /// La seed produite est illisible.
    #[error("seed produite illisible : {0}")]
    Seed(String),
}

/// Apworlds posés par la Bibliothèque dans `worlds_dir` (profils du catalogue, hash vérifié).
///
/// « Créer une partie » les ajoute tout seul à la génération : le joueur n'a pas à les
/// rechoisir. Seuls les fichiers ordinaires `*.apworld` comptent (pas les dossiers ni les
/// liens) ; un dossier absent ou illisible donne une liste vide. Ordre trié, donc stable.
pub fn installed_apworlds(worlds_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(worlds_dir) else {
        return Vec::new();
    };
    let mut found: Vec<std::path::PathBuf> = entries
        // `flatten` saute les entrées illisibles (un `Result` en erreur ne donne rien).
        .flatten()
        .filter(|entry| {
            // `file_type` ne suit pas les liens : un lien n'est pas un fichier ordinaire.
            entry.file_type().is_ok_and(|t| t.is_file())
                && entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("apworld"))
        })
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::installed_apworlds;

    #[test]
    fn apworlds_installes_listes_et_tries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("zeta.apworld"), b"z").unwrap();
        std::fs::write(dir.path().join("alpha.APWORLD"), b"a").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"n").unwrap();
        std::fs::create_dir(dir.path().join("dossier.apworld")).unwrap();
        let found = installed_apworlds(dir.path());
        assert_eq!(
            found,
            vec![
                dir.path().join("alpha.APWORLD"),
                dir.path().join("zeta.apworld")
            ]
        );
        // Dossier absent : liste vide, pas d'erreur.
        assert!(installed_apworlds(&dir.path().join("absent")).is_empty());
    }
}
