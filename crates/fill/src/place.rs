//! Placement : objets à placer tôt, puis progression par placement supposé, puis utiles,
//! puis remplissage.
//!
//! Chaque objet ne va que dans un lieu qui l'accepte ([`MultiWorld::can_hold`] : règle
//! d'objet du lieu, lieux exclus). Les objets à placer tôt (`early_items`, ADR 0029) vont
//! d'abord dans les lieux de la sphère 0.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;

use logic::{
    sweep, Accessibility, ItemKindId, LocationId, LogicError, MultiWorld, Progress, Reachability,
    State,
};
use rand_core::Rng;
use thiserror::Error;

use crate::explain::{explain, Blocked};
use crate::shuffle::shuffle;

/// Erreurs du placement. En cas d'erreur, le monde est remis comme avant l'appel.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FillError {
    #[error(transparent)]
    Logic(#[from] LogicError),
    #[error("{items} objets à placer pour {locations} lieux vides : il en faut autant")]
    CountMismatch { items: usize, locations: usize },
    #[error("{excluded} lieux exclus pour {filler} objets de remplissage : il en manque")]
    TooManyExcluded { excluded: usize, filler: usize },
    #[error("options impossibles, même avec tous les objets : {}", list(.blocked))]
    Impossible { blocked: Vec<Blocked> },
    #[error("aucun lieu libre atteignable pour « {item} » (joueur {slot})")]
    NoReachableLocation { slot: String, item: String },
    #[error("aucun lieu libre n'accepte « {item} » (joueur {slot})")]
    NoAllowedLocation { slot: String, item: String },
    #[error("aucun objet de remplissage accepté par le lieu exclu « {location} » (joueur {slot})")]
    NoAllowedItem { slot: String, location: String },
    #[error("placement injouable : {}", list(.blocked))]
    Unbeatable { blocked: Vec<Blocked> },
}

/// Les trois premiers blocages, puis le nombre des autres.
fn list(blocked: &[Blocked]) -> String {
    let mut text = String::new();
    for (i, b) in blocked.iter().take(3).enumerate() {
        if i > 0 {
            text.push_str(" ; ");
        }
        // Écrire dans une `String` ne peut pas échouer : on ignore le `Result`.
        let _ = write!(text, "{b}");
    }
    if blocked.len() > 3 {
        let _ = write!(text, " ; et {} autres", blocked.len() - 3);
    }
    text
}

/// Résultat d'un placement réussi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillReport {
    /// Sphères du balayage final, depuis l'inventaire de départ (pour le spoiler).
    pub spheres: Vec<Vec<LocationId>>,
    /// Objets à placer tôt qui n'ont pas trouvé de place en sphère 0 : placés plus loin,
    /// comme le fait Archipelago (qui l'écrit dans son journal).
    pub early_not_placed: u32,
}

/// Objets d'un joueur à placer tôt, dans un lieu atteignable dès le départ (sphère 0).
///
/// C'est `early_items` d'Archipelago ; avec `local` (`local_early_items`), le lieu doit en
/// plus appartenir au propriétaire de l'objet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EarlyItem {
    pub item: ItemKindId,
    /// Nombre d'exemplaires du pool à placer tôt.
    pub count: u32,
    pub local: bool,
}

/// Vérifie qu'avec l'inventaire de départ **et** tous les objets du pool en main, chaque
/// lieu qui doit l'être (voir [`MultiWorld::must_reach`]) et chaque objectif sont
/// atteignables.
pub fn check_feasible(
    world: &MultiWorld,
    pool: &[ItemKindId],
    start: &State,
) -> Result<(), FillError> {
    let mut state = start.clone();
    for item in pool {
        state.collect(world, *item);
    }
    let result = sweep(world, state);
    let blocked = explain(world, &result);
    if blocked.is_empty() {
        Ok(())
    } else {
        Err(FillError::Impossible { blocked })
    }
}

/// Place les objets du `pool` (un élément par exemplaire) dans les lieux vides du monde.
///
/// - `start` : inventaire de départ (objets reçus dès le début de la partie) ;
/// - `rng` : seule source de hasard ; même suite de nombres ⇒ même placement.
///
/// Il faut exactement autant d'objets que de lieux vides. Les lieux déjà remplis (les
/// événements, par exemple) ne bougent pas et leurs objets comptent dans la logique.
///
/// Coût : un balayage complet par objet de progression ; suffisant pour quelques milliers
/// de lieux, à revoir pour de très gros multiworlds.
pub fn fill<R: Rng + ?Sized>(
    world: &mut MultiWorld,
    pool: Vec<ItemKindId>,
    start: &State,
    rng: &mut R,
) -> Result<FillReport, FillError> {
    fill_with_early(world, pool, start, &[], rng)
}

