//! Durée estimée d'une partie, par joueur (SPEC V2, ADR 0030).
//!
//! On part de la seed (`multidata`) : le nombre de lieux de chaque joueur et les
//! **sphères** (sphère 0 = ce qui est ouvert au départ, sphère 1 = ce que les objets de la
//! sphère 0 ouvrent, etc.). Chaque jeu a un temps moyen par lieu, lu dans son profil
//! (`game.seconds_per_location`) ; sans profil ou sans ce champ, on prend
//! [`DEFAULT_SECONDS_PER_LOCATION`].
//!
//! Deux chiffres par joueur :
//!
//! - **temps de jeu** : ses lieux × son temps par lieu, sans aucune attente ;
//! - **fin au plus tard** : on suppose que la sphère suivante ne s'ouvre que quand
//!   **tous** les joueurs ont fini la sphère en cours (le plus lent fixe le rythme). C'est
//!   pessimiste : en vrai, un objet arrive souvent avant la fin de la sphère.
//!
//! La vraie durée tombe entre les deux. Tout est en secondes entières : même résultat
//! partout, pas de virgule.

use std::collections::BTreeMap;
use std::path::Path;

use multidata::{
    load_archipelago, read_file_bounded, ArchiveLimits, MultiData, MAX_SEED_FILE_BYTES,
};
use serde::Serialize;

use crate::catalog::Catalog;
use crate::error::LauncherError;

/// Temps moyen d'un lieu quand le profil du jeu n'en donne pas : 2 minutes. C'est un ordre
/// de grandeur (un coffre, un achat, un petit défi), pas une mesure.
pub const DEFAULT_SECONDS_PER_LOCATION: u32 = 120;

/// Plus grande valeur acceptée dans un profil : 1 heure par lieu.
pub const MAX_SECONDS_PER_LOCATION: u32 = 3600;

/// L'estimation d'un joueur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayerEstimate {
    pub slot: u32,
    pub name: String,
    pub game: String,
    /// Nombre de lieux du joueur.
    pub locations: usize,
    /// Temps par lieu utilisé, en secondes.
    pub seconds_per_location: u32,
    /// Vrai si ce temps vient du profil du jeu (faux : valeur par défaut).
    pub from_profile: bool,
    /// Temps de jeu sans attente, en secondes.
    pub play_seconds: u64,
    /// Fin au plus tard (avec les attentes entre sphères), en secondes.
    pub finish_seconds: u64,
}

impl PlayerEstimate {
    /// Attente au plus : fin au plus tard moins le temps de jeu.
    pub fn wait_seconds(&self) -> u64 {
        self.finish_seconds.saturating_sub(self.play_seconds)
    }
}

/// L'estimation d'une seed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeedEstimate {
    /// Les vrais joueurs (sans les groupes), dans l'ordre des slots.
    pub players: Vec<PlayerEstimate>,
    /// Nombre de sphères de la seed (0 si la seed ne les donne pas).
    pub spheres: usize,
}

impl SeedEstimate {
    /// Fin au plus tard de la partie : celle du dernier joueur.
    pub fn finish_seconds(&self) -> u64 {
        self.players
            .iter()
            .map(|p| p.finish_seconds)
            .max()
            .unwrap_or(0)
    }

    /// Le plus long temps de jeu sans attente.
    pub fn longest_play_seconds(&self) -> u64 {
        self.players
            .iter()
            .map(|p| p.play_seconds)
            .max()
            .unwrap_or(0)
    }
}

/// Estime une seed avec les temps des profils du catalogue.
pub fn estimate(data: &MultiData, catalog: &Catalog) -> SeedEstimate {
    estimate_with(data, |game| {
        catalog
            .by_ap_game(game)
            .and_then(|p| p.game.seconds_per_location)
    })
}

/// Réglages de l'équilibrage par temps de jeu (ADR 0030), **activé**, pour des joueurs
/// dont on donne les jeux (noms Archipelago) dans l'ordre des slots. Le temps par lieu
/// vient de `seconds_for` (le profil du jeu), [`DEFAULT_SECONDS_PER_LOCATION`] sinon,
/// comme pour [`estimate_with`].
pub fn time_balance_settings<'a>(
    games: impl IntoIterator<Item = &'a str>,
    seconds_for: impl Fn(&str) -> Option<u32>,
) -> generate::TimeBalance {
    let seconds_per_location = games
        .into_iter()
        .map(|game| {
            let given = seconds_for(game).filter(|s| (1..=MAX_SECONDS_PER_LOCATION).contains(s));
            u64::from(given.unwrap_or(DEFAULT_SECONDS_PER_LOCATION))
        })
        .collect();
    generate::TimeBalance {
        enabled: true,
        seconds_per_location,
        // Les autres réglages (nombre d'échanges) gardent leur valeur par défaut.
        ..generate::TimeBalance::default()
    }
}

