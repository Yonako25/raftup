//! Équilibrage par temps de jeu (ADR 0030), **désactivé par défaut**.
//!
//! L'équilibrage de progression ([`crate::balance`]) compare des **nombres de lieux** :
//! un joueur à 30 % de ses lieux est « en retard » sur un joueur à 60 %. Mais un lieu de
//! Slime Rancher ne prend pas le même temps qu'un lieu d'APQuest. Ici, on raisonne en
//! **secondes** : chaque jeu a un temps moyen par lieu (donné par l'appelant, d'après le
//! profil du jeu), et on simule la partie pour voir qui attend, et combien de temps.
//!
//! La simulation ([`simulate`]) :
//!
//! - tous les joueurs jouent en même temps, chacun un lieu à la fois, dans l'ordre de
//!   création de ses lieux atteignables ; un lieu du joueur `p` lui prend `secondes[p]` ;
//! - un objet de progression est reçu dès que son lieu est visité (comme sur le serveur) ;
//! - un événement (lieu sans numéro réseau) ne prend pas de temps ;
//! - un joueur sans lieu atteignable **attend** : c'est le « BK » d'Archipelago.
//!
//! L'équilibrage ([`time_balance`]) répète, au plus `max_swaps` fois : prendre le joueur
//! qui attend le plus, chercher un objet de progression à lui posé dans un lieu visité
//! **après** le début de son attente, et l'échanger avec un objet sans importance d'un
//! lieu visité **avant**. L'échange n'est gardé que s'il fait baisser l'attente totale
//! (puis, à égalité, l'heure de fin de la partie) ; sinon il est défait.
//!
//! Tout est en secondes entières (`u64`), sans virgule : même résultat sur tous les
//! systèmes. Pas de hasard : les candidats sont parcourus dans un ordre fixe.
//!
//! Limites (voir l'ADR) : la simulation suppose qu'un joueur joue sans pause et visite
//! tous ses lieux. Un échange respecte les règles d'objet des lieux (`can_hold`, ADR 0029).

use logic::{sweep, ItemKindId, LocationId, MultiWorld, Progress, Reachability, State};

use crate::explain::explain;
use crate::place::{progress, FillError};

/// Temps moyen d'un lieu quand le jeu n'en donne pas (2 minutes), comme
/// `launcher_core::estimate`.
pub const DEFAULT_SECONDS_PER_LOCATION: u64 = 120;

/// Nombre d'échanges au plus, par défaut.
pub const DEFAULT_MAX_SWAPS: u32 = 32;

/// Objets essayés pour un joueur, à chaque tour (les plus tardifs d'abord ignorés).
const MAX_CANDIDATES: usize = 16;

/// Lieux de destination essayés pour chaque objet.
const MAX_TARGETS: usize = 4;

/// Réglages de l'équilibrage par temps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeBalance {
    /// Faux par défaut : l'équilibrage ne fait rien tant qu'on ne l'a pas demandé.
    pub enabled: bool,
    /// Secondes par lieu de chaque joueur (index = slot). Un slot absent, ou à 0, prend
    /// [`DEFAULT_SECONDS_PER_LOCATION`].
    pub seconds_per_location: Vec<u64>,
    /// Nombre d'échanges au plus.
    pub max_swaps: u32,
}

impl Default for TimeBalance {
    fn default() -> TimeBalance {
        TimeBalance {
            enabled: false,
            seconds_per_location: Vec::new(),
            max_swaps: DEFAULT_MAX_SWAPS,
        }
    }
}

/// Le temps d'un joueur dans la simulation, en secondes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerTime {
    /// Temps passé à jouer (lieux visités × secondes par lieu).
    pub play: u64,
    /// Heure de son dernier lieu visité (0 s'il n'en a visité aucun).
    pub finish: u64,
    /// Temps passé à attendre avant son dernier lieu : `finish - play`.
    pub idle: u64,
    /// Heure à laquelle il a attendu pour la première fois (s'il a attendu).
    pub first_stall: Option<u64>,
    /// Lieux jamais visités (inatteignables).
    pub unfinished: u32,
}

