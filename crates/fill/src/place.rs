//! Placement : progression par placement supposé, puis utiles, puis remplissage.

use std::fmt::Write as _;

use logic::{sweep, ItemKindId, LocationId, LogicError, MultiWorld, Progress, State};
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
}

/// Vérifie qu'avec l'inventaire de départ **et** tous les objets du pool en main, chaque
/// lieu et chaque objectif sont atteignables.
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
    world.validate()?;
    for item in &pool {
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
        let result = place_all(world, pool.clone(), empty.clone(), start, rng, &mut placed);
        if result.is_err() {
            for location in placed {
                // Le lieu existe (on vient d'y placer un objet) : l'erreur est impossible.
                let _ = world.take(location);
            }
        }
        match result {
            Err(FillError::NoReachableLocation { .. }) if attempt < MAX_ATTEMPTS => {
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

fn place_all<R: Rng + ?Sized>(
    world: &mut MultiWorld,
    pool: Vec<ItemKindId>,
    empty: Vec<LocationId>,
    start: &State,
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
    // posés dans les lieux atteints. L'objet va dans le premier lieu prioritaire libre et
    // atteignable, sinon dans le premier lieu normal ; jamais dans un lieu exclu.
    // `pop` retire le dernier élément : le paquet rétrécit à chaque tour.
    while let Some(item) = progression.pop() {
        let mut state = start.clone();
        for other in &progression {
            state.collect(world, *other);
        }
        let reach = sweep(world, state).reach;
        let found = [&mut priority, &mut normal].into_iter().find_map(|list| {
            list.iter()
                .position(|l| reach.location(*l))
                // `remove` garde l'ordre (mélangé) des lieux restants.
                .map(|index| list.remove(index))
        });
        let Some(location) = found else {
            let kind = world.item_kind(item);
            return Err(FillError::NoReachableLocation {
                slot: kind
                    .and_then(|k| world.slot(k.slot))
                    .map(|s| s.name.clone())
                    .unwrap_or_default(),
                item: kind.map(|k| k.name.clone()).unwrap_or_default(),
            });
        };
        world.place(location, item)?;
        placed.push(location);
    }

    // Lieux exclus : du remplissage d'abord (`fill` a vérifié qu'il y en a assez).
    // `split_off` coupe la liste : `rest` garde le début, `for_excluded` prend la fin.
    let for_excluded = rest.split_off(rest.len().saturating_sub(excluded.len()));
    for (item, location) in for_excluded.into_iter().zip(excluded) {
        world.place(location, item)?;
        placed.push(location);
    }

    // Utiles puis le reste du remplissage : n'importe quel lieu libre restant.
    // `zip` associe un objet à un lieu ; les deux listes ont la même longueur.
    let remaining = priority.into_iter().chain(normal);
    for (item, location) in useful.into_iter().chain(rest).zip(remaining) {
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
    })
}
