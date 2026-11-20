//! Générateur Rust (ADR 0027, étape 4) : options des joueurs → mondes Rust → placement
//! (`fill`) → [`MultiData`] → octets d'un fichier `.archipelago`.
//!
//! La seed produite est un vrai `.archipelago` : elle s'héberge sur Raftup comme sur le
//! serveur officiel. Elle est **reproductible dans Raftup** : même numéro de seed, mêmes
//! options et même version de Raftup ⇒ mêmes octets. Elle n'est pas identique à ce que
//! donnerait le générateur Python pour le même numéro (ADR 0027, § 4).

pub mod convert;
mod datapackage;
pub mod imported;
mod multidata_build;
pub mod spoiler;
pub mod worlds;
pub mod yaml;

use std::collections::BTreeSet;

use logic::{ItemKindId, LogicError, MultiWorld, State};
use multidata::{MultiData, WriteError};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

// Réglages et rapport de l'équilibrage par temps (ADR 0030), pour les appelants qui ne
// dépendent pas de `fill`.
pub use datapackage::{archipelago_package, game_package, PackageSource};
pub use fill::{TimeBalance, TimeBalanceReport};
pub use spoiler::Spoiler;
pub use worlds::{GameOptions, NativeOptions, SUPPORTED_GAMES};
pub use worlds_native::NativeWorld;
pub use yaml::{read_players, read_players_with_meta, read_players_with_worlds, Unsupported};

/// Longueur maximale d'un nom de joueur, comme Archipelago.
pub const MAX_NAME_LEN: usize = 16;

/// Un joueur : son nom et son jeu avec ses options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerRequest {
    pub name: String,
    pub options: GameOptions,
    /// `progression_balancing` : 0 (aucun) à 99 ; 50 par défaut, comme Archipelago.
    pub progression_balancing: u8,
}

impl PlayerRequest {
    /// Un joueur avec l'équilibrage par défaut.
    pub fn new(name: impl Into<String>, options: GameOptions) -> PlayerRequest {
        PlayerRequest {
            name: name.into(),
            options,
            progression_balancing: fill::DEFAULT_BALANCING,
        }
    }
}

/// Ce qu'il faut pour générer une partie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateRequest {
    /// Numéro de seed : tout le hasard en découle.
    pub seed: u64,
    /// Joueurs dans l'ordre des slots (le premier reçoit le slot 1).
    pub players: Vec<PlayerRequest>,
    /// Équilibrage par temps de jeu (ADR 0030), après l'équilibrage de progression.
    /// Désactivé par défaut (`TimeBalance::default()`) : la seed est alors la même
    /// qu'avant l'arrivée de cette option.
    pub time_balance: fill::TimeBalance,
}

impl GenerateRequest {
    /// Une demande sans équilibrage par temps.
    pub fn new(seed: u64, players: Vec<PlayerRequest>) -> GenerateRequest {
        GenerateRequest {
            seed,
            players,
            time_balance: fill::TimeBalance::default(),
        }
    }
}

/// Une partie générée.
#[derive(Debug, Clone, PartialEq)]
pub struct Generated {
    /// Nom de la seed : 20 chiffres, comme Archipelago.
    pub seed_name: String,
    pub multidata: MultiData,
    /// Contenu du fichier `AP_<seed_name>.archipelago`.
    pub archipelago: Vec<u8>,
    /// Spoiler de la partie (`AP_<seed_name>_Spoiler.txt` une fois mis en texte).
    pub spoiler: Spoiler,
    /// Ce qu'a fait l'équilibrage par temps, s'il était activé.
    pub time_balance: Option<fill::TimeBalanceReport>,
}

