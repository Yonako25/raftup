//! Modèle de partie et lecture sécurisée des fichiers `.archipelago` (ADR 0004).
//!
//! - [`pickle`] : lecteur de pickle Python qui n'exécute jamais de code ;
//! - [`archive`] : lecture d'un fichier `.archipelago` (version, zlib, pickle),
//!   avec une limite de taille (NF08).
//! - [`model`] : la partie avec des types Rust ([`MultiData`]), construite par
//!   [`load_archipelago`] ;
//! - [`raftup`] : format de sauvegarde `.raftup` (instantané + deltas) ;
//! - [`file`] : lecture d'un fichier avec une taille maximale ;
//! - [`pickle_writer`] et [`write_archipelago`] : écriture d'une seed
//!   `.archipelago` à partir du modèle (générateur Rust, ADR 0027).

pub mod archive;
mod convert;
pub mod error;
mod export;
pub mod file;
pub mod model;
pub mod pickle;
pub mod pickle_writer;
pub mod raftup;

pub use archive::{
    read_archipelago, write_archipelago, ArchiveLimits, MAX_FORMAT_VERSION, WRITTEN_FORMAT_VERSION,
};
pub use error::{ArchiveError, LoadError, ModelError, PickleError, PickleErrorKind, WriteError};
pub use file::{read_file_bounded, MAX_SEED_FILE_BYTES};
pub use model::{GamePackage, MultiData, PlacedItem, ServerOptions};
pub use pickle::{Global, Limits, Node, NodeId, PickleDocument, PyClass};
pub use pickle_writer::PyValue;

/// Lit un fichier `.archipelago` et construit le modèle de partie.
pub fn load_archipelago(bytes: &[u8], limits: &ArchiveLimits) -> Result<MultiData, LoadError> {
    let doc = read_archipelago(bytes, limits)?;
    Ok(MultiData::from_document(&doc)?)
}
