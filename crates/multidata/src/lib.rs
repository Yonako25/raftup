//! Modèle de partie et lecture sécurisée des fichiers `.archipelago` (ADR 0004).
//!
//! - [`pickle`] : lecteur de pickle Python qui n'exécute jamais de code ;
//! - [`archive`] : lecture d'un fichier `.archipelago` (version, zlib, pickle),
//!   avec une limite de taille (NF08).
//!
//! Le modèle de partie typé arrive à l'étape 2 de J1.

pub mod archive;
pub mod error;
pub mod pickle;

pub use archive::{read_archipelago, ArchiveLimits, MAX_FORMAT_VERSION};
pub use error::{ArchiveError, PickleError, PickleErrorKind};
pub use pickle::{Global, Limits, Node, NodeId, PickleDocument, PyClass};
