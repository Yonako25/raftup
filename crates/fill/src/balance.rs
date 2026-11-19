//! Équilibrage de progression (ADR 0027, étape 5), d'après
//! `balance_multiworld_progression` d'Archipelago 0.6.8 (`Fill.py`, MIT).
//!
//! But : éviter qu'un joueur reste bloqué (« BK ») pendant que les autres avancent. On
//! parcourt la partie sphère par sphère. À chaque sphère, on regarde quelle part de ses
//! lieux chaque joueur peut déjà visiter. Le joueur le plus avancé fixe la barre ; un
//! joueur qui a l'équilibrage à `b` (0 à 99) doit atteindre `b %` de cette part. S'il est
//! en dessous, on va chercher dans les sphères suivantes les objets de progression **à
//! lui** posés **chez les autres**, et on les échange avec des objets sans importance
//! (utiles ou remplissage) de sphères déjà visitées : ils arrivent plus tôt.
//!
//! Différences avec Archipelago :
//!
//! - aucune virgule : les pourcentages sont comparés par produits en croix d'entiers, pour
//!   un résultat identique sur tous les systèmes ;
//! - les lieux sont parcourus dans l'ordre de création (index croissant), pas dans l'ordre
//!   d'un `set` Python ; le hasard vient du même flux que le placement ;
//! - il n'y a ni règle d'objet par lieu (`item_rule`) ni lieu « prioritaire » dans le
//!   modèle : tout lieu libre accepte tout objet ;
//! - un lieu « verrouillé » est un lieu sans numéro réseau (un événement) ;
//! - à la fin, on vérifie que la partie reste jouable de bout en bout.

use logic::{sweep, ItemKindId, LocationId, MultiWorld, Reachability, SlotId, State};
use rand_core::Rng;

use crate::explain::explain;
use crate::place::FillError;
use crate::shuffle::shuffle;

/// Valeur la plus haute de `progression_balancing` dans Archipelago (« extreme »).
pub const MAX_BALANCING: u8 = 99;

/// Valeur par défaut de `progression_balancing` dans Archipelago (« normal »).
pub const DEFAULT_BALANCING: u8 = 50;

/// Résultat de l'équilibrage.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BalanceReport {
    /// Nombre d'objets de progression avancés.
    pub moved: u32,
    /// Objets qu'on aurait voulu avancer sans trouver de place.
    pub not_moved: u32,
}

/// Une proportion `num / den` (lieux atteints / lieux du joueur), comparée sans virgule.
#[derive(Debug, Clone, Copy)]
struct Fraction {
    num: u64,
    den: u64,
}

impl Fraction {
    /// `self < other`, par produit en croix (`a/b < c/d` ⇔ `a·d < c·b`, `b` et `d` > 0).
    /// `u128` : aucun débordement possible avec des compteurs `u64` multipliés.
    fn less_than(self, other: Fraction) -> bool {
        u128::from(self.num) * u128::from(other.den) < u128::from(other.num) * u128::from(self.den)
    }
}

/// Tout ce que l'algorithme garde sur la partie, pour ne pas le recalculer.
struct Balancer<'a> {
    world: &'a MultiWorld,
    /// Réglage de chaque joueur (index = slot), 0 si absent.
    settings: &'a [u8],
    /// Nombre de lieux non verrouillés de chaque joueur.
    total: &'a [u64],
}

