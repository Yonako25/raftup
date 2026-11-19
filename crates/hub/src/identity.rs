//! Pseudos et adresses e-mail : validation et forme normalisée (ADR 0002).
//!
//! L'unicité se fait sur la forme normalisée : Unicode NFKC puis minuscules.
//! `Mimi`, `mimi` et `Ｍｉｍｉ` (lettres pleine chasse) sont donc le même pseudo.

use unicode_normalization::UnicodeNormalization;

use crate::error::HubError;

pub const MIN_USERNAME_CHARS: usize = 3;
pub const MAX_USERNAME_CHARS: usize = 24;
const MAX_EMAIL_BYTES: usize = 254;

/// Pseudos interdits (comparés à la forme normalisée).
const RESERVED_USERNAMES: &[&str] = &[
    "admin",
    "administrator",
    "archipelago",
    "help",
    "mod",
    "moderator",
    "official",
    "raftup",
    "root",
    "staff",
    "support",
    "system",
];

/// Pseudo validé : forme affichée (telle que tapée) et forme normalisée.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Username {
    pub display: String,
    pub normalized: String,
}

/// Forme normalisée d'un pseudo, pour la recherche et l'unicité.
pub fn normalize_username(raw: &str) -> String {
    raw.nfkc().collect::<String>().to_lowercase()
}

fn allowed_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

/// Valide un pseudo : 3 à 24 caractères, lettres, chiffres, `_`, `-`, `.`, pas réservé.
pub fn parse_username(raw: &str) -> Result<Username, HubError> {
    let chars = raw.chars().count();
    if chars < MIN_USERNAME_CHARS {
        return Err(HubError::InvalidUsername("3 caractères au moins"));
    }
    if chars > MAX_USERNAME_CHARS {
        return Err(HubError::InvalidUsername("24 caractères au plus"));
    }
    let normalized = normalize_username(raw);
    // On vérifie aussi la forme normalisée : NFKC peut changer un caractère.
    if !raw.chars().all(allowed_char) || !normalized.chars().all(allowed_char) {
        return Err(HubError::InvalidUsername(
            "seulement des lettres, des chiffres, _, - et .",
        ));
    }
    if RESERVED_USERNAMES.contains(&normalized.as_str()) {
        return Err(HubError::InvalidUsername("pseudo réservé"));
    }
    Ok(Username {
        display: raw.to_string(),
        normalized,
    })
}

/// Valide une adresse e-mail et renvoie sa forme normalisée (espaces retirés, minuscules).
///
/// Vérification volontairement simple : la vraie preuve sera le lien de vérification
/// envoyé par e-mail (pas encore fait, ADR 0033).
pub fn normalize_email(raw: &str) -> Result<String, HubError> {
    let email = raw.trim().to_lowercase();
    if email.is_empty() || email.len() > MAX_EMAIL_BYTES {
        return Err(HubError::InvalidEmail);
    }
    if email.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(HubError::InvalidEmail);
    }
    // `split_once` coupe au premier `@` ; un deuxième `@` dans le domaine est refusé.
    let Some((local, domain)) = email.split_once('@') else {
        return Err(HubError::InvalidEmail);
    };
    if local.is_empty()
        || domain.contains('@')
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
    {
        return Err(HubError::InvalidEmail);
    }
    Ok(email)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casse_et_pleine_chasse_donnent_la_meme_forme() {
        assert_eq!(normalize_username("Mimi"), "mimi");
        assert_eq!(normalize_username("Ｍｉｍｉ"), "mimi");
    }

    #[test]
    fn pseudos_acceptes() {
        for ok in [
            "abc",
            "Mimi_42",
            "jean-luc.p",
            "Élodie",
            "a".repeat(24).as_str(),
        ] {
            assert!(parse_username(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn pseudos_refuses() {
        for bad in [
            "ab",
            "a b",
            "a@b",
            "x".repeat(25).as_str(),
            "Admin",
            "RAFTUP",
        ] {
            assert!(parse_username(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn emails() {
        assert_eq!(
            normalize_email("  Bob@Example.ORG ").unwrap(),
            "bob@example.org"
        );
        for bad in [
            "",
            "bob",
            "@example.org",
            "bob@",
            "bob@org",
            "bob@@x.org",
            "b ob@x.org",
        ] {
            assert!(normalize_email(bad).is_err(), "{bad}");
        }
    }
}
