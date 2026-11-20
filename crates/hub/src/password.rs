//! Mots de passe : hachage argon2id et vérification (NF09).
//!
//! Le mot de passe en clair ne vit que dans un [`Password`], dont l'affichage de
//! débogage est masqué : il ne peut pas finir dans un journal par erreur.

use std::fmt;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use serde::Deserialize;

use crate::error::HubError;

/// Longueur minimale d'un mot de passe, en caractères.
pub const MIN_PASSWORD_CHARS: usize = 8;
/// Longueur maximale : évite de hacher des mégaoctets envoyés exprès.
pub const MAX_PASSWORD_CHARS: usize = 128;

/// Mot de passe en clair, reçu du client.
///
/// `#[serde(transparent)]` : dans le JSON, c'est une simple chaîne.
#[derive(Clone, Deserialize)]
#[serde(transparent)]
pub struct Password(String);

impl Password {
    pub fn new(value: impl Into<String>) -> Self {
        Password(value.into())
    }

    /// Vérifie la longueur (comptée en caractères, pas en octets).
    pub fn validate(&self) -> Result<(), HubError> {
        let chars = self.0.chars().count();
        if chars < MIN_PASSWORD_CHARS {
            return Err(HubError::WeakPassword("8 caractères au moins"));
        }
        if chars > MAX_PASSWORD_CHARS {
            return Err(HubError::WeakPassword("128 caractères au plus"));
        }
        Ok(())
    }

    /// Vrai si les deux saisies sont identiques (champ « confirmer le mot de passe »).
    pub fn same_as(&self, other: &Password) -> bool {
        self.0 == other.0
    }

    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// Affichage de débogage masqué : `{:?}` écrit `Password(***)`.
impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(***)")
    }
}

/// Coût du hachage argon2id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashParams {
    /// Mémoire utilisée, en Kio.
    pub memory_kib: u32,
    /// Nombre de passes.
    pub iterations: u32,
    /// Nombre de voies parallèles.
    pub parallelism: u32,
}

impl HashParams {
    /// Réglage de production : celui recommandé par l'OWASP pour argon2id
    /// (19 Mio, 2 passes, 1 voie), qui est aussi le réglage par défaut de la crate.
    pub fn production() -> Self {
        HashParams {
            memory_kib: 19 * 1024,
            iterations: 2,
            parallelism: 1,
        }
    }

    /// Réglage minimal, **pour les tests seulement** : en debug, le réglage de
    /// production prend plusieurs secondes par hachage.
    pub fn fast_for_tests() -> Self {
        HashParams {
            memory_kib: 8,
            iterations: 1,
            parallelism: 1,
        }
    }

    fn hasher(&self) -> Result<Argon2<'static>, HubError> {
        let params = Params::new(self.memory_kib, self.iterations, self.parallelism, None)
            .map_err(|e| HubError::Internal(format!("paramètres argon2 : {e}")))?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }
}

/// Hache un mot de passe : renvoie une chaîne au format PHC (`$argon2id$v=19$...`),
/// qui contient l'algorithme, les paramètres, le sel et le hash.
///
/// Calcul lent exprès : à appeler dans `spawn_blocking` depuis du code async.
pub fn hash_password(password: &Password, params: &HashParams) -> Result<String, HubError> {
    // Sel aléatoire de 16 octets, tiré par le générateur du système.
    let salt = SaltString::generate(&mut OsRng);
    let hash = params
        .hasher()?
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| HubError::Internal(format!("hachage argon2 : {e}")))?;
    Ok(hash.to_string())
}

/// Vérifie un mot de passe contre un hash PHC. Les paramètres sont lus dans le hash :
/// un compte créé avec un ancien réglage reste vérifiable.
pub fn verify_password(password: &Password, phc: &str) -> Result<bool, HubError> {
    let parsed =
        PasswordHash::new(phc).map_err(|e| HubError::Internal(format!("hash illisible : {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_ne_montre_pas_le_mot_de_passe() {
        let password = Password::new("très-secret-42");
        assert_eq!(format!("{password:?}"), "Password(***)");
    }

    #[test]
    fn hachage_puis_verification() {
        let params = HashParams::fast_for_tests();
        let password = Password::new("correct horse");
        let phc = hash_password(&password, &params).unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(!phc.contains("correct horse"));
        assert!(verify_password(&password, &phc).unwrap());
        assert!(!verify_password(&Password::new("wrong horse"), &phc).unwrap());
    }

    #[test]
    fn deux_hachages_ont_des_sels_differents() {
        let params = HashParams::fast_for_tests();
        let password = Password::new("même mot de passe");
        let a = hash_password(&password, &params).unwrap();
        let b = hash_password(&password, &params).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn longueurs_refusees() {
        assert!(Password::new("court").validate().is_err());
        assert!(Password::new("x".repeat(129)).validate().is_err());
        assert!(Password::new("éééééééé").validate().is_ok());
    }
}
