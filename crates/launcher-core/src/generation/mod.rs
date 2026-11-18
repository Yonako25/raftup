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
