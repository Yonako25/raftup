//! Registre de paquets signés de Raftup (V2, ADR 0032).
//!
//! Le registre est un **index** : une liste de paquets (apworlds et profils d'installation)
//! avec, pour chacun, l'adresse chez son auteur et son sha256. Raftup ne redistribue rien :
//! jamais de jeu, de ROM, d'asset ni de mod hébergé par Raftup.
//!
//! - [`index`] : le format de l'index (`format_version`), sa lecture et sa validation ;
//! - [`verify`] : la signature minisign de l'index et le contrôle d'un paquet téléchargé ;
//! - [`publish`] (fonctionnalité `publish`) : construire, signer et écrire un index en local.
//!
//! Cette crate n'ouvre aucune connexion réseau.

pub mod error;
pub mod index;
#[cfg(feature = "publish")]
pub mod publish;
pub mod verify;

pub use error::RegistryError;
pub use index::{
    Index, Package, PackageKind, Trust, TrustLevel, ALLOWED_HOSTS, FORMAT_VERSION,
    NATIVE_WORLD_FORMAT, READABLE_FORMATS,
};
pub use verify::{check_package, sha256_hex, verify_index, INDEX_MAX_BYTES, SIGNATURE_MAX_BYTES};
