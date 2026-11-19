//! Tests de l'équilibrage par temps de jeu (ADR 0030), sur des mondes écrits à la main.

#![allow(clippy::unwrap_used)]

use fill::{simulate, time_balance, TimeBalance, TimeBalanceReport, DEFAULT_SECONDS_PER_LOCATION};
use logic::{
    sweep, Classification, ItemKindId, ItemRule, LocationId, MultiWorld, Progress, Rule, SlotId,
    State,
};

const PROG: Classification = Classification::PROGRESSION;

/// Deux joueurs, placement écrit à la main :
///
/// - Alice (100 s par lieu) : 3 lieux ouverts A1, A2, A3 ;
/// - Bob (10 s par lieu) : 1 lieu ouvert S, puis une porte « Key B » vers E1 et E2 ;
/// - « Key B » est en A3, le dernier lieu d'Alice : Bob attend de 10 s à 300 s.
///
/// Renvoie le monde, S et A3.
fn bob_attend() -> (MultiWorld, LocationId, LocationId) {
    let mut w = MultiWorld::new();
    let alice = w.add_slot("Alice", "Test").unwrap();
    let bob = w.add_slot("Bob", "Test").unwrap();
    let rupee_a = w
        .add_item_kind(alice, "Rupee", Some(1), Classification::FILLER)
        .unwrap();
    let win_a = w.add_item_kind(alice, "Victory", None, PROG).unwrap();
    let key_b = w.add_item_kind(bob, "Key B", Some(1), PROG).unwrap();
    let rupee_b = w
        .add_item_kind(bob, "Rupee", Some(2), Classification::FILLER)
        .unwrap();
    let win_b = w.add_item_kind(bob, "Victory", None, PROG).unwrap();

    let menu_a = w.add_region(alice, "Menu").unwrap();
    let mut a3 = None;
    for i in 1..=3 {
        let l = w
            .add_location(menu_a, format!("A{i}"), Some(i), Rule::True)
            .unwrap();
        w.place(l, if i == 3 { key_b } else { rupee_a }).unwrap();
        a3 = Some(l);
    }
    let end_a = w.add_location(menu_a, "End", None, Rule::True).unwrap();
    w.place(end_a, win_a).unwrap();
    w.set_goal(alice, Rule::has(win_a)).unwrap();

    let menu_b = w.add_region(bob, "Menu").unwrap();
    let deep_b = w.add_region(bob, "Deep").unwrap();
    w.add_entrance(menu_b, deep_b, "Gate", Rule::has(key_b))
        .unwrap();
    let s = w.add_location(menu_b, "S", Some(1), Rule::True).unwrap();
    w.place(s, rupee_b).unwrap();
    for i in 1..=2 {
        let l = w
            .add_location(deep_b, format!("E{i}"), Some(100 + i), Rule::True)
            .unwrap();
        w.place(l, rupee_b).unwrap();
    }
    let end_b = w.add_location(deep_b, "End", None, Rule::True).unwrap();
    w.place(end_b, win_b).unwrap();
    w.set_goal(bob, Rule::has(win_b)).unwrap();
    w.validate().unwrap();
    (w, s, a3.unwrap())
}

fn enabled(seconds: Vec<u64>) -> TimeBalance {
    TimeBalance {
        enabled: true,
        seconds_per_location: seconds,
        ..TimeBalance::default()
    }
}

fn placement(w: &MultiWorld) -> Vec<Option<ItemKindId>> {
    w.locations().iter().map(|l| l.item).collect()
}

/// Objets du monde, triés : le pool ne doit jamais changer.
fn pool(w: &MultiWorld) -> Vec<ItemKindId> {
    let mut items: Vec<ItemKindId> = w.locations().iter().filter_map(|l| l.item).collect();
    items.sort();
    items
}

fn slot(w: &MultiWorld, n: usize) -> SlotId {
    w.slot_ids().nth(n).unwrap()
}