/// Le résultat d'une simulation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Timeline {
    /// Un par joueur, dans l'ordre des slots.
    pub players: Vec<PlayerTime>,
    /// Heure de visite de chaque lieu (index = lieu), `None` s'il n'est jamais visité.
    check_time: Vec<Option<u64>>,
}

impl Timeline {
    /// Somme des attentes de tous les joueurs.
    pub fn total_idle(&self) -> u64 {
        self.players.iter().map(|p| p.idle).sum()
    }

    /// Heure de fin du dernier joueur.
    pub fn makespan(&self) -> u64 {
        self.players.iter().map(|p| p.finish).max().unwrap_or(0)
    }

    /// Heure de visite d'un lieu.
    pub fn check_time(&self, location: LocationId) -> Option<u64> {
        self.check_time.get(location.index()).copied().flatten()
    }

    /// Note à faire baisser : d'abord l'attente totale, puis l'heure de fin.
    /// Un tuple se compare champ par champ, dans l'ordre.
    fn score(&self) -> (u64, u64) {
        (self.total_idle(), self.makespan())
    }
}

/// Ce que l'équilibrage a fait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimeBalanceReport {
    /// Objets de progression avancés.
    pub moved: u32,
    /// Attente totale (secondes) avant et après.
    pub idle_before: u64,
    pub idle_after: u64,
    /// Heure de fin du dernier joueur avant et après.
    pub makespan_before: u64,
    pub makespan_after: u64,
}

/// Secondes par lieu du joueur `slot`.
fn seconds_of(seconds: &[u64], slot: usize) -> u64 {
    match seconds.get(slot).copied() {
        Some(s) if s > 0 => s,
        _ => DEFAULT_SECONDS_PER_LOCATION,
    }
}

fn is_event(world: &MultiWorld, id: LocationId) -> bool {
    world.location(id).is_none_or(|l| l.address.is_none())
}

/// Lieu dont l'objet peut être échangé : ni événement, ni objet posé d'avance (ADR 0054).
/// Un lieu posé d'avance prend quand même du temps à visiter (il a un numéro).
fn movable(world: &MultiWorld, id: LocationId) -> bool {
    !world.is_locked(id)
}

fn progression_item(world: &MultiWorld, id: LocationId) -> Option<ItemKindId> {
    let item = world.location(id)?.item?;
    let kind = world.item_kind(item)?;
    kind.classification.progression.then_some(item)
}

