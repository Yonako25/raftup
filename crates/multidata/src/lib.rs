//! Modèle de partie et lecture sécurisée des fichiers `.archipelago` (ADR 0004).
//!
//! - [`pickle`] : lecteur de pickle Python qui n'exécute jamais de code ;
//! - [`archive`] : lecture d'un fichier `.archipelago` (version, zlib, pickle),
//!   avec une limite de taille (NF08).
//! - [`model`] : la partie avec des types Rust ([`MultiData`]), construite par
//!   [`load_archipelago`] ;
//! - [`raftup`] : format de sauvegarde `.raftup` (instantané + deltas).

pub mod archive;
mod convert;
pub mod error;
pub mod model;
pub mod pickle;
pub mod raftup;

pub use archive::{read_archipelago, ArchiveLimits, MAX_FORMAT_VERSION};
pub use error::{ArchiveError, LoadError, ModelError, PickleError, PickleErrorKind};
pub use model::{GamePackage, MultiData, PlacedItem, ServerOptions};
pub use pickle::{Global, Limits, Node, NodeId, PickleDocument, PyClass};

/// Lit un fichier `.archipelago` et construit le modèle de partie.
pub fn load_archipelago(bytes: &[u8], limits: &ArchiveLimits) -> Result<MultiData, LoadError> {
    let doc = read_archipelago(bytes, limits)?;
    Ok(MultiData::from_document(&doc)?)
}
