//! Ce qui est atteignable, et le balayage par sphères.
//!
//! **Atteignabilité** : on part des régions de départ, puis on répète jusqu'à ce que plus
//! rien ne change : une entrée est franchissable si sa région d'origine est atteinte et sa
//! règle vraie ; elle rend sa région d'arrivée atteinte ; un lieu est atteignable si sa
//! région l'est et sa règle vraie. Les règles n'ayant pas de « non », ce qui est atteint le
//! reste : la boucle ne fait qu'ajouter, donc elle s'arrête toujours.
//!
//! **Sphères** : sphère 0 = les lieux atteignables avec l'inventaire de départ ; on ramasse
//! leurs objets ; sphère 1 = les lieux devenus atteignables grâce à eux ; etc. jusqu'à ce
//! qu'aucun nouveau lieu ne s'ouvre.

use crate::ids::{EntranceId, LocationId, RegionId, SlotId};
use crate::state::State;
use crate::world::MultiWorld;

/// Lit un drapeau sans paniquer (faux hors du tableau).
fn flag(flags: &[bool], index: usize) -> bool {
    flags.get(index).copied().unwrap_or(false)
}

/// Lève un drapeau ; renvoie `true` s'il était baissé.
fn raise(flags: &mut [bool], index: usize) -> bool {
    match flags.get_mut(index) {
        Some(value) if !*value => {
            *value = true;
            true
        }
        _ => false,
    }
}

/// Ce qui est atteignable pour un inventaire donné.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reachability {
    regions: Vec<bool>,
    entrances: Vec<bool>,
    locations: Vec<bool>,
}

impl Reachability {
    /// Calcule ce qui est atteignable avec `state`.
    pub fn compute(world: &MultiWorld, state: &State) -> Reachability {
        let mut reach = Reachability::default();
        reach.update(world, state);
        reach
    }

    /// Complète le calcul après que l'inventaire a **grandi**.
    ///
    /// Si des objets ont été retirés de l'inventaire, il faut repartir de zéro avec
    /// [`Reachability::compute`] : ce calcul ne retire jamais rien.
    pub fn update(&mut self, world: &MultiWorld, state: &State) {
        // Le monde a pu grandir depuis le dernier calcul : on ajuste les tailles.
        self.regions.resize(world.regions().len(), false);
        self.entrances.resize(world.entrances().len(), false);
        self.locations.resize(world.locations().len(), false);

        for slot in world.slots() {
            if let Some(origin) = slot.origin {
                raise(&mut self.regions, origin.index());
            }
        }

        // Point fixe : on repasse tant qu'un tour a ajouté quelque chose. Les entrées sont
        // parcourues dans l'ordre de création, donc une chaîne de régions créée dans
        // l'ordre est souvent découverte en un seul tour.
        loop {
            let mut changed = false;
            for (index, entrance) in world.entrances().iter().enumerate() {
                // La règle est évaluée seulement si l'entrée n'est pas encore franchie et
                // que son origine est atteinte (`&&` s'arrête au premier faux).
                if !flag(&self.entrances, index)
                    && flag(&self.regions, entrance.from.index())
                    && entrance.rule.eval(world, state, self)
                {
                    // L'emprunt de `self` par `eval` est terminé : on peut le modifier.
                    raise(&mut self.entrances, index);
                    raise(&mut self.regions, entrance.to.index());
                    changed = true;
                }
            }
            for (index, location) in world.locations().iter().enumerate() {
                if !flag(&self.locations, index)
                    && flag(&self.regions, location.region.index())
                    && location.rule.eval(world, state, self)
                {
                    raise(&mut self.locations, index);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// La région est atteignable.
    pub fn region(&self, id: RegionId) -> bool {
        flag(&self.regions, id.index())
    }

    /// L'entrée est franchissable.
    pub fn entrance(&self, id: EntranceId) -> bool {
        flag(&self.entrances, id.index())
    }

    /// Le lieu est atteignable.
    pub fn location(&self, id: LocationId) -> bool {
        flag(&self.locations, id.index())
    }
}

/// Résultat d'un balayage par sphères.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepResult {
    /// Lieux de chaque sphère, dans l'ordre de création à l'intérieur d'une sphère.
    pub spheres: Vec<Vec<LocationId>>,
    /// Inventaire final : départ + tout ce qui a été ramassé.
    pub state: State,
    /// Ce qui est atteignable à la fin.
    pub reach: Reachability,
}

impl SweepResult {
    /// Lieux jamais atteints, dans l'ordre.
    pub fn unreachable_locations(&self, world: &MultiWorld) -> Vec<LocationId> {
        world
            .location_ids()
            .filter(|id| !self.reach.location(*id))
            .collect()
    }

    /// La condition de victoire du joueur est remplie à la fin du balayage.
    pub fn goal_reached(&self, world: &MultiWorld, slot: SlotId) -> bool {
        world
            .slot(slot)
            .is_some_and(|slot| slot.goal.eval(world, &self.state, &self.reach))
    }

    /// Tous les joueurs atteignent leur victoire.
    pub fn all_goals_reached(&self, world: &MultiWorld) -> bool {
        world.slot_ids().all(|slot| self.goal_reached(world, slot))
    }
}

/// Balaye le monde par sphères à partir de l'inventaire `start`, en ramassant les objets
/// placés dans les lieux atteints.
///
/// `start` est pris par valeur (déplacé) : il devient l'inventaire final du résultat.
pub fn sweep(world: &MultiWorld, start: State) -> SweepResult {
    let mut state = start;
    let mut reach = Reachability::compute(world, &state);
    let mut collected = vec![false; world.locations().len()];
    let mut spheres = Vec::new();

    loop {
        let sphere: Vec<LocationId> = world
            .location_ids()
            .filter(|id| reach.location(*id) && !flag(&collected, id.index()))
            .collect();
        if sphere.is_empty() {
            break;
        }
        for id in &sphere {
            raise(&mut collected, id.index());
            if let Some(item) = world.location(*id).and_then(|location| location.item) {
                state.collect(world, item);
            }
        }
        spheres.push(sphere);
        reach.update(world, &state);
    }

    SweepResult {
        spheres,
        state,
        reach,
    }
}
