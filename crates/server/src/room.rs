//! La salle : une partie chargée, et ce que le serveur en dit aux clients.
//!
//! Pour l'instant, la salle ne change pas pendant la partie : elle ne fait que
//! lire la seed. L'état qui évolue (lieux visités, objets reçus…) viendra aux
//! étapes suivantes.

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use ap_protocol::server::RoomInfo;
use ap_protocol::{NetworkVersion, Permission};
use indexmap::IndexMap;
use multidata::MultiData;

/// Version du protocole annoncée aux clients : celle du serveur officiel imité.
pub const SERVER_VERSION: NetworkVersion = NetworkVersion {
    major: 0,
    minor: 6,
    build: 8,
};

/// Nom du « jeu » commun à toutes les parties (objets et lieux spéciaux).
pub const COMMON_GAME: &str = "Archipelago";

/// Une partie prête à être servie.
#[derive(Debug, Clone)]
pub struct Room {
    data: MultiData,
    /// Mot de passe de la salle : celui de la seed, ou celui donné au lancement.
    password: Option<String>,
}

impl Room {
    /// Prépare la salle à partir d'une seed chargée.
    pub fn new(data: MultiData) -> Room {
        // Un mot de passe vide revient à ne pas en avoir, comme dans le serveur officiel.
        let password = data
            .server_options
            .password
            .clone()
            .filter(|p| !p.is_empty());
        Room { data, password }
    }

    /// Remplace le mot de passe de la seed (option `--password` du serveur).
    pub fn with_password(mut self, password: Option<String>) -> Room {
        self.password = password.filter(|p| !p.is_empty());
        self
    }

    /// La partie chargée.
    pub fn data(&self) -> &MultiData {
        &self.data
    }

    /// Mot de passe de la salle, s'il y en a un.
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// Jeux présents dans la partie, triés, avec le jeu commun `Archipelago`.
    pub fn played_games(&self) -> Vec<String> {
        // BTreeSet : trie et retire les doublons (plusieurs joueurs sur le même jeu).
        let mut games: BTreeSet<String> = BTreeSet::new();
        games.insert(COMMON_GAME.to_string());
        for slot in self.data.slots.values() {
            games.insert(slot.game.clone());
        }
        games.into_iter().collect()
    }

    /// Premier paquet envoyé à chaque client, dès l'ouverture de la connexion.
    ///
    /// `time` : l'heure à annoncer (secondes depuis 1970) ; voir [`now`].
    pub fn room_info(&self, time: f64) -> RoomInfo {
        let options = &self.data.server_options;
        let mut permissions = IndexMap::new();
        permissions.insert(
            "release".to_string(),
            permission_from_text(&options.release_mode),
        );
        permissions.insert(
            "remaining".to_string(),
            permission_from_text(&options.remaining_mode),
        );
        permissions.insert(
            "collect".to_string(),
            permission_from_text(&options.collect_mode),
        );

        let games = self.played_games();
        // Empreinte de chaque jeu joué, dans l'ordre des jeux (triés).
        let mut checksums = IndexMap::new();
        for game in &games {
            if let Some(package) = self.data.datapackage.get(game) {
                if !package.checksum.is_empty() {
                    checksums.insert(game.clone(), package.checksum.clone());
                }
            }
        }

        RoomInfo {
            version: SERVER_VERSION,
            generator_version: self.data.generator_version,
            tags: vec!["AP".to_string()],
            password: self.password.is_some(),
            permissions,
            hint_cost: to_u32(options.hint_cost),
            location_check_points: to_u32(options.location_check_points),
            games,
            datapackage_checksums: checksums,
            seed_name: self.data.seed_name.clone(),
            time,
        }
    }
}

/// Heure actuelle en secondes depuis 1970, comme `time.time()` en Python.
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Traduit un mode (`auto`, `enabled`, `goal`, `auto-enabled`, `disabled`)
/// en droits, exactement comme `Permission.from_text` du serveur officiel :
/// on cherche des morceaux de texte, pas une valeur exacte.
pub fn permission_from_text(text: &str) -> Permission {
    let mut bits = 0;
    if text.contains("auto") {
        bits |= 0b110;
    } else if text.contains("goal") {
        bits |= 0b010;
    }
    if text.contains("enabled") {
        bits |= 0b001;
    }
    Permission(bits)
}

/// Ramène un nombre de la seed dans les bornes d'un `u32` (0 si négatif).
fn to_u32(value: i64) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_comme_python() {
        assert_eq!(permission_from_text("auto"), Permission::AUTO);
        assert_eq!(
            permission_from_text("auto-enabled"),
            Permission::AUTO_ENABLED
        );
        assert_eq!(permission_from_text("enabled"), Permission::ENABLED);
        assert_eq!(permission_from_text("goal"), Permission::GOAL);
        assert_eq!(permission_from_text("disabled"), Permission::DISABLED);
    }

    #[test]
    fn bornes_u32() {
        assert_eq!(to_u32(-5), 0);
        assert_eq!(to_u32(10), 10);
        assert_eq!(to_u32(i64::MAX), u32::MAX);
    }
}
