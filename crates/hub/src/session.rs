//! Jetons de session (ADR 0033).
//!
//! Le jeton est une chaîne aléatoire opaque (32 octets écrits en hexadécimal).
//! La base ne garde que son empreinte SHA-256 : une copie volée de la base ne
//! permet pas d'ouvrir une session. Révoquer = supprimer la ligne.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

/// Nouveau jeton aléatoire, tiré par le générateur du système.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    to_hex(&bytes)
}

/// Empreinte SHA-256 du jeton, celle qui est stockée en base.
pub fn token_hash(token: &str) -> String {
    to_hex(&Sha256::digest(token.as_bytes()))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jetons_differents_et_empreinte_stable() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(token_hash(&a), token_hash(&a));
        assert_ne!(token_hash(&a), a);
    }
}