impl Balancer<'_> {
    fn locked(&self, location: LocationId) -> bool {
        self.world
            .location(location)
            .is_none_or(|l| l.address.is_none())
    }

    fn owner(&self, location: LocationId) -> Option<SlotId> {
        self.world.location(location).map(|l| l.slot)
    }

    /// Objet posé dans le lieu, s'il est de progression.
    fn progression_item(&self, location: LocationId) -> Option<ItemKindId> {
        let item = self.world.location(location)?.item?;
        let kind = self.world.item_kind(item)?;
        kind.classification.progression.then_some(item)
    }

    /// Les lieux encore `unchecked` atteignables avec `state`, retirés de `unchecked`.
    /// `counts` reçoit +1 par lieu non verrouillé de chaque joueur.
    fn next_sphere(
        &self,
        state: &State,
        unchecked: &mut [bool],
        counts: &mut [u64],
    ) -> Vec<LocationId> {
        let reach = Reachability::compute(self.world, state);
        let mut sphere = Vec::new();
        for id in self.world.location_ids() {
            if flag(unchecked, id.index()) && reach.location(id) {
                set(unchecked, id.index(), false);
                if !self.locked(id) {
                    if let Some(slot) = self.owner(id) {
                        bump(counts, slot.index());
                    }
                }
                sphere.push(id);
            }
        }
        sphere
    }

    /// Part des lieux du joueur `slot` atteinte avec `n` lieux visités.
    fn share(&self, slot: usize, n: u64) -> Fraction {
        Fraction {
            num: n,
            den: self.total.get(slot).copied().unwrap_or(1).max(1),
        }
    }

    /// Le joueur `slot` est sous sa barre avec `n` lieux visités. La barre vaut
    /// `best × réglage / 100`, où `best` est la part du joueur le plus avancé.
    fn below_threshold(&self, slot: usize, n: u64, best: Fraction) -> bool {
        let setting = u64::from(self.settings.get(slot).copied().unwrap_or(0));
        let threshold = Fraction {
            num: best.num * setting,
            den: best.den * 100,
        };
        self.share(slot, n).less_than(threshold)
    }

    /// Joueurs qui ont l'équilibrage et au moins un lieu.
    fn balanceable(&self, slot: usize) -> bool {
        self.settings.get(slot).copied().unwrap_or(0) > 0
            && self.total.get(slot).copied().unwrap_or(0) > 0
    }

    /// La part la plus haute parmi les joueurs qui ont des lieux.
    fn best_share(&self, counts: &[u64]) -> Fraction {
        let mut best = Fraction { num: 0, den: 1 };
        for (slot, n) in counts.iter().enumerate() {
            if self.total.get(slot).copied().unwrap_or(0) == 0 {
                continue;
            }
            let share = self.share(slot, *n);
            if best.less_than(share) {
                best = share;
            }
        }
        best
    }

    /// Tous les joueurs à équilibrer ont atteint leur barre.
    fn all_above(&self, counts: &[u64], best: Fraction) -> bool {
        (0..counts.len()).all(|slot| {
            !self.balanceable(slot)
                || !self.below_threshold(slot, counts.get(slot).copied().unwrap_or(0), best)
        })
    }

    fn beaten(&self, state: &State) -> bool {
        let reach = Reachability::compute(self.world, state);
        self.world.slot_ids().all(|slot| {
            self.world
                .slot(slot)
                .is_some_and(|s| s.goal.eval(self.world, state, &reach))
        })
    }

    /// Ramasse, tant que c'est possible, les objets des lieux `locations` atteignables
    /// (`sweep_for_advancements` d'Archipelago, limité à ces lieux). Renvoie ce qui est
    /// atteignable à la fin.
    fn sweep_within(&self, state: &mut State, locations: &[LocationId]) -> Reachability {
        let mut taken = vec![false; locations.len()];
        loop {
            let reach = Reachability::compute(self.world, state);
            let mut progressed = false;
            for (i, id) in locations.iter().enumerate() {
                if !flag(&taken, i) && reach.location(*id) {
                    set(&mut taken, i, true);
                    if let Some(item) = self.progression_item(*id) {
                        state.collect(self.world, item);
                        progressed = true;
                    }
                }
            }
            if !progressed {
                return reach;
            }
        }
    }
}

fn flag(flags: &[bool], index: usize) -> bool {
    flags.get(index).copied().unwrap_or(false)
}