/// Erreurs de génération.
#[derive(Debug, Error)]
pub enum GenerateError {
    #[error("aucun joueur")]
    NoPlayers,
    #[error("nom de joueur « {name} » refusé : {reason}")]
    InvalidName { name: String, reason: &'static str },
    #[error("deux joueurs s'appellent « {0} »")]
    DuplicateName(String),
    #[error("option « {option} » de {player} : {message}")]
    InvalidOption {
        player: String,
        option: String,
        message: String,
    },
    #[error("monde mal construit : {0}")]
    Logic(#[from] LogicError),
    #[error("monde natif : {0}")]
    Native(#[from] worlds_native::BuildError),
    #[error("placement impossible : {0}")]
    Fill(#[from] fill::FillError),
    #[error("écriture de la seed : {0}")]
    Write(#[from] WriteError),
}

/// Nom de seed à 20 chiffres tiré du numéro de seed.
///
/// Un `u64` a au plus 20 chiffres : on complète par des zéros à gauche.
pub fn seed_name(seed: u64) -> String {
    format!("{seed:020}")
}

/// Un flux de hasard propre à un usage (`label`) et à un slot, dérivé de la seed.
///
/// Chaque monde a son propre flux : ajouter un joueur ne change pas les tirages des autres
/// mondes. SHA-256 mélange (étiquette, seed, slot) en 32 octets, la graine de ChaCha20.
pub fn stream(label: &str, seed: u64, slot: u32) -> ChaCha20Rng {
    let mut hasher = Sha256::new();
    hasher.update(b"raftup/generate/");
    hasher.update(label.as_bytes());
    hasher.update([0u8]);
    hasher.update(seed.to_le_bytes());
    hasher.update(slot.to_le_bytes());
    let digest = hasher.finalize();
    let mut key = [0u8; 32];
    // Le résultat de SHA-256 fait exactement 32 octets.
    key.copy_from_slice(&digest);
    ChaCha20Rng::from_seed(key)
}

/// Mondes construits, objets pas encore placés.
#[derive(Debug, Clone)]
pub struct Built {
    pub world: MultiWorld,
    /// Objets à placer (un élément par exemplaire).
    pub pool: Vec<ItemKindId>,
    /// Inventaire de départ, déjà ramassé.
    pub start: State,
    /// Objets de départ de chaque joueur, dans l'ordre des slots.
    pub start_items: Vec<Vec<ItemKindId>>,
    /// `slot_data` de chaque joueur.
    pub slot_data: Vec<Value>,
}

/// Construit les mondes de la demande, sans rien placer (sert aussi à comparer un monde
/// porté en Rust avec le même monde exporté depuis Python, ADR 0028).
pub fn build(request: &GenerateRequest) -> Result<Built, GenerateError> {
    check_players(request)?;

    let mut world = MultiWorld::new();
    let mut pool = Vec::new();
    let mut start_items = Vec::new();
    let mut slot_data = Vec::new();
    for player in &request.players {
        let slot = world.add_slot(player.name.clone(), player.options.game())?;
        // Les slots Archipelago commencent à 1.
        let number = slot.raw() + 1;
        let mut rng = stream("world", request.seed, number);
        let built = player.options.create(&mut world, slot, &mut rng)?;
        pool.extend(built.pool);
        start_items.push(built.start);
        slot_data.push(built.slot_data);
    }

    let mut start = State::new(&world);
    for item in start_items.iter().flatten() {
        start.collect(&world, *item);
    }
    Ok(Built {
        world,
        pool,
        start,
        start_items,
        slot_data,
    })
}

/// Génère une partie.
pub fn generate(request: &GenerateRequest) -> Result<Generated, GenerateError> {
    let Built {
        mut world,
        pool,
        start,
        start_items,
        slot_data,
    } = build(request)?;
    let mut rng = stream("fill", request.seed, 0);
    fill::fill(&mut world, pool, &start, &mut rng)?;
    // Équilibrage de progression (ADR 0027, étape 5), avec la suite du même flux.
    let balancing: Vec<u8> = request
        .players
        .iter()
        .map(|p| p.progression_balancing)
        .collect();
    fill::balance(&mut world, &balancing, &start, &mut rng)?;
    // Équilibrage par temps (ADR 0030) : sans hasard, donc la suite du flux `fill` n'est
    // pas touchée. Appelé seulement s'il est activé : désactivé, il simulerait la partie
    // pour rien.
    let time_balance = run_time_balance(&mut world, &request.time_balance, &start)?;

    let seed_name = seed_name(request.seed);
    let spoiler = spoiler::build(&world, request, &seed_name, &start, &start_items);
    let multidata =
        multidata_build::build(&world, request, &seed_name, &start, &start_items, slot_data)?;
    let archipelago = multidata::write_archipelago(&multidata)?;
    Ok(Generated {
        seed_name,
        multidata,
        archipelago,
        spoiler,
        time_balance,
    })
}

/// Lance [`fill::time_balance`] s'il est activé ; `None` sinon.
pub(crate) fn run_time_balance(
    world: &mut MultiWorld,
    settings: &fill::TimeBalance,
    start: &State,
) -> Result<Option<fill::TimeBalanceReport>, fill::FillError> {
    if !settings.enabled {
        return Ok(None);
    }
    fill::time_balance(world, settings, start).map(Some)
}

fn check_players(request: &GenerateRequest) -> Result<(), GenerateError> {
    if request.players.is_empty() {
        return Err(GenerateError::NoPlayers);
    }
    let mut seen = BTreeSet::new();
    for player in &request.players {
        let name = &player.name;
        let reason = if name.trim().is_empty() {
            Some("vide")
        } else if name.chars().count() > MAX_NAME_LEN {
            Some("plus de 16 caractères")
        } else if name.trim() != name {
            Some("espaces au début ou à la fin")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(GenerateError::InvalidName {
                name: name.clone(),
                reason,
            });
        }
        // Archipelago compare les noms sans tenir compte de la casse.
        if !seen.insert(name.to_lowercase()) {
            return Err(GenerateError::DuplicateName(name.clone()));
        }
        if let Err((option, message)) = player.options.check() {
            return Err(GenerateError::InvalidOption {
                player: name.clone(),
                option,
                message,
            });
        }
    }
    Ok(())
}
