//! Mondes écrits en Rust (ADR 0027, § 1). En V3, le format déclaratif `worlds-native`
//! prendra le relais ; en attendant, un monde est un module ici.

pub mod apquest;

use logic::{LogicError, MultiWorld, SlotId};
use multidata::GamePackage;
use rand_core::Rng;

use crate::datapackage::game_package;
pub(crate) use apquest::BuiltPlayer;

/// Le jeu d'un joueur et ses options. Un nouveau monde ajoute une variante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameOptions {
    ApQuest(apquest::ApQuestOptions),
}

impl GameOptions {
    /// Nom du jeu (celui du YAML et du datapackage).
    pub fn game(&self) -> &'static str {
        match self {
            GameOptions::ApQuest(_) => apquest::GAME,
        }
    }

    /// Datapackage du jeu (le même quelles que soient les options).
    pub fn package(&self) -> GamePackage {
        match self {
            GameOptions::ApQuest(_) => game_package(&apquest::PACKAGE),
        }
    }

    /// Vérifie les options ; renvoie le nom de l'option fautive et le problème.
    pub fn check(&self) -> Result<(), (&'static str, String)> {
        match self {
            GameOptions::ApQuest(o) => o.check(),
        }
    }

    /// Options lisibles pour le spoiler : (nom affiché, valeur), dans l'ordre du monde.
    pub fn spoiler_options(&self) -> Vec<(&'static str, String)> {
        match self {
            GameOptions::ApQuest(o) => o.spoiler_options(),
        }
    }

    pub(crate) fn create<R: Rng + ?Sized>(
        &self,
        world: &mut MultiWorld,
        slot: SlotId,
        rng: &mut R,
    ) -> Result<BuiltPlayer, LogicError> {
        match self {
            GameOptions::ApQuest(o) => apquest::create(world, slot, o, rng),
        }
    }
}

/// Jeux que le moteur Rust sait générer.
pub const SUPPORTED_GAMES: &[&str] = &[apquest::GAME];