fn set(flags: &mut [bool], index: usize, value: bool) {
    if let Some(f) = flags.get_mut(index) {
        *f = value;
    }
}

fn bump(counts: &mut [u64], index: usize) {
    if let Some(c) = counts.get_mut(index) {
        *c += 1;
    }
}

fn count(counts: &[u64], index: usize) -> u64 {
    counts.get(index).copied().unwrap_or(0)
}

/// Équilibre la progression d'un monde déjà rempli par [`crate::fill`].
///
/// - `settings` : `progression_balancing` de chaque joueur, dans l'ordre des slots (0 à
///   [`MAX_BALANCING`] ; 0 = pas d'équilibrage pour ce joueur ; un slot absent vaut 0) ;
/// - `start` : inventaire de départ ;
/// - `rng` : mêmes nombres ⇒ mêmes échanges.
///
/// Les objets ne font qu'**échanger** leur place : le pool reste le même. Si, malgré tout,
/// la partie n'est plus jouable à la fin, l'erreur est renvoyée et le monde est remis
/// comme avant l'appel.
pub fn balance<R: Rng + ?Sized>(
    world: &mut MultiWorld,
    settings: &[u8],
    start: &State,
    rng: &mut R,
) -> Result<BalanceReport, FillError> {
    let before: Vec<Option<ItemKindId>> = world.locations().iter().map(|l| l.item).collect();
    let mut report = BalanceReport::default();
    let swaps = plan_and_swap(world, settings, start, rng, &mut report)?;
    if swaps == 0 {
        return Ok(report);
    }
    // Vérification finale : les échanges n'avancent que de la progression, la partie doit
    // rester jouable ; on le contrôle quand même plutôt que de rendre une seed injouable.
    let result = sweep(world, start.clone());
    let blocked = explain(world, &result);
    if !blocked.is_empty() {
        restore(world, &before);
        return Err(FillError::Unbeatable { blocked });
    }
    Ok(report)
}

/// Remet chaque lieu avec l'objet qu'il avait.
fn restore(world: &mut MultiWorld, before: &[Option<ItemKindId>]) {
    let ids: Vec<LocationId> = world.location_ids().collect();
    for id in ids {
        // Les lieux existent : `take` et `place` ne peuvent pas échouer ici.
        let _ = world.take(id);
        if let Some(Some(item)) = before.get(id.index()) {
            let _ = world.place(id, *item);
        }
    }
}

/// Échange les objets de deux lieux.
fn swap(world: &mut MultiWorld, a: LocationId, b: LocationId) -> Result<(), FillError> {
    let item_a = world.take(a)?;
    let item_b = world.take(b)?;
    if let Some(item) = item_b {
        world.place(a, item)?;
    }
    if let Some(item) = item_a {
        world.place(b, item)?;
    }
    Ok(())
}