#[test]
fn la_simulation_compte_les_secondes() {
    let (w, s, a3) = bob_attend();
    let t = simulate(&w, &State::new(&w), &[100, 10]);
    // Alice : A1 à 100 s, A2 à 200 s, A3 à 300 s, sans attendre.
    assert_eq!(t.players[0].play, 300);
    assert_eq!(t.players[0].finish, 300);
    assert_eq!(t.players[0].idle, 0);
    assert_eq!(t.players[0].first_stall, None);
    // Bob : S à 10 s, puis attend la clé (300 s) : E1 à 310 s, E2 à 320 s.
    assert_eq!(t.players[1].play, 30);
    assert_eq!(t.players[1].finish, 320);
    assert_eq!(t.players[1].idle, 290);
    assert_eq!(t.players[1].first_stall, Some(10));
    assert_eq!(t.check_time(s), Some(10));
    assert_eq!(t.check_time(a3), Some(300));
    assert_eq!(t.total_idle(), 290);
    assert_eq!(t.makespan(), 320);
    assert!(t.players.iter().all(|p| p.unfinished == 0));
}

#[test]
fn sans_temps_donne_le_defaut_s_applique() {
    let (w, _, _) = bob_attend();
    let t = simulate(&w, &State::new(&w), &[]);
    assert_eq!(t.players[0].play, 3 * DEFAULT_SECONDS_PER_LOCATION);
    // Un 0 vaut aussi « pas de temps donné ».
    let t0 = simulate(&w, &State::new(&w), &[0, 0]);
    assert_eq!(t, t0);
}

#[test]
fn desactive_par_defaut() {
    let (mut w, _, _) = bob_attend();
    let before = placement(&w);
    let settings = TimeBalance {
        seconds_per_location: vec![100, 10],
        ..TimeBalance::default()
    };
    assert!(!settings.enabled);
    let report = {
        let start = State::new(&w);
        time_balance(&mut w, &settings, &start).unwrap()
    };
    assert_eq!(report.moved, 0);
    assert_eq!(report.idle_before, 290);
    assert_eq!(report.idle_after, 290);
    assert_eq!(placement(&w), before);
}

#[test]
fn la_cle_de_bob_avance_et_il_n_attend_plus() {
    let (mut w, s, a3) = bob_attend();
    let before_pool = pool(&w);
    let start = State::new(&w);
    let report = time_balance(&mut w, &enabled(vec![100, 10]), &start).unwrap();
    assert_eq!(
        report,
        TimeBalanceReport {
            moved: 1,
            idle_before: 290,
            idle_after: 0,
            makespan_before: 320,
            makespan_after: 300,
        }
    );
    // La clé est passée dans S (le seul lieu visité avant l'attente), le remplissage en A3.
    let key_b = w.item_named(slot(&w, 1), "Key B").unwrap();
    assert_eq!(w.location(s).unwrap().item, Some(key_b));
    assert_ne!(w.location(a3).unwrap().item, Some(key_b));
    assert_eq!(pool(&w), before_pool);
    assert!(sweep(&w, start).all_goals_reached(&w));
}

#[test]
fn le_temps_par_lieu_change_la_decision() {
    // Si Alice est rapide (1 s par lieu) et Bob lent (1000 s), Bob reçoit sa clé à 3 s,
    // pendant son premier lieu : personne n'attend, rien ne bouge.
    let (mut w, _, _) = bob_attend();
    let before = placement(&w);
    let report = {
        let start = State::new(&w);
        time_balance(&mut w, &enabled(vec![1, 1000]), &start).unwrap()
    };
    assert_eq!(report.moved, 0);
    assert_eq!(report.idle_before, 0);
    assert_eq!(placement(&w), before);
}

#[test]
fn un_lieu_exclu_ne_recoit_pas_la_cle() {
    let (mut w, s, _) = bob_attend();
    w.set_progress(s, Progress::Excluded).unwrap();
    let before = placement(&w);
    let report = {
        let start = State::new(&w);
        time_balance(&mut w, &enabled(vec![100, 10]), &start).unwrap()
    };
    // S était la seule place possible : on ne fait rien plutôt que d'y mettre la clé.
    assert_eq!(report.moved, 0);
    assert_eq!(placement(&w), before);
}

#[test]
fn un_lieu_prioritaire_garde_son_objet() {
    let (mut w, _, a3) = bob_attend();
    w.set_progress(a3, Progress::Priority).unwrap();
    let before = placement(&w);
    let report = {
        let start = State::new(&w);
        time_balance(&mut w, &enabled(vec![100, 10]), &start).unwrap()
    };
    assert_eq!(report.moved, 0);
    assert_eq!(placement(&w), before);
}

