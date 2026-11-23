//! Chiffrement au repos des mots de passe des parties hébergées (ADR 0052, point 8).
//!
//! Le serveur de jeu a besoin du mot de passe **en clair** pour le comparer à celui
//! d'un joueur ; un hachage ne suffit donc pas. Le hub le chiffre avec une clé à lui,
//! rangée dans son dossier de données (`games.key`), jamais dans la base : une copie
//! de la base seule (sauvegarde PostgreSQL…) ne donne aucun mot de passe.
//!
//! Algorithme : ChaCha20-Poly1305 (chiffrement authentifié) de `ring`, déjà le moteur
//! de rustls dans tout le dépôt. Chaque valeur a son nonce aléatoire de 12 octets.
//! Les « données associées » (AAD) lient le texte chiffré à sa partie et à son
//! usage : recopier le mot de passe d'une partie dans une autre ligne le rend
//! illisible au lieu de le faire accepter.
//!
//! Forme rangée en base : `hex(nonce || texte chiffré || étiquette)`.

use std::path::Path;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

use crate::error::HubError;

/// Nom du fichier de la clé, dans le dossier des parties.
pub const KEY_FILE: &str = "games.key";
/// Taille de la clé ChaCha20-Poly1305, en octets.
const KEY_LEN: usize = 32;

/// Clé de chiffrement des mots de passe des parties.
///
/// Pas de `Debug` dérivé : il afficherait la clé.
pub struct GameKey {
    key: LessSafeKey,
}

impl std::fmt::Debug for GameKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GameKey(***)")
    }
}

impl GameKey {
    /// Relit la clé de `dir`, ou la crée (32 octets tirés par le système) si le
    /// fichier n'existe pas. Un fichier présent mais illisible est une erreur : en
    /// refaire un rendrait illisibles les mots de passe déjà rangés.
    pub fn load_or_create(dir: &Path) -> Result<GameKey, HubError> {
        let path = dir.join(KEY_FILE);
        let bytes = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| HubError::Internal(format!("clé des parties illisible : {e}")))?;
            from_hex(text.trim())
                .filter(|bytes| bytes.len() == KEY_LEN)
                .ok_or_else(|| HubError::Internal("clé des parties mal formée".to_string()))?
        } else {
            let mut bytes = vec![0u8; KEY_LEN];
            SystemRandom::new()
                .fill(&mut bytes)
                .map_err(|_| HubError::Internal("aléa du système indisponible".to_string()))?;
            std::fs::create_dir_all(dir)
                .map_err(|e| HubError::Internal(format!("dossier des parties : {e}")))?;
            write_private(&path, to_hex(&bytes).as_bytes())?;
            tracing::info!("clé de chiffrement des parties créée");
            bytes
        };
        GameKey::from_bytes(&bytes)
    }

    fn from_bytes(bytes: &[u8]) -> Result<GameKey, HubError> {
        let unbound = UnboundKey::new(&CHACHA20_POLY1305, bytes)
            .map_err(|_| HubError::Internal("clé des parties refusée".to_string()))?;
        Ok(GameKey {
            key: LessSafeKey::new(unbound),
        })
    }

    /// Chiffre `plain` pour la partie `game_id` et l'usage `purpose`
    /// (`"password"`, `"admin_password"`).
    pub fn seal(&self, game_id: &str, purpose: &str, plain: &str) -> Result<String, HubError> {
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| HubError::Internal("aléa du système indisponible".to_string()))?;
        // Le texte est chiffré « sur place » : `in_out` contient le clair, puis le
        // chiffré suivi de l'étiquette d'authentification (16 octets).
        let mut in_out = plain.as_bytes().to_vec();
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(associated_data(game_id, purpose).as_bytes()),
                &mut in_out,
            )
            .map_err(|_| HubError::Internal("chiffrement impossible".to_string()))?;
        let mut stored = nonce.to_vec();
        stored.extend_from_slice(&in_out);
        Ok(to_hex(&stored))
    }

    /// Déchiffre une valeur de [`GameKey::seal`]. Erreur si la clé, la partie ou
    /// l'usage ne sont pas les mêmes, ou si la valeur a été modifiée.
    pub fn open(&self, game_id: &str, purpose: &str, stored: &str) -> Result<String, HubError> {
        let refused = || HubError::Internal("mot de passe de partie indéchiffrable".to_string());
        let bytes = from_hex(stored).ok_or_else(refused)?;
        if bytes.len() < NONCE_LEN {
            return Err(refused());
        }
        let (nonce, sealed) = bytes.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| refused())?;
        let mut in_out = sealed.to_vec();
        // `open_in_place` vérifie l'étiquette, puis rend la tranche du texte clair.
        let plain = self
            .key
            .open_in_place(
                nonce,
                Aad::from(associated_data(game_id, purpose).as_bytes()),
                &mut in_out,
            )
            .map_err(|_| refused())?;
        String::from_utf8(plain.to_vec()).map_err(|_| refused())
    }
}

/// Données associées : la partie et l'usage, séparés par un caractère qui
/// n'apparaît ni dans un identifiant ni dans un usage.
fn associated_data(game_id: &str, purpose: &str) -> String {
    format!("raftup-game:{game_id}:{purpose}")
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// Écrit un fichier secret : sous Unix, lisible par le seul compte du hub (600).
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), HubError> {
    let error = |e: std::io::Error| HubError::Internal(format!("écriture de la clé : {e}"));
    // `#[cfg(unix)]` : ce bloc n'existe que sous Linux / macOS (droits POSIX).
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(error)?;
        file.write_all(bytes).map_err(error)?;
        file.sync_all().map_err(error)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes).map_err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> GameKey {
        GameKey::from_bytes(&[7u8; KEY_LEN]).unwrap()
    }

    #[test]
    fn aller_retour() {
        let key = key();
        let sealed = key.seal("abc", "password", "secret").unwrap();
        assert!(!sealed.contains("secret"));
        assert_eq!(
            key.open("abc", "password", &sealed).ok().as_deref(),
            Some("secret")
        );
        // Deux chiffrements du même texte ne se ressemblent pas (nonce aléatoire).
        let again = key.seal("abc", "password", "secret").unwrap();
        assert_ne!(sealed, again);
    }

    #[test]
    fn autre_partie_autre_usage_ou_valeur_modifiee_refuses() {
        let key = key();
        let sealed = key.seal("abc", "password", "secret").unwrap();
        assert!(key.open("abd", "password", &sealed).is_err());
        assert!(key.open("abc", "admin_password", &sealed).is_err());
        let mut tampered = sealed.into_bytes();
        let last = tampered.len() - 1;
        tampered[last] = if tampered[last] == b'0' { b'1' } else { b'0' };
        let tampered = String::from_utf8(tampered).unwrap();
        assert!(key.open("abc", "password", &tampered).is_err());
        assert!(key.open("abc", "password", "zz").is_err());
    }

    #[test]
    fn cle_gardee_sur_le_disque() {
        let dir = tempfile::tempdir().unwrap();
        let first = GameKey::load_or_create(dir.path()).unwrap();
        let sealed = first.seal("id", "password", "pw").unwrap();
        let second = GameKey::load_or_create(dir.path()).unwrap();
        assert_eq!(
            second.open("id", "password", &sealed).ok().as_deref(),
            Some("pw")
        );
        // Une clé abîmée n'est pas remplacée en silence.
        std::fs::write(dir.path().join(KEY_FILE), "pas une clé").unwrap();
        assert!(GameKey::load_or_create(dir.path()).is_err());
    }
}