/// Simule la partie depuis l'inventaire `start` (voir le module).
pub fn simulate(world: &MultiWorld, start: &State, seconds: &[u64]) -> Timeline {
    let slots = world.slots().len();
    let ids: Vec<LocationId> = world.location_ids().collect();
    let mut state = start.clone();
    let mut reach = Reachability::compute(world, &state);
    let mut done = vec![false; ids.len()];
    let mut check_time: Vec<Option<u64>> = vec![None; ids.len()];
    let mut players = vec![PlayerTime::default(); slots];
    // Lieux (avec numéro réseau) qui restent à chaque joueur.
    let mut remaining = vec![0u32; slots];
    for &id in &ids {
        if let Some(l) = world.location(id).filter(|l| l.address.is_some()) {
            if let Some(r) = remaining.get_mut(l.slot.index()) {
                *r += 1;
            }
        }
    }
    // Ce que fait chaque joueur : `Some((heure de fin, lieu))`, ou `None` s'il est libre.
    let mut busy: Vec<Option<(u64, LocationId)>> = vec![None; slots];
    let mut now = 0u64;

    loop {
        // 1. Les événements atteignables sont ramassés tout de suite (0 seconde) ; un
        //    événement peut en ouvrir un autre, d'où la boucle.
        loop {
            let mut changed = false;
            for &id in &ids {
                if !done.get(id.index()).copied().unwrap_or(true)
                    && is_event(world, id)
                    && reach.location(id)
                {
                    mark(&mut done, &mut check_time, id, now);
                    if let Some(item) = progression_item(world, id) {
                        changed |= state.collect(world, item);
                    }
                }
            }
            if !changed {
                break;
            }
            // L'inventaire n'a fait que grandir : `update` suffit (pas besoin de tout
            // recalculer).
            reach.update(world, &state);
        }

        // 2. Chaque joueur libre commence son premier lieu atteignable.
        for (slot, task) in busy.iter_mut().enumerate() {
            if task.is_some() {
                continue;
            }
            let next = ids.iter().copied().find(|id| {
                !done.get(id.index()).copied().unwrap_or(true)
                    && !is_event(world, *id)
                    && world.location(*id).map(|l| l.slot.index()) == Some(slot)
                    && reach.location(*id)
            });
            match next {
                Some(id) => *task = Some((now + seconds_of(seconds, slot), id)),
                None => {
                    // Libre alors qu'il lui reste des lieux : il attend.
                    if let Some(p) = players.get_mut(slot) {
                        if remaining.get(slot).copied().unwrap_or(0) > 0 {
                            p.first_stall.get_or_insert(now);
                        }
                    }
                }
            }
        }

        // 3. On avance jusqu'au prochain lieu terminé.
        let Some(next) = busy.iter().flatten().map(|(end, _)| *end).min() else {
            break;
        };
        now = next;
        for (slot, task) in busy.iter_mut().enumerate() {
            let Some((end, id)) = *task else {
                continue;
            };
            if end != now {
                continue;
            }
            *task = None;
            mark(&mut done, &mut check_time, id, now);
            if let Some(r) = remaining.get_mut(slot) {
                *r = r.saturating_sub(1);
            }
            if let Some(p) = players.get_mut(slot) {
                p.play += seconds_of(seconds, slot);
                p.finish = now;
            }
            if let Some(item) = progression_item(world, id) {
                state.collect(world, item);
            }
        }
        reach.update(world, &state);
    }

    for (slot, p) in players.iter_mut().enumerate() {
        p.idle = p.finish.saturating_sub(p.play);
        p.unfinished = remaining.get(slot).copied().unwrap_or(0);
    }
    Timeline {
        players,
        check_time,
    }
}

fn mark(done: &mut [bool], check_time: &mut [Option<u64>], id: LocationId, now: u64) {
    if let Some(d) = done.get_mut(id.index()) {
        *d = true;
    }
    if let Some(t) = check_time.get_mut(id.index()) {
        *t = Some(now);
    }
}

/// Équilibre un monde déjà rempli par [`crate::fill`] en temps de jeu (voir le module).
///
/// Ne fait rien si `settings.enabled` est faux (le rapport donne quand même les temps
/// simulés). Les objets ne font qu'échanger leur place : le pool ne change pas. Si la
/// partie n'est plus jouable à la fin (ce qui ne devrait pas arriver : on n'avance que de
/// la progression), le monde est remis comme avant et l'erreur est renvoyée.
pub fn time_balance(
    world: &mut MultiWorld,
    settings: &TimeBalance,
    start: &State,
) -> Result<TimeBalanceReport, FillError> {
    let seconds = &settings.seconds_per_location;
    let mut timeline = simulate(world, start, seconds);
    let mut report = TimeBalanceReport {
        moved: 0,
        idle_before: timeline.total_idle(),
        idle_after: timeline.total_idle(),
        makespan_before: timeline.makespan(),
        makespan_after: timeline.makespan(),
    };
    if !settings.enabled {
        return Ok(report);
    }
    let before: Vec<Option<ItemKindId>> = world.locations().iter().map(|l| l.item).collect();
    while report.moved < settings.max_swaps {
        match improve_once(world, start, seconds, &timeline)? {
            Some(better) => {
                timeline = better;
                report.moved += 1;
            }
            None => break,
        }
    }
    report.idle_after = timeline.total_idle();
    report.makespan_after = timeline.makespan();
    if report.moved > 0 {
        let result = sweep(world, start.clone());
        let blocked = explain(world, &result);
        if !blocked.is_empty() {
            restore(world, &before);
            return Err(FillError::Unbeatable { blocked });
        }
    }
    Ok(report)
}