/// Comme [`fill`], avec des objets à placer tôt (`early_items`, voir [`EarlyItem`]).
pub fn fill_with_early<R: Rng + ?Sized>(
    world: &mut MultiWorld,
    pool: Vec<ItemKindId>,
    start: &State,
    early: &[EarlyItem],
    rng: &mut R,
) -> Result<FillReport, FillError> {
    world.validate()?;
    for item in pool.iter().chain(early.iter().map(|e| &e.item)) {
        if world.item_kind(*item).is_none() {
            return Err(LogicError::UnknownItem(item.raw()).into());
        }
    }
    let empty: Vec<LocationId> = world
        .location_ids()
        .filter(|id| world.location(*id).is_some_and(|l| l.item.is_none()))
        .collect();
    if empty.len() != pool.len() {
        return Err(FillError::CountMismatch {
            items: pool.len(),
            locations: empty.len(),
        });
    }
    // Un lieu exclu ne reçoit que du remplissage (ni progression ni utile).
    let excluded = empty
        .iter()
        .filter(|id| progress(world, **id) == Progress::Excluded)
        .count();
    let filler = pool
        .iter()
        .filter(|item| {
            world
                .item_kind(**item)
                .is_some_and(|k| !k.classification.progression && !k.classification.useful)
        })
        .count();
    if excluded > filler {
        return Err(FillError::TooManyExcluded { excluded, filler });
    }
    check_feasible(world, &pool, start)?;

    // Le placement supposé peut se bloquer selon l'ordre tiré : si la clé est posée avant
    // l'épée dans le seul lieu ouvert sans épée, l'épée n'a plus de place. On recommence
    // alors avec la suite du même flux de hasard (le résultat reste reproductible).
    let mut attempt = 1;
    loop {
        // On note chaque lieu rempli pour tout défaire en cas d'échec.
        let mut placed = Vec::with_capacity(empty.len());
        let result = place_all(
            world,
            pool.clone(),
            empty.clone(),
            start,
            early,
            rng,
            &mut placed,
        );
        if result.is_err() {
            for location in placed {
                // Le lieu existe (on vient d'y placer un objet) : l'erreur est impossible.
                let _ = world.take(location);
            }
        }
        match result {
            Err(
                FillError::NoReachableLocation { .. }
                | FillError::NoAllowedLocation { .. }
                | FillError::NoAllowedItem { .. },
            ) if attempt < MAX_ATTEMPTS => {
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// Type du lieu (normal si le lieu n'existe pas).
pub(crate) fn progress(world: &MultiWorld, location: LocationId) -> Progress {
    world
        .location(location)
        .map(|l| l.progress)
        .unwrap_or_default()
}

/// Nombre d'essais de placement avant d'abandonner.
pub const MAX_ATTEMPTS: u32 = 20;

/// Accessibilité du joueur à qui appartient l'objet (`Full` si inconnu).
fn owner_accessibility(world: &MultiWorld, item: ItemKindId) -> Accessibility {
    world
        .item_kind(item)
        .and_then(|k| world.slot(k.slot))
        .map(|s| s.accessibility)
        .unwrap_or_default()
}

/// Accessibilité du joueur à qui appartient le lieu (`Full` si inconnu).
fn location_accessibility(world: &MultiWorld, location: LocationId) -> Accessibility {
    world
        .location(location)
        .and_then(|l| world.slot(l.slot))
        .map(|s| s.accessibility)
        .unwrap_or_default()
}

/// Nom du joueur et nom de l'objet, pour les erreurs.
fn item_names(world: &MultiWorld, item: ItemKindId) -> (String, String) {
    let kind = world.item_kind(item);
    (
        kind.and_then(|k| world.slot(k.slot))
            .map(|s| s.name.clone())
            .unwrap_or_default(),
        kind.map(|k| k.name.clone()).unwrap_or_default(),
    )
}

/// Retire et renvoie le premier élément qui vérifie `ok` (l'ordre des autres est gardé).
fn take_first<T: Copy>(list: &mut Vec<T>, ok: impl Fn(T) -> bool) -> Option<T> {
    let index = list.iter().position(|x| ok(*x))?;
    Some(list.remove(index))
}

/// Retire et renvoie le dernier élément qui vérifie `ok`.
fn take_last<T: Copy>(list: &mut Vec<T>, ok: impl Fn(T) -> bool) -> Option<T> {
    let index = list.iter().rposition(|x| ok(*x))?;
    Some(list.remove(index))
}

fn place_all<R: Rng + ?Sized>(
    world: &mut MultiWorld,
    pool: Vec<ItemKindId>,
    empty: Vec<LocationId>,
    start: &State,
    early: &[EarlyItem],
    rng: &mut R,
    placed: &mut Vec<LocationId>,
) -> Result<FillReport, FillError> {
    // Trois paquets, dans l'ordre du pool : progression, utiles, le reste.
    let mut progression = Vec::new();
    let mut useful = Vec::new();
    let mut rest = Vec::new();
    for item in pool {
        let class = world
            .item_kind(item)
            .map(|k| k.classification)
            .unwrap_or_default();
        if class.progression {
            progression.push(item);
        } else if class.useful {
            useful.push(item);
        } else {
            rest.push(item);
        }
    }
    let mut empty = empty;
    shuffle(rng, &mut empty);
    shuffle(rng, &mut progression);
    shuffle(rng, &mut useful);
    shuffle(rng, &mut rest);

    let mut packs = Packs {
        progression,
        useful,
        rest,
    };
    let early_not_placed = if early.is_empty() {
        0
    } else {
        place_early(world, &mut empty, &mut packs, start, early, placed)?
    };
    let Packs {
        mut progression,
        useful,
        mut rest,
    } = packs;

    // Trois listes de lieux, dans l'ordre mélangé : prioritaires, normaux, exclus.
    let mut priority = Vec::new();
    let mut normal = Vec::new();
    let mut excluded = Vec::new();
    for location in empty {
        match progress(world, location) {
            Progress::Priority => priority.push(location),
            Progress::Default => normal.push(location),
            Progress::Excluded => excluded.push(location),
        }
    }

    // Progression : placement supposé. On prend le dernier objet du paquet ; on suppose
    // qu'on possède tous ceux qui restent ; le balayage ramasse en plus les objets déjà
    // posés dans les lieux atteints. L'objet va dans le premier lieu prioritaire libre,
    // atteignable et qui l'accepte, sinon dans le premier lieu normal ; jamais dans un
    // lieu exclu (sauf `always_allow`).
    // `pop` retire le dernier élément : le paquet rétrécit à chaque tour.
    while let Some(item) = progression.pop() {
        let mut state = start.clone();
        for other in &progression {
            state.collect(world, *other);
        }
        let result = sweep(world, state);
        // `accessibility: minimal` (comme `fill_restrictive` d'Archipelago) : si toutes les
        // victoires sont déjà atteintes sans cet objet, il peut aller n'importe où, même
        // hors d'atteinte, sauf chez un joueur qui veut atteindre tous ses lieux.
        let anywhere = owner_accessibility(world, item) == Accessibility::Minimal
            && result.all_goals_reached(world);
        let reach = &result.reach;
        // Les listes sont empruntées pendant la recherche ; le monde seulement lu.
        let world_ref: &MultiWorld = world;
        let fits = |l: LocationId| {
            let open = reach.location(l)
                || (anywhere && location_accessibility(world_ref, l) != Accessibility::Full);
            open && world_ref.can_hold(l, item)
        };
        let found = take_first(&mut priority, fits).or_else(|| take_first(&mut normal, fits));
        let Some(location) = found else {
            let (slot, item) = item_names(world, item);
            return Err(FillError::NoReachableLocation { slot, item });
        };
        world.place(location, item)?;
        placed.push(location);
    }

    // Avec un joueur qui n'est pas en `Full`, des lieux peuvent rester fermés après la
    // progression : comme Archipelago (`inaccessible_location_rules`), ils ne reçoivent pas
    // d'objet important (progression ou utile) d'un joueur qui n'est pas en `Minimal`.
    let closed: Vec<bool> = if world
        .slots()
        .iter()
        .any(|s| s.accessibility != Accessibility::Full)
    {
        let reach = sweep(world, start.clone()).reach;
        world.location_ids().map(|l| !reach.location(l)).collect()
    } else {
        Vec::new()
    };
    let allowed = |world: &MultiWorld, l: LocationId, item: ItemKindId| {
        if !world.can_hold(l, item) {
            return false;
        }
        let is_closed = closed.get(l.index()).copied().unwrap_or(false);
        let important = world
            .item_kind(item)
            .is_some_and(|k| k.classification.progression || k.classification.useful);
        !(is_closed && important && owner_accessibility(world, item) != Accessibility::Minimal)
    };

    // Lieux exclus : du remplissage d'abord (`fill` a vérifié qu'il y en a assez). On
    // réserve la fin du paquet ; chaque lieu prend le premier objet réservé qu'il accepte,
    // sinon un autre objet du paquet.
    // `split_off` coupe la liste : `rest` garde le début, `reserved` prend la fin.
    let mut reserved = rest.split_off(rest.len().saturating_sub(excluded.len()));
    for location in excluded {
        let world_ref: &MultiWorld = world;
        let ok = |item: ItemKindId| allowed(world_ref, location, item);
        let Some(item) = take_first(&mut reserved, ok).or_else(|| take_last(&mut rest, ok)) else {
            let l = world.location(location);
            return Err(FillError::NoAllowedItem {
                slot: l
                    .and_then(|l| world.slot(l.slot))
                    .map(|s| s.name.clone())
                    .unwrap_or_default(),
                location: l.map(|l| l.name.clone()).unwrap_or_default(),
            });
        };
        world.place(location, item)?;
        placed.push(location);
    }
    rest.extend(reserved);

    // Utiles puis le reste du remplissage : chaque objet prend le premier lieu libre
    // restant qui l'accepte. Sans règle d'objet, c'est simplement le lieu suivant.
    // `VecDeque` : retirer en tête est immédiat (un `Vec` décalerait tout).
    let mut remaining: VecDeque<LocationId> = priority.into_iter().chain(normal).collect();
    for item in useful.into_iter().chain(rest) {
        let world_ref: &MultiWorld = world;
        let position = remaining.iter().position(|l| allowed(world_ref, *l, item));
        let Some(location) = position.and_then(|p| remaining.remove(p)) else {
            let (slot, item) = item_names(world, item);
            return Err(FillError::NoAllowedLocation { slot, item });
        };
        world.place(location, item)?;
        placed.push(location);
    }

    // Vérification finale depuis l'inventaire de départ seul.
    let result = sweep(world, start.clone());
    let blocked = explain(world, &result);
    if !blocked.is_empty() {
        return Err(FillError::Unbeatable { blocked });
    }
    Ok(FillReport {
        spheres: result.spheres,
        early_not_placed,
    })
}

/// Les trois paquets d'objets à placer, déjà mélangés.
struct Packs {
    progression: Vec<ItemKindId>,
    useful: Vec<ItemKindId>,
    rest: Vec<ItemKindId>,
}

/// Inventaire de départ plus les objets des **événements** (lieux sans numéro) atteints,
/// ramassés tant que c'est possible : l'état de base de `distribute_early_items`.
fn events_state(world: &MultiWorld, start: &State) -> State {
    let mut state = start.clone();
    let mut taken = vec![false; world.locations().len()];
    loop {
        let reach = Reachability::compute(world, &state);
        let mut changed = false;
        for id in world.location_ids() {
            let Some(location) = world.location(id) else {
                continue;
            };
            let fresh = taken.get(id.index()).is_some_and(|t| !*t);
            if fresh && location.address.is_none() && reach.location(id) {
                if let Some(slot) = taken.get_mut(id.index()) {
                    *slot = true;
                }
                if let Some(item) = location.item {
                    changed |= state.collect(world, item);
                }
            }
        }
        if !changed {
            return state;
        }
    }
}

/// Place les objets à placer tôt dans les lieux vides atteignables dès le départ, d'après
/// `distribute_early_items` d'Archipelago 0.6.8 (`Fill.py`, MIT) :
///
/// 1. les exemplaires concernés sont pris dans les paquets (dans leur ordre mélangé) ;
/// 2. les objets qui ne sont pas de progression d'abord, dans les lieux de la sphère 0
///    qui ne sont pas prioritaires : ceux à garder chez leur joueur (`local`) puis les
///    autres ;
/// 3. puis la progression, dans tous les lieux de la sphère 0 restants (prioritaires
///    compris), les locaux d'abord ;
/// 4. ce qui n'a pas trouvé de place retourne dans son paquet et sera placé normalement.
///
/// Tous ces lieux sont atteignables avec l'inventaire de départ seul : poser un objet de
/// progression n'y ferme rien. Renvoie le nombre d'objets non placés tôt.
fn place_early(
    world: &mut MultiWorld,
    empty: &mut Vec<LocationId>,
    packs: &mut Packs,
    start: &State,
    early: &[EarlyItem],
    placed: &mut Vec<LocationId>,
) -> Result<u32, FillError> {
    // (exemplaires à placer tôt n'importe où, exemplaires à garder chez le joueur).
    let mut wanted: BTreeMap<ItemKindId, (u32, u32)> = BTreeMap::new();
    for e in early {
        let entry = wanted.entry(e.item).or_insert((0, 0));
        if e.local {
            entry.1 = entry.1.saturating_add(e.count);
        } else {
            entry.0 = entry.0.saturating_add(e.count);
        }
    }

    // Les exemplaires, pris dans les paquets ; `(objet, local)`.
    let mut early_prog: Vec<(ItemKindId, bool)> = Vec::new();
    let mut early_rest: Vec<(ItemKindId, bool)> = Vec::new();
    for (pack, is_prog) in [
        (&mut packs.progression, true),
        (&mut packs.useful, false),
        (&mut packs.rest, false),
    ] {
        // `retain` garde les objets pour lesquels la fonction renvoie `true`.
        pack.retain(|item| {
            let Some(entry) = wanted.get_mut(item) else {
                return true;
            };
            let local = if entry.1 > 0 {
                entry.1 -= 1;
                true
            } else if entry.0 > 0 {
                entry.0 -= 1;
                false
            } else {
                return true;
            };
            let target = if is_prog {
                &mut early_prog
            } else {
                &mut early_rest
            };
            target.push((*item, local));
            false
        });
    }

    let state = events_state(world, start);
    let reach = Reachability::compute(world, &state);
    let mut spots: Vec<LocationId> = empty
        .iter()
        .copied()
        .filter(|l| reach.location(*l))
        .collect();

    let mut unplaced: Vec<ItemKindId> = Vec::new();
    for (items, with_priority) in [(early_rest, false), (early_prog, true)] {
        // Les locaux d'abord, puis les autres (un local qui n'a pas de place chez son
        // joueur repasse avec les autres, comme dans Archipelago).
        let (locals, globals): (Vec<_>, Vec<_>) = items.into_iter().partition(|(_, l)| *l);
        let mut retry = Vec::new();
        for (item, _) in locals {
            let owner = world.item_kind(item).map(|k| k.slot);
            let world_ref: &MultiWorld = world;
            let ok = |l: LocationId| {
                (with_priority || progress(world_ref, l) != Progress::Priority)
                    && world_ref.location(l).map(|x| x.slot) == owner
                    && world_ref.can_hold(l, item)
            };
            match take_first(&mut spots, ok) {
                Some(location) => {
                    world.place(location, item)?;
                    placed.push(location);
                }
                None => retry.push(item),
            }
        }
        for item in retry.into_iter().chain(globals.into_iter().map(|(i, _)| i)) {
            let world_ref: &MultiWorld = world;
            let ok = |l: LocationId| {
                (with_priority || progress(world_ref, l) != Progress::Priority)
                    && world_ref.can_hold(l, item)
            };
            match take_first(&mut spots, ok) {
                Some(location) => {
                    world.place(location, item)?;
                    placed.push(location);
                }
                None => unplaced.push(item),
            }
        }
    }

    // Les lieux remplis sortent de la liste des lieux vides.
    empty.retain(|l| world.location(*l).is_some_and(|x| x.item.is_none()));
    // Ce qui n'a pas trouvé de place retourne dans son paquet.
    let count = u32::try_from(unplaced.len()).unwrap_or(u32::MAX);
    for item in unplaced {
        let class = world
            .item_kind(item)
            .map(|k| k.classification)
            .unwrap_or_default();
        if class.progression {
            packs.progression.push(item);
        } else if class.useful {
            packs.useful.push(item);
        } else {
            packs.rest.push(item);
        }
    }
    Ok(count)
}