/// Le cœur de l'algorithme ; renvoie le nombre d'échanges faits.
fn plan_and_swap<R: Rng + ?Sized>(
    world: &mut MultiWorld,
    settings: &[u8],
    start: &State,
    rng: &mut R,
    report: &mut BalanceReport,
) -> Result<u32, FillError> {
    let slots = world.slots().len();
    let locations = world.locations().len();
    let mut total = vec![0u64; slots];
    for id in world.location_ids() {
        let location = world.location(id);
        if let Some(l) = location.filter(|l| l.address.is_some()) {
            bump(&mut total, l.slot.index());
        }
    }
    let mut padded = settings.to_vec();
    padded.resize(slots, 0);
    for s in &mut padded {
        *s = (*s).min(MAX_BALANCING);
    }
    if !(0..slots).any(|s| padded.get(s).copied().unwrap_or(0) > 0 && count(&total, s) > 0) {
        return Ok(0);
    }

    let mut state = start.clone();
    let mut unchecked = vec![true; locations];
    let mut checked = vec![false; locations];
    let mut any_checked = false;
    let mut counts = vec![0u64; slots];
    let mut swaps = 0u32;

    loop {
        // Le monde change pendant les échanges : on recrée la vue en lecture à chaque tour.
        // (On ne peut pas garder un `&MultiWorld` pendant qu'on le modifie.)
        let b = Balancer {
            world,
            settings: &padded,
            total: &total,
        };
        let mut sphere = b.next_sphere(&state, &mut unchecked, &mut counts);

        let mut to_swap: Vec<(LocationId, LocationId)> = Vec::new();
        let mut unlocked_all: Vec<LocationId> = Vec::new();
        if any_checked {
            let best = b.best_share(&counts);
            let balancing: Vec<usize> = (0..slots)
                .filter(|s| b.balanceable(*s) && b.below_threshold(*s, count(&counts, *s), best))
                .collect();
            if !balancing.is_empty() {
                let planned = plan(
                    &b, &state, &unchecked, &counts, &sphere, &balancing, best, rng,
                )?;
                to_swap = pair_up(&b, &checked, planned.items_to_replace, rng, report);
                unlocked_all = planned.unlocked;
            }
        }

        // Fin de la lecture : on peut modifier le monde.
        let moved_now = !to_swap.is_empty();
        for (old, new) in &to_swap {
            swap(world, *old, *new)?;
            swaps += 1;
            report.moved += 1;
            // L'objet de progression est maintenant dans un lieu déjà visité.
            let item = world.location(*new).and_then(|l| l.item);
            if let Some(item) = item {
                state.collect(world, item);
            }
        }
        if moved_now {
            // Des lieux du joueur aidé s'ouvrent dès cette sphère.
            let b = Balancer {
                world,
                settings: &padded,
                total: &total,
            };
            let reach = Reachability::compute(world, &state);
            for id in unlocked_all {
                if flag(&unchecked, id.index()) && reach.location(id) {
                    set(&mut unchecked, id.index(), false);
                    if !b.locked(id) {
                        if let Some(slot) = b.owner(id) {
                            bump(&mut counts, slot.index());
                        }
                    }
                    sphere.push(id);
                }
            }
        }

        let b = Balancer {
            world,
            settings: &padded,
            total: &total,
        };
        for id in &sphere {
            if let Some(item) = b.progression_item(*id) {
                state.collect(world, item);
            }
            set(&mut checked, id.index(), true);
        }
        any_checked |= !sphere.is_empty();
        if b.beaten(&state) || sphere.is_empty() {
            return Ok(swaps);
        }
    }
}

/// Ce que la recherche a trouvé pour une sphère.
struct Planned {
    /// Lieux (plus tardifs) dont l'objet de progression doit avancer.
    items_to_replace: Vec<LocationId>,
    /// Lieux des joueurs aidés qui s'ouvrent si ces objets avancent.
    unlocked: Vec<LocationId>,
}