#[test]
fn un_lieu_qui_refuse_la_cle_ne_la_recoit_pas() {
    // Règle d'objet (ADR 0029) sur S : « Key B » interdite. S était la seule place
    // possible : l'échange n'est pas tenté.
    let (mut w, s, _) = bob_attend();
    let key_b = w.item_named(slot(&w, 1), "Key B").unwrap();
    let rule = w
        .add_item_rule(ItemRule::forbid(vec![key_b], Vec::new()))
        .unwrap();
    w.set_item_rule(s, Some(rule)).unwrap();
    let before = placement(&w);
    let report = {
        let start = State::new(&w);
        time_balance(&mut w, &enabled(vec![100, 10]), &start).unwrap()
    };
    assert_eq!(report.moved, 0);
    assert_eq!(placement(&w), before);
}

#[test]
fn max_swaps_a_zero_ne_fait_rien() {
    let (mut w, _, _) = bob_attend();
    let before = placement(&w);
    let settings = TimeBalance {
        max_swaps: 0,
        ..enabled(vec![100, 10])
    };
    let report = {
        let start = State::new(&w);
        time_balance(&mut w, &settings, &start).unwrap()
    };
    assert_eq!(report.moved, 0);
    assert_eq!(placement(&w), before);
}

/// Trois joueurs, chacun bloqué par une clé posée au fond d'un autre monde.
fn en_chaine() -> MultiWorld {
    let mut w = MultiWorld::new();
    let names = ["P0", "P1", "P2"];
    let slots: Vec<SlotId> = names
        .iter()
        .map(|n| w.add_slot(*n, "Test").unwrap())
        .collect();
    let keys: Vec<ItemKindId> = slots
        .iter()
        .map(|s| w.add_item_kind(*s, "Key", Some(1), PROG).unwrap())
        .collect();
    let fillers: Vec<ItemKindId> = slots
        .iter()
        .map(|s| {
            w.add_item_kind(*s, "Coin", Some(2), Classification::FILLER)
                .unwrap()
        })
        .collect();
    for (i, s) in slots.iter().enumerate() {
        let menu = w.add_region(*s, "Menu").unwrap();
        let deep = w.add_region(*s, "Deep").unwrap();
        w.add_entrance(menu, deep, "Gate", Rule::has(keys[i]))
            .unwrap();
        for j in 1..=4 {
            let l = w
                .add_location(menu, format!("M{j}"), Some(j), Rule::True)
                .unwrap();
            // La clé du joueur suivant est dans le dernier lieu ouvert de celui-ci.
            let item = if j == 4 {
                keys[(i + 1) % 3]
            } else {
                fillers[i]
            };
            w.place(l, item).unwrap();
        }
        for j in 1..=4 {
            let l = w
                .add_location(deep, format!("D{j}"), Some(10 + j), Rule::True)
                .unwrap();
            w.place(l, fillers[i]).unwrap();
        }
        w.set_goal(*s, Rule::CanReachRegion(deep)).unwrap();
    }
    w.validate().unwrap();
    w
}

#[test]
fn plusieurs_joueurs_attente_qui_baisse_et_resultat_reproductible() {
    let seconds = vec![60, 120, 300];
    let mut a = en_chaine();
    let before_pool = pool(&a);
    let start = State::new(&a);
    let report = time_balance(&mut a, &enabled(seconds.clone()), &start).unwrap();
    assert!(report.moved > 0);
    assert!(report.idle_after < report.idle_before);
    assert_eq!(pool(&a), before_pool);
    assert!(sweep(&a, start.clone()).all_goals_reached(&a));
    // Même monde, mêmes réglages ⇒ même placement (aucun hasard).
    let mut b = en_chaine();
    let again = time_balance(&mut b, &enabled(seconds.clone()), &start).unwrap();
    assert_eq!(report, again);
    assert_eq!(placement(&a), placement(&b));
    // Le rapport dit vrai : on resimule le résultat.
    let t = simulate(&a, &start, &seconds);
    assert_eq!(t.total_idle(), report.idle_after);
    assert_eq!(t.makespan(), report.makespan_after);
}
