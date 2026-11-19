//! Vérification de l'index signé et des paquets téléchargés (ADR 0032).
//!
//! L'ordre compte : la signature est vérifiée **avant** de lire le JSON. Un index qui ne
//! vient pas de la mainteneuse n'est jamais interprété.

use minisign_verify::{PublicKey, Signature};
use sha2::{Digest, Sha256};

use crate::error::RegistryError;
use crate::index::{Index, Package};

/// Taille maximale de `index.json` (4 Mio).
pub const INDEX_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Taille maximale de `index.json.minisig` (un fichier minisign fait environ 300 octets).
pub const SIGNATURE_MAX_BYTES: u64 = 4 * 1024;

/// Vérifie la signature minisign de `index_bytes`, puis lit et valide l'index.
///
/// `public_key` : ligne base64 de la clé publique (commence par « RW »). Une clé vide fait
/// tout refuser (`PublicKeyNotConfigured`).
pub fn verify_index(
    index_bytes: &[u8],
    signature: &str,
    public_key: &str,
) -> Result<Index, RegistryError> {
    if index_bytes.len() as u64 > INDEX_MAX_BYTES {
        return Err(RegistryError::TooLarge {
            what: "index",
            max: INDEX_MAX_BYTES,
        });
    }
    if signature.len() as u64 > SIGNATURE_MAX_BYTES {
        return Err(RegistryError::TooLarge {
            what: "signature",
            max: SIGNATURE_MAX_BYTES,
        });
    }
    let key = load_public_key(public_key)?;
    let signature =
        Signature::decode(signature).map_err(|e| RegistryError::BadSignature(e.to_string()))?;
    // `false` : on refuse les signatures « legacy » (non pré-hachées), plus faibles.
    key.verify(index_bytes, &signature, false)
        .map_err(|_| RegistryError::SignatureMismatch)?;
    Index::parse(index_bytes)
}

/// Contrôle les octets téléchargés d'un paquet : taille puis sha256 annoncés par l'index.
pub fn check_package(package: &Package, data: &[u8]) -> Result<(), RegistryError> {
    let label = format!("{} {}", package.kind.as_str(), package.name);
    let found = data.len() as u64;
    if found != package.size {
        return Err(RegistryError::SizeMismatch {
            package: label,
            expected: package.size,
            found,
        });
    }
    let digest = sha256_hex(data);
    if digest != package.sha256 {
        return Err(RegistryError::HashMismatch {
            package: label,
            expected: package.sha256.clone(),
            found: digest,
        });
    }
    Ok(())
}

/// sha256 en hexadécimal minuscule.
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    // `fold` : on ajoute chaque octet en deux caractères hexadécimaux à la même chaîne.
    digest.iter().fold(String::with_capacity(64), |mut out, b| {
        out.push_str(&format!("{b:02x}"));
        out
    })
}

/// Lit la clé publique minisign (ligne base64).
fn load_public_key(public_key: &str) -> Result<PublicKey, RegistryError> {
    let trimmed = public_key.trim();
    if trimmed.is_empty() {
        return Err(RegistryError::PublicKeyNotConfigured);
    }
    PublicKey::from_base64(trimmed).map_err(|e| RegistryError::BadPublicKey(e.to_string()))
}
