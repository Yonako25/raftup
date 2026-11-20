//! Mondes du moteur Rust : portés en Rust (ADR 0027, § 1), un module par monde, ou
//! natifs (ADR 0041), lus depuis un fichier TOML par la crate `worlds-native`.

pub mod apquest;
pub mod native;

use logic::{MultiWorld, SlotId};
use multidata::GamePackage;
use rand_core::Rng;

use crate::datapackage::game_package;
use crate::GenerateError;
pub(crate) use apquest::BuiltPlayer;
pub use native::NativeOptions;

/// Le jeu d'un joueur et ses options. Un nouveau monde ajoute une variante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameOptions {
    ApQuest(apquest::ApQuestOptions),
    /// Un monde natif (fichier TOML) et ses options.
    Native(NativeOptions),
}

impl GameOptions {
    /// Nom du jeu (celui du YAML et du datapackage).
    pub fn game(&self) -> &str {
        match self {
            GameOptions::ApQuest(_) => apquest::GAME,
            GameOptions::Native(o) => &o.world.game,
        }
    }

    /// Datapackage du jeu (le même quelles que soient les options).
    pub fn package(&self) -> GamePackage {
        match self {
            GameOptions::ApQuest(_) => game_package(&apquest::PACKAGE),
            GameOptions::Native(o) => o.package(),
        }
    }

    /// Vérifie les options ; renvoie le nom de l'option fautive et le problème.
    pub fn check(&self) -> Result<(), (String, String)> {
        match self {
            GameOptions::ApQuest(o) => o.check().map_err(|(name, m)| (name.to_string(), m)),
            GameOptions::Native(o) => o.world.check_values(&o.values).map_err(|e| match e {
                worlds_native::BuildError::BadOption { option, message } => (option, message),
                other => (String::new(), other.to_string()),
            }),
        }
    }

    /// Options lisibles pour le spoiler : (nom affiché, valeur), dans l'ordre du monde.
    pub fn spoiler_options(&self) -> Vec<(String, String)> {
        match self {
            GameOptions::ApQuest(o) => o
                .spoiler_options()
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
            GameOptions::Native(o) => o.world.spoiler_options(&o.values),
        }
    }

    pub(crate) fn create<R: Rng + ?Sized>(
        &self,
        world: &mut MultiWorld,
        slot: SlotId,
        rng: &mut R,
    ) -> Result<BuiltPlayer, GenerateError> {
        match self {
            GameOptions::ApQuest(o) => Ok(apquest::create(world, slot, o, rng)?),
            GameOptions::Native(o) => o.create(world, slot, rng),
        }
    }
}

/// Jeux que le moteur Rust sait générer.
pub const SUPPORTED_GAMES: &[&str] = &[apquest::GAME];
