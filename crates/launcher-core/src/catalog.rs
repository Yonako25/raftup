//! Tous les profils embarqués dans le binaire (ADR 0015, décision 3).
//!
//! La liste `EMBEDDED` est écrite à la main avec `include_str!` : le texte des profils est
//! copié dans le binaire à la compilation. Un test compare cette liste au dossier
//! `profiles/` et échoue si on ajoute un profil sans le référencer ici.
//! La liste blanche des domaines vit dans `profile::ALLOWED_HOSTS` (réexportée ici).

use std::collections::BTreeMap;

pub use crate::profile::ALLOWED_HOSTS;
use crate::profile::{Profile, ProfileError};

/// Profils embarqués : (nom de fichier, contenu).
/// Vide pour l'instant : le profil de Slime Rancher attend l'autorisation de son auteur.
pub const EMBEDDED: &[(&str, &str)] = &[
    (
        "a-short-hike.toml",
        include_str!("../../../profiles/a-short-hike.toml"),
    ),
    (
        "aquaria.toml",
        include_str!("../../../profiles/aquaria.toml"),
    ),
    (
        "blasphemous.toml",
        include_str!("../../../profiles/blasphemous.toml"),
    ),
    (
        "bomb-rush-cyberfunk.toml",
        include_str!("../../../profiles/bomb-rush-cyberfunk.toml"),
    ),
    (
        "choo-choo-charles.toml",
        include_str!("../../../profiles/choo-choo-charles.toml"),
    ),
    (
        "hollow-knight.toml",
        include_str!("../../../profiles/hollow-knight.toml"),
    ),
    (
        "risk-of-rain-2.toml",
        include_str!("../../../profiles/risk-of-rain-2.toml"),
    ),
    (
        "slime-rancher.toml",
        include_str!("../../../profiles/slime-rancher.toml"),
    ),
    (
        "stardew-valley.toml",
        include_str!("../../../profiles/stardew-valley.toml"),
    ),
    (
        "the-planet-crafter.toml",
        include_str!("../../../profiles/the-planet-crafter.toml"),
    ),
];

/// L'ensemble des profils, rangés par `game.id` (`BTreeMap` : ordre stable).
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub profiles: BTreeMap<String, Profile>,
}

impl Catalog {
    /// Le catalogue embarqué dans le binaire.
    pub fn embedded() -> Result<Catalog, ProfileError> {
        Catalog::from_texts(EMBEDDED)
    }

    /// Construit un catalogue à partir de couples (nom de fichier, contenu).
    /// Chaque profil est validé ; deux profils de même `game.id` sont refusés.
    pub fn from_texts(texts: &[(&str, &str)]) -> Result<Catalog, ProfileError> {
        let mut profiles = BTreeMap::new();
        for (file, text) in texts {
            let profile = Profile::parse(file, text)?;
            let id = profile.game.id.clone();
            if profiles.insert(id.clone(), profile).is_some() {
                return Err(ProfileError::Invalid {
                    file: (*file).to_string(),
                    reason: format!("game.id « {id} » déjà présent dans le catalogue"),
                });
            }
        }
        Ok(Catalog { profiles })
    }

    /// Le profil d'un jeu, par `game.id`.
    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.profiles.get(id)
    }

    /// Le profil d'un jeu, par nom Archipelago (le premier dans l'ordre des id).
    pub fn by_ap_game(&self, name: &str) -> Option<&Profile> {
        self.profiles.values().find(|p| p.game.ap_game == name)
    }

    /// Tous les profils, triés par `game.id`.
    pub fn games(&self) -> impl Iterator<Item = &Profile> {
        self.profiles.values()
    }
}
