//! Logique d'un multiworld pour le générateur Rust (ADR 0027, étape 1).
//!
//! Ce que contient la crate :
//!
//! - le **modèle** ([`MultiWorld`]) : joueurs, régions, entrées, lieux, sortes d'objets
//!   et groupes d'objets, rangés dans des tableaux (arènes) et désignés par des index `u32`
//!   ([`RegionId`], [`LocationId`]…), jamais par des références ;
//! - les **règles** ([`Rule`]) : un arbre fermé qui reprend le vocabulaire du module
//!   `rule_builder` d'Archipelago (`Has`, `HasAll`, `HasAny`, `CanReach`, `And`, `Or`…) ;
//! - l'**inventaire** ([`State`]) : un compteur par sorte d'objet ;
//! - le **calcul d'atteignabilité** ([`Reachability`]) et le **balayage par sphères**
//!   ([`sweep`]) : sphère 0 = ce qui est atteignable au départ, sphère 1 = ce que les objets
//!   de la sphère 0 ouvrent, etc.
//!
//! Aucun hasard ici : tout est parcouru dans l'ordre de création (index croissant), donc le
//! même monde donne toujours les mêmes sphères. Le placement des objets est le travail de la
//! future crate `fill`.

mod ids;
mod item_rule;
mod rule;
mod state;
mod sweep;
mod world;

pub use ids::{EntranceId, ItemGroupId, ItemKindId, ItemRuleId, LocationId, RegionId, SlotId};
pub use item_rule::{Accessibility, ItemRule};
pub use rule::Rule;
pub use state::State;
pub use sweep::{sweep, Reachability, SweepResult};
pub use world::{
    Classification, Entrance, ItemGroup, ItemKind, Location, LogicError, MultiWorld, Progress,
    Region, Slot,
};
