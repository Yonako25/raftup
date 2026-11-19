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

/// Alphabet des codes d'invitation : base32 de Crockford (pas de I, L, O, U, qui se
/// confondent avec 1, 0 ou V à la lecture).
const INVITE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Nouveau code d'invitation, lisible et facile à recopier : 16 caractères en
/// quatre groupes (`K7QX-2M9P-…`), soit 80 bits d'aléa.
pub fn new_invite_code() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    let mut code = String::new();
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            code.push('-');
        }
        // 5 bits par caractère : 256 / 32 = 8, donc chaque lettre a la même chance.
        code.push(char::from(INVITE_ALPHABET[usize::from(byte % 32)]));
    }
    code
}

/// Forme rangée d'un code tapé par un joueur : majuscules, sans tirets ni espaces,
/// O lu comme 0 et I ou L lus comme 1. C'est cette forme qui est hachée.
pub fn normalize_invite_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            other => other,
        })
        .collect()
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

    #[test]
    fn codes_d_invitation() {
        let code = new_invite_code();
        assert_eq!(code.len(), 19);
        assert_eq!(code.matches('-').count(), 3);
        assert_ne!(code, new_invite_code());
        // Tapé en minuscules, avec des espaces : même forme rangée.
        let typed = code.to_lowercase().replace('-', " ");
        assert_eq!(normalize_invite_code(&typed), normalize_invite_code(&code));
        assert_eq!(normalize_invite_code("o0-il"), "0011");
    }
}
