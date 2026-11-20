//! Mondes natifs (ADR 0041) : un fichier TOML lu par la crate `worlds-native`, compilé
//! vers `logic` comme un monde porté en Rust.

use std::sync::Arc;

use logic::{MultiWorld, SlotId};
use multidata::GamePackage;
use rand_core::Rng;
use worlds_native::{NativeWorld, OptionValues};

use crate::datapackage::{game_package, PackageSource};
use crate::worlds::BuiltPlayer;
use crate::GenerateError;

/// Un monde natif et les options d'un joueur.
///
/// `Arc` : le monde est partagé par tous les joueurs qui y jouent, sans le recopier ;
/// cloner un `Arc` ne copie qu'un pointeur (et compte une référence de plus).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeOptions {
    pub world: Arc<NativeWorld>,
    pub values: OptionValues,
}

impl NativeOptions {
    /// Le monde avec ses options par défaut.
    pub fn new(world: Arc<NativeWorld>) -> NativeOptions {
        let values = world.default_values();
        NativeOptions { world, values }
    }

    /// Change une option (nom du YAML, valeur entière : 0/1, nombre ou n° du choix).
    pub fn set(&mut self, name: &str, value: i64) -> Result<(), worlds_native::BuildError> {
        self.world.set_option(&mut self.values, name, value)
    }

    /// Datapackage du jeu (le même quelles que soient les options).
    pub fn package(&self) -> GamePackage {
        let items = self.world.package_items();
        let locations = self.world.package_locations();
        let item_groups = self.world.package_item_groups();
        let location_groups = self.world.package_location_groups();
        // `PackageSource` prête des tranches (`&[…]`) : on garde ici les listes qui les
        // portent, le temps de l'appel.
        let item_groups: Vec<(&str, &[&str])> = item_groups
            .iter()
            .map(|(n, m)| (*n, m.as_slice()))
            .collect();
        let location_groups: Vec<(&str, &[&str])> = location_groups
            .iter()
            .map(|(n, m)| (*n, m.as_slice()))
            .collect();
        game_package(&PackageSource {
            items: &items,
            locations: &locations,
            item_groups: &item_groups,
            location_groups: &location_groups,
        })
    }

    pub(crate) fn create<R: Rng + ?Sized>(
        &self,
        world: &mut MultiWorld,
        slot: SlotId,
        rng: &mut R,
    ) -> Result<BuiltPlayer, GenerateError> {
        let built = self.world.build(&self.values, world, slot, rng)?;
        Ok(BuiltPlayer {
            pool: built.pool,
            start: built.start,
            slot_data: built.slot_data,
        })
    }
}
