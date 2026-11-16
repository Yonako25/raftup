//! Lecture d'une seed `.archipelago` pour l'écran « Héberger une partie ».
//!
//! La lecture est sûre (aucun code exécuté, NF08) : c'est celle de `multidata`.

use std::path::Path;

use multidata::{
    load_archipelago, read_file_bounded, ArchiveLimits, MultiData, MAX_SEED_FILE_BYTES,
};
use serde::Serialize;

use crate::error::LauncherError;

/// Ce qu'on montre d'une seed avant de démarrer le serveur.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeedSummary {
    pub seed_name: String,
    /// Version d'Archipelago qui a généré la seed : `0.6.8`.
    pub generator_version: String,
    pub slots: Vec<SeedSlot>,
    /// Nombre de vrais joueurs (sans les groupes).
    pub players: usize,
    /// La seed impose un mot de passe.
    pub has_password: bool,
    pub release_mode: String,
    pub collect_mode: String,
    /// Coût d'un indice, en pourcentage des lieux.
    pub hint_cost: i64,
}

/// Un slot de la seed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeedSlot {
    pub slot: u32,
    pub name: String,
    pub game: String,
    /// Vrai pour un groupe (« item link ») : il ne joue pas lui-même.
    pub is_group: bool,
    /// Pour un groupe : les noms de ses membres.
    pub members: Vec<String>,
    /// Nombre de lieux du slot.
    pub locations: usize,
}

impl SeedSummary {
    /// Lit et résume une seed.
    pub fn load(path: &Path) -> Result<(SeedSummary, MultiData), LauncherError> {
        let bytes =
            read_file_bounded(path, MAX_SEED_FILE_BYTES).map_err(|source| LauncherError::Read {
                path: path.display().to_string(),
                source,
            })?;
        let data = load_archipelago(&bytes, &ArchiveLimits::default())?;
        Ok((SeedSummary::of(&data), data))
    }

    /// Résume une seed déjà chargée.
    pub fn of(data: &MultiData) -> SeedSummary {
        let mut slots = Vec::new();
        for (slot, info) in &data.slots {
            let members = info
                .group_members
                .iter()
                .filter_map(|m| data.slots.get(m).map(|s| s.name.clone()))
                .collect();
            slots.push(SeedSlot {
                slot: *slot,
                name: info.name.clone(),
                game: info.game.clone(),
                is_group: !info.group_members.is_empty(),
                members,
                locations: data.locations.get(slot).map_or(0, |l| l.len()),
            });
        }
        let version = data.generator_version;
        let options = &data.server_options;
        SeedSummary {
            seed_name: data.seed_name.clone(),
            generator_version: format!("{}.{}.{}", version.major, version.minor, version.build),
            players: slots.iter().filter(|s| !s.is_group).count(),
            slots,
            has_password: options.password.as_deref().is_some_and(|p| !p.is_empty()),
            release_mode: options.release_mode.clone(),
            collect_mode: options.collect_mode.clone(),
            hint_cost: options.hint_cost,
        }
    }
}