/// Cherche un échange qui améliore la note ; le garde et renvoie la nouvelle simulation,
/// ou `None` si aucun échange essayé n'aide.
fn improve_once(
    world: &mut MultiWorld,
    start: &State,
    seconds: &[u64],
    current: &Timeline,
) -> Result<Option<Timeline>, FillError> {
    // Joueurs qui ont attendu, du plus long au plus court (à égalité : slot croissant).
    let mut waiting: Vec<(usize, u64, u64)> = current
        .players
        .iter()
        .enumerate()
        .filter_map(|(slot, p)| p.first_stall.map(|t| (slot, p.idle, t)))
        .filter(|(_, idle, _)| *idle > 0)
        .collect();
    // `Reverse` : tri décroissant sur l'attente, puis croissant sur le slot.
    waiting.sort_by_key(|(slot, idle, _)| (std::cmp::Reverse(*idle), *slot));

    for (player, _, stall) in waiting {
        let candidates = candidates(world, current, player, stall);
        let targets = targets(world, current, stall);
        for &late in &candidates {
            for &early in &targets {
                // Règles d'objet (ADR 0029) : chaque objet doit être accepté par son
                // nouveau lieu, sinon on n'essaie même pas.
                if !fits(world, late, early) {
                    continue;
                }
                swap(world, late, early)?;
                let trial = simulate(world, start, seconds);
                if trial.score() < current.score() {
                    return Ok(Some(trial));
                }
                // Pas mieux : on défait l'échange.
                swap(world, late, early)?;
            }
        }
    }
    Ok(None)
}

/// Lieux dont l'objet de progression du joueur `player` arrive après `stall`, du plus
/// tôt au plus tard. Un lieu prioritaire garde son objet, comme dans [`crate::balance`].
fn candidates(
    world: &MultiWorld,
    timeline: &Timeline,
    player: usize,
    stall: u64,
) -> Vec<LocationId> {
    let mut found: Vec<(u64, LocationId)> = world
        .location_ids()
        .filter(|id| movable(world, *id) && progress(world, *id) != Progress::Priority)
        .filter(|id| {
            progression_item(world, *id)
                .and_then(|item| world.item_kind(item))
                .is_some_and(|k| k.slot.index() == player && !k.classification.skip_balancing)
        })
        .filter_map(|id| match timeline.check_time(id) {
            Some(t) if t <= stall => None,
            // Jamais visité : rangé tout à la fin.
            Some(t) => Some((t, id)),
            None => Some((u64::MAX, id)),
        })
        .collect();
    found.sort();
    found
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|(_, id)| id)
        .collect()
}

/// Lieux visités au plus tard à `stall`, sans objet de progression, qui peuvent en
/// recevoir un (pas exclus) ; du plus tard au plus tôt.
fn targets(world: &MultiWorld, timeline: &Timeline, stall: u64) -> Vec<LocationId> {
    let mut found: Vec<(std::cmp::Reverse<u64>, LocationId)> = world
        .location_ids()
        .filter(|id| {
            movable(world, *id)
                && progression_item(world, *id).is_none()
                && progress(world, *id) != Progress::Excluded
        })
        .filter_map(|id| match timeline.check_time(id) {
            Some(t) if t <= stall => Some((std::cmp::Reverse(t), id)),
            _ => None,
        })
        .collect();
    found.sort();
    found
        .into_iter()
        .take(MAX_TARGETS)
        .map(|(_, id)| id)
        .collect()
}

/// Vrai si l'objet de `a` peut aller en `b` et celui de `b` en `a` (règles d'objet,
/// `MultiWorld::can_hold`). Un lieu vide ne gêne pas.
fn fits(world: &MultiWorld, a: LocationId, b: LocationId) -> bool {
    let item_a = world.location(a).and_then(|l| l.item);
    let item_b = world.location(b).and_then(|l| l.item);
    item_a.is_none_or(|i| world.can_hold(b, i)) && item_b.is_none_or(|i| world.can_hold(a, i))
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