/// Comme [`time_balance_settings`], avec les temps des profils embarqués dans le
/// launcher. Si le catalogue ne se lit pas (ce qui ne devrait pas arriver : il est testé),
/// chaque jeu prend la valeur par défaut.
pub fn time_balance_embedded<'a>(
    games: impl IntoIterator<Item = &'a str>,
) -> generate::TimeBalance {
    let catalog = Catalog::embedded().ok();
    time_balance_settings(games, |game| {
        catalog
            .as_ref()
            .and_then(|c| c.by_ap_game(game))
            .and_then(|p| p.game.seconds_per_location)
    })
}

/// Lit une seed `.archipelago` (lecture sûre, NF08) et l'estime.
pub fn estimate_file(path: &Path, catalog: &Catalog) -> Result<SeedEstimate, LauncherError> {
    let bytes =
        read_file_bounded(path, MAX_SEED_FILE_BYTES).map_err(|source| LauncherError::Read {
            path: path.display().to_string(),
            source,
        })?;
    let data = load_archipelago(&bytes, &ArchiveLimits::default())?;
    Ok(estimate(&data, catalog))
}

/// Estime une seed ; `seconds_for` donne le temps par lieu d'un jeu (nom Archipelago),
/// ou `None` pour la valeur par défaut. `impl Fn` : n'importe quelle fonction ou
/// fermeture qui prend un `&str`, ce qui permet de tester sans catalogue.
pub fn estimate_with(data: &MultiData, seconds_for: impl Fn(&str) -> Option<u32>) -> SeedEstimate {
    // Les vrais joueurs : un groupe (« item link ») ne joue pas lui-même.
    let mut players: Vec<PlayerEstimate> = Vec::new();
    for (slot, info) in &data.slots {
        if !info.group_members.is_empty() {
            continue;
        }
        let given = seconds_for(&info.game).filter(|s| (1..=MAX_SECONDS_PER_LOCATION).contains(s));
        let seconds = given.unwrap_or(DEFAULT_SECONDS_PER_LOCATION);
        let locations = data.locations.get(slot).map_or(0, |l| l.len());
        players.push(PlayerEstimate {
            slot: *slot,
            name: info.name.clone(),
            game: info.game.clone(),
            locations,
            seconds_per_location: seconds,
            from_profile: given.is_some(),
            play_seconds: locations as u64 * u64::from(seconds),
            finish_seconds: 0,
        });
    }

    // Lieux de chaque joueur dans chaque sphère (seulement des lieux connus du joueur).
    let spheres: Vec<BTreeMap<u32, u64>> = data
        .spheres
        .iter()
        .map(|sphere| {
            sphere
                .iter()
                .map(|(slot, ids)| {
                    let known = data.locations.get(slot);
                    let n = ids
                        .iter()
                        .filter(|id| known.is_some_and(|k| k.contains_key(id)))
                        .count();
                    (*slot, n as u64)
                })
                .collect()
        })
        .collect();

    // Durée de chaque sphère : celle du joueur le plus long dans cette sphère.
    let work = |p: &PlayerEstimate, sphere: &BTreeMap<u32, u64>| {
        sphere.get(&p.slot).copied().unwrap_or(0) * u64::from(p.seconds_per_location)
    };
    let durations: Vec<u64> = spheres
        .iter()
        .map(|s| players.iter().map(|p| work(p, s)).max().unwrap_or(0))
        .collect();

    for p in &mut players {
        // Début de la dernière sphère où le joueur a des lieux, plus son travail dans
        // celle-ci ; puis les lieux qu'aucune sphère ne cite (seed sans sphères, ou
        // lieux inatteignables en accessibilité minimale), faits à la fin.
        let mut start = 0u64;
        let mut finish = 0u64;
        let mut in_spheres = 0u64;
        for (sphere, duration) in spheres.iter().zip(&durations) {
            let n = sphere.get(&p.slot).copied().unwrap_or(0);
            if n > 0 {
                finish = start + n * u64::from(p.seconds_per_location);
                in_spheres += n;
            }
            start += duration;
        }
        let rest = (p.locations as u64).saturating_sub(in_spheres);
        p.finish_seconds = finish + rest * u64::from(p.seconds_per_location);
    }

    SeedEstimate {
        players,
        spheres: data.spheres.len(),
    }
}
