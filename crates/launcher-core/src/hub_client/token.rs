//! Où le launcher range le jeton de session du hub (ADR 0002, 0035).
//!
//! - [`KeyringStore`] : le trousseau du système (Gestionnaire d'identification Windows,
//!   Trousseau macOS, keyutils du noyau Linux), jamais un fichier en clair ;
//! - [`MemoryStore`] : en mémoire, pour les tests et quand le trousseau est indisponible.
//!
//! Un jeton est rangé **par adresse de hub** : changer d'adresse ne renvoie pas le jeton
//! d'un hub à un autre.

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::types::SessionToken;
use super::HubUrl;

/// Nom du « service » dans le trousseau du système.
const KEYRING_SERVICE: &str = "raftup-hub";

/// Le trousseau a refusé une opération (message du système, sans le jeton).
#[derive(Debug, thiserror::Error)]
#[error("trousseau du système : {0}")]
pub struct TokenStoreError(pub String);

/// Un endroit où garder le jeton entre deux lancements.
///
/// `Send + Sync` : le magasin passe dans le thread réseau de l'interface.
pub trait TokenStore: Send + Sync {
    /// Le jeton rangé pour ce hub, s'il y en a un.
    fn load(&self, hub: &HubUrl) -> Result<Option<SessionToken>, TokenStoreError>;
    /// Range (ou remplace) le jeton de ce hub.
    fn save(&self, hub: &HubUrl, token: &SessionToken) -> Result<(), TokenStoreError>;
    /// Oublie le jeton de ce hub ; rien à oublier n'est pas une erreur.
    fn clear(&self, hub: &HubUrl) -> Result<(), TokenStoreError>;
}

/// Le trousseau du système, par la crate `keyring`.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

impl KeyringStore {
    fn entry(hub: &HubUrl) -> Result<keyring::Entry, TokenStoreError> {
        // Le « compte » de l'entrée est l'adresse du hub : un jeton par hub.
        keyring::Entry::new(KEYRING_SERVICE, hub.as_str()).map_err(store_error)
    }
}

/// Message du trousseau, sans jamais le jeton (`keyring` ne le recopie pas).
fn store_error(error: keyring::Error) -> TokenStoreError {
    TokenStoreError(error.to_string())
}

impl TokenStore for KeyringStore {
    fn load(&self, hub: &HubUrl) -> Result<Option<SessionToken>, TokenStoreError> {
        match Self::entry(hub)?.get_password() {
            Ok(token) => Ok(Some(SessionToken::new(token))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(other) => Err(store_error(other)),
        }
    }

    fn save(&self, hub: &HubUrl, token: &SessionToken) -> Result<(), TokenStoreError> {
        Self::entry(hub)?
            .set_password(token.expose())
            .map_err(store_error)
    }

    fn clear(&self, hub: &HubUrl) -> Result<(), TokenStoreError> {
        match Self::entry(hub)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(other) => Err(store_error(other)),
        }
    }
}

/// Jetons gardés en mémoire seulement : perdus à la fermeture du launcher.
#[derive(Debug, Default)]
pub struct MemoryStore {
    // `Mutex` : `TokenStore` prend `&self`, et le magasin peut être partagé entre threads ;
    // le verrou permet quand même de modifier la table.
    tokens: Mutex<BTreeMap<String, SessionToken>>,
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, SessionToken>> {
        // Un thread qui a paniqué en tenant le verrou ne rend pas la table inutilisable.
        self.tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl TokenStore for MemoryStore {
    fn load(&self, hub: &HubUrl) -> Result<Option<SessionToken>, TokenStoreError> {
        Ok(self.lock().get(hub.as_str()).cloned())
    }

    fn save(&self, hub: &HubUrl, token: &SessionToken) -> Result<(), TokenStoreError> {
        self.lock().insert(hub.as_str().to_string(), token.clone());
        Ok(())
    }

    fn clear(&self, hub: &HubUrl) -> Result<(), TokenStoreError> {
        self.lock().remove(hub.as_str());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memoire_un_jeton_par_hub() {
        let store = MemoryStore::new();
        let a = HubUrl::parse("http://127.0.0.1:8787").unwrap();
        let b = HubUrl::parse("https://hub.example.org").unwrap();
        store.save(&a, &SessionToken::new("jeton-a")).unwrap();
        assert_eq!(store.load(&a).unwrap(), Some(SessionToken::new("jeton-a")));
        assert_eq!(store.load(&b).unwrap(), None);
        store.clear(&a).unwrap();
        assert_eq!(store.load(&a).unwrap(), None);
        // Oublier deux fois n'est pas une erreur.
        store.clear(&a).unwrap();
    }

    /// Le vrai trousseau du système : écrit, relit puis efface une entrée de test.
    #[test]
    #[ignore = "touche au trousseau du système"]
    fn reel_trousseau_aller_retour() {
        let store = KeyringStore;
        let hub = HubUrl::parse("https://test-du-trousseau.invalid").unwrap();
        store
            .save(&hub, &SessionToken::new("jeton-de-test"))
            .unwrap();
        assert_eq!(
            store.load(&hub).unwrap(),
            Some(SessionToken::new("jeton-de-test"))
        );
        store.clear(&hub).unwrap();
        assert_eq!(store.load(&hub).unwrap(), None);
        store.clear(&hub).unwrap();
    }

    #[test]
    fn jeton_masque_dans_debug() {
        let token = SessionToken::new("tres-secret");
        assert!(!format!("{token:?}").contains("tres-secret"));
    }
}