/// Cherche les objets à avancer pour les joueurs `balancing`.
#[allow(clippy::too_many_arguments)]
fn plan<R: Rng + ?Sized>(
    b: &Balancer<'_>,
    state: &State,
    unchecked: &[bool],
    counts: &[u64],
    sphere: &[LocationId],
    balancing: &[usize],
    best: Fraction,
    rng: &mut R,
) -> Result<Planned, FillError> {
    let world = b.world;
    let mut b_state = state.clone();
    let mut b_unchecked = unchecked.to_vec();
    let mut b_counts = counts.to_vec();
    let mut b_sphere = sphere.to_vec();
    let mut candidate = vec![false; world.locations().len()];

    // On avance sphère par sphère dans le futur jusqu'à ce que tous les joueurs à
    // équilibrer aient atteint leur barre (ou que la partie soit gagnée), en notant les
    // objets de progression d'un joueur aidé posés chez un autre joueur.
    loop {
        for id in &b_sphere {
            let Some(item) = b.progression_item(*id) else {
                continue;
            };
            b_state.collect(world, item);
            let Some(kind) = world.item_kind(item) else {
                continue;
            };
            let owner = kind.slot.index();
            if !b.locked(*id)
                && !kind.classification.skip_balancing
                && balancing.contains(&owner)
                && b.owner(*id).map(|s| s.index()) != Some(owner)
            {
                set(&mut candidate, id.index(), true);
            }
        }
        b_sphere = b.next_sphere(&b_state, &mut b_unchecked, &mut b_counts);
        if b.beaten(&b_state) || b.all_above(&b_counts, best) {
            break;
        }
        if b_sphere.is_empty() {
            // Impossible après un placement jouable ; on le signale au lieu de boucler.
            let result = sweep(world, state.clone());
            return Err(FillError::Unbeatable {
                blocked: explain(world, &result),
            });
        }
    }
    let beaten_later = b.beaten(&b_state);

    // Lieux encore à visiter maintenant, mais visités pendant la recherche.
    let mut unlocked_all = Vec::new();
    let mut items_to_replace: Vec<LocationId> = Vec::new();
    for &player in balancing {
        let unlocked: Vec<LocationId> = world
            .location_ids()
            .filter(|id| {
                flag(unchecked, id.index())
                    && !flag(&b_unchecked, id.index())
                    && b.owner(*id).map(|s| s.index()) == Some(player)
            })
            .collect();
        let mut to_test: Vec<LocationId> = world
            .location_ids()
            .filter(|id| {
                flag(&candidate, id.index())
                    && b.progression_item(*id)
                        .and_then(|i| world.item_kind(i))
                        .map(|k| k.slot.index())
                        == Some(player)
            })
            .collect();
        shuffle(rng, &mut to_test);
        // On essaie de laisser chaque objet à sa place : s'il manque pour atteindre la
        // barre (ou la victoire), il doit avancer.
        while let Some(testing) = to_test.pop() {
            let mut reducing = state.clone();
            let keep = items_to_replace
                .iter()
                .filter(|l| {
                    b.progression_item(**l)
                        .and_then(|i| world.item_kind(i))
                        .map(|k| k.slot.index())
                        == Some(player)
                })
                .chain(to_test.iter());
            for l in keep {
                if let Some(item) = b.progression_item(*l) {
                    reducing.collect(world, item);
                }
            }
            let reach = b.sweep_within(&mut reducing, &unlocked);
            let needed = if beaten_later {
                !b.beaten(&reducing)
            } else {
                let reduced = unlocked.iter().filter(|id| reach.location(**id)).count() as u64;
                b.below_threshold(player, count(counts, player) + reduced, best)
            };
            if needed {
                items_to_replace.push(testing);
            }
        }
        unlocked_all.extend(unlocked);
    }
    Ok(Planned {
        items_to_replace,
        unlocked: unlocked_all,
    })
}

/// Associe chaque objet à avancer à un lieu déjà visité qui ne contient pas de progression.
/// Renvoie les paires (lieu tardif, lieu visité).
fn pair_up<R: Rng + ?Sized>(
    b: &Balancer<'_>,
    checked: &[bool],
    mut items: Vec<LocationId>,
    rng: &mut R,
    report: &mut BalanceReport,
) -> Vec<(LocationId, LocationId)> {
    let mut replacement: Vec<LocationId> = b
        .world
        .location_ids()
        .filter(|id| {
            flag(checked, id.index()) && !b.locked(*id) && b.progression_item(*id).is_none()
        })
        .collect();
    shuffle(rng, &mut replacement);
    items.sort();
    shuffle(rng, &mut items);
    let mut pairs = Vec::new();
    // Tout lieu accepte tout objet (pas de règle d'objet) : le premier lieu libre convient.
    while let Some(old) = items.pop() {
        if replacement.is_empty() {
            report.not_moved += 1;
            continue;
        }
        let new = replacement.remove(0);
        pairs.push((old, new));
    }
    pairs
}
