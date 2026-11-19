//! Règles d'objet, objets à placer tôt et accessibilité (ADR 0029).

#![allow(clippy::unwrap_used)]

use fill::{balance, fill, fill_with_early, EarlyItem, FillError};
use logic::{
    sweep, Accessibility, Classification, ItemKindId, ItemRule, LocationId, MultiWorld, Progress,
    Reachability, Rule, SlotId, State,
};
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;

const PROG: Classification = Classification::PROGRESSION;

fn rng(seed: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed)
}

/// Par joueur : chaîne de 4 régions derrière des clés, 3 lieux par région (12 lieux),
/// victoire au bout. Pool : 4 clés, 1 utile, 7 pièces.
fn monde(players: u32) -> (MultiWorld, Vec<ItemKindId>) {
    let mut w = MultiWorld::new();
    let mut pool = Vec::new();
    for n in 0..players {
        let p = w.add_slot(format!("P{n}"), "Test").unwrap();
        let boots = w
            .add_item_kind(p, "Boots", Some(100), Classification::USEFUL)
            .unwrap();
        let coin = w
            .add_item_kind(p, "Coin", Some(101), Classification::FILLER)
            .unwrap();
        let victory = w.add_item_kind(p, "Victory", None, PROG).unwrap();
        let mut prev = w.add_region(p, "R0").unwrap();
        for r in 0..4u32 {
            for l in 0..3u32 {
                w.add_location(
                    prev,
                    format!("R{r} L{l}"),
                    Some(i64::from(r * 10 + l)),
                    Rule::True,
                )
                .unwrap();
            }
            let key = w
                .add_item_kind(p, format!("Key {r}"), Some(i64::from(r)), PROG)
                .unwrap();
            pool.push(key);
            let next = w.add_region(p, format!("R{}", r + 1)).unwrap();
            w.add_entrance(prev, next, format!("Door {r}"), Rule::has(key))
                .unwrap();
            prev = next;
        }
        let end = w.add_location(prev, "End", None, Rule::True).unwrap();
        w.place(end, victory).unwrap();
        w.set_goal(p, Rule::has(victory)).unwrap();
        pool.push(boots);
        pool.extend(std::iter::repeat_n(coin, 7));
    }
    (w, pool)
}

fn slot(w: &MultiWorld, n: u32) -> SlotId {
    w.slot_ids().nth(n as usize).unwrap()
}

fn location(w: &MultiWorld, p: SlotId, name: &str) -> LocationId {
    w.location_named(p, name).unwrap()
}

/// Où est posé l'objet (premier lieu qui le contient).
fn where_is(w: &MultiWorld, item: ItemKindId) -> Vec<LocationId> {
    w.location_ids()
        .filter(|l| w.location(*l).unwrap().item == Some(item))
        .collect()
}

#[test]
fn regle_d_objet_respectee_avant_et_apres_equilibrage() {
    // Comme `local_items` : les clés de P0 ne vont jamais chez P1.
    for seed in 0..200 {
        let (mut w, pool) = monde(2);
        let p0 = slot(&w, 0);
        let p1 = slot(&w, 1);
        let keys: Vec<ItemKindId> = (0..4)
            .map(|r| w.item_named(p0, &format!("Key {r}")).unwrap())
            .collect();
        let rule = w
            .add_item_rule(ItemRule::forbid(keys.clone(), Vec::new()))
            .unwrap();
        let theirs: Vec<LocationId> = w
            .location_ids()
            .filter(|l| w.location(*l).unwrap().slot == p1)
            .collect();
        for l in &theirs {
            w.set_item_rule(*l, Some(rule)).unwrap();
        }
        let start = State::new(&w);
        let mut r = rng(seed);
        fill(&mut w, pool, &start, &mut r).unwrap();
        let check = |w: &MultiWorld| {
            for key in &keys {
                for l in where_is(w, *key) {
                    assert_eq!(w.location(l).unwrap().slot, p0, "seed {seed}");
                }
            }
        };
        check(&w);
        balance(&mut w, &[99, 99], &start, &mut r).unwrap();
        check(&w);
    }
}

#[test]
fn seuls_acceptes_et_toujours_acceptes() {
    let (mut w, pool) = monde(1);
    let p = slot(&w, 0);
    let coin = w.item_named(p, "Coin").unwrap();
    let key0 = w.item_named(p, "Key 0").unwrap();
    let boots = w.item_named(p, "Boots").unwrap();
    // « R0 L0 » n'accepte que des pièces ; « R0 L1 » est exclu mais accepte la clé 0.
    let only_coins = w
        .add_item_rule(ItemRule::allow_only(vec![coin], Vec::new()))
        .unwrap();
    let always_key = w
        .add_item_rule(ItemRule::forbid(Vec::new(), vec![key0]))
        .unwrap();
    let l0 = location(&w, p, "R0 L0");
    let l1 = location(&w, p, "R0 L1");
    w.set_item_rule(l0, Some(only_coins)).unwrap();
    w.set_item_rule(l1, Some(always_key)).unwrap();
    w.set_progress(l1, Progress::Excluded).unwrap();
    assert!(w.can_hold(l0, coin));
    assert!(!w.can_hold(l0, key0));
    assert!(w.can_hold(l1, key0), "always_allow passe l'exclusion");
    assert!(!w.can_hold(l1, boots), "exclu : pas d'objet utile");
    assert!(w.can_hold(l1, coin));
    for seed in 0..100 {
        let mut w = w.clone();
        let start = State::new(&w);
        fill(&mut w, pool.clone(), &start, &mut rng(seed)).unwrap();
        assert_eq!(w.location(l0).unwrap().item, Some(coin), "seed {seed}");
        let in_l1 = w.location(l1).unwrap().item.unwrap();
        assert!(in_l1 == coin || in_l1 == key0, "seed {seed}");
    }
}

#[test]
fn aucun_lieu_n_accepte_l_objet() {
    let (mut w, pool) = monde(1);
    let p = slot(&w, 0);
    let boots = w.item_named(p, "Boots").unwrap();
    let rule = w
        .add_item_rule(ItemRule::forbid(vec![boots], Vec::new()))
        .unwrap();
    for l in w.location_ids().collect::<Vec<_>>() {
        w.set_item_rule(l, Some(rule)).unwrap();
    }
    let before: Vec<_> = w.locations().iter().map(|l| l.item).collect();
    let start = State::new(&w);
    let err = fill(&mut w, pool, &start, &mut rng(1)).unwrap_err();
    assert!(
        matches!(err, FillError::NoAllowedLocation { ref item, .. } if item == "Boots"),
        "{err}"
    );
    // Monde remis comme avant.
    let after: Vec<_> = w.locations().iter().map(|l| l.item).collect();
    assert_eq!(before, after);
}

/// Lieux atteignables dès le départ (sphère 0) avec l'inventaire de départ.
fn sphere0(w: &MultiWorld, start: &State) -> Vec<LocationId> {
    let reach = Reachability::compute(w, start);
    w.location_ids().filter(|l| reach.location(*l)).collect()
}

#[test]
fn objets_a_placer_tot_en_sphere_0() {
    for seed in 0..200 {
        let (mut w, pool) = monde(2);
        let p0 = slot(&w, 0);
        let p1 = slot(&w, 1);
        let key3 = w.item_named(p0, "Key 3").unwrap();
        let boots = w.item_named(p1, "Boots").unwrap();
        let start = State::new(&w);
        let zero = sphere0(&w, &start);
        let early = [
            EarlyItem {
                item: key3,
                count: 1,
                local: false,
            },
            EarlyItem {
                item: boots,
                count: 1,
                local: true,
            },
        ];
        let report = fill_with_early(&mut w, pool, &start, &early, &mut rng(seed)).unwrap();
        assert_eq!(report.early_not_placed, 0);
        let k = where_is(&w, key3);
        assert!(zero.contains(&k[0]), "seed {seed} : clé 3 hors sphère 0");
        let b = where_is(&w, boots);
        assert!(zero.contains(&b[0]), "seed {seed} : bottes hors sphère 0");
        assert_eq!(w.location(b[0]).unwrap().slot, p1, "local : chez P1");
    }
}

#[test]
fn trop_d_objets_a_placer_tot() {
    // 3 lieux en sphère 0 pour 7 pièces à placer tôt : 4 vont plus loin.
    let (mut w, pool) = monde(1);
    let p = slot(&w, 0);
    let coin = w.item_named(p, "Coin").unwrap();
    let start = State::new(&w);
    let early = [EarlyItem {
        item: coin,
        count: 7,
        local: true,
    }];
    // Les pièces prennent toute la sphère 0 : la clé 0 n'a plus de place, le placement
    // échoue proprement (Archipelago échangerait ; Raftup repasse alors au Python).
    let err = fill_with_early(&mut w, pool.clone(), &start, &early, &mut rng(3)).unwrap_err();
    assert!(
        matches!(err, FillError::NoReachableLocation { .. }),
        "{err}"
    );
    // Avec 2 pièces seulement : une place reste pour la clé.
    let early = [EarlyItem {
        item: coin,
        count: 2,
        local: false,
    }];
    let report = fill_with_early(&mut w, pool, &start, &early, &mut rng(3)).unwrap();
    assert_eq!(report.early_not_placed, 0);
    let zero = sphere0(&w, &start);
    let coins_early = zero
        .iter()
        .filter(|l| w.location(**l).unwrap().item == Some(coin))
        .count();
    assert!(coins_early >= 2);
}

#[test]
fn objets_a_placer_tot_sans_place() {
    // Les 3 lieux de la sphère 0 n'acceptent pas les bottes : elles vont plus loin.
    let (mut w, pool) = monde(1);
    let p = slot(&w, 0);
    let boots = w.item_named(p, "Boots").unwrap();
    let rule = w
        .add_item_rule(ItemRule::forbid(vec![boots], Vec::new()))
        .unwrap();
    for l in 0..3 {
        let id = location(&w, p, &format!("R0 L{l}"));
        w.set_item_rule(id, Some(rule)).unwrap();
    }
    let start = State::new(&w);
    let early = [EarlyItem {
        item: boots,
        count: 1,
        local: false,
    }];
    let report = fill_with_early(&mut w, pool, &start, &early, &mut rng(5)).unwrap();
    assert_eq!(report.early_not_placed, 1);
    assert_eq!(where_is(&w, boots).len(), 1);
}

#[test]
fn accessibilite_minimale() {
    // Un lieu que rien n'ouvre : impossible en `full`, accepté en `minimal`.
    let (mut w, pool) = monde(2);
    let p1 = slot(&w, 1);
    let r0 = w.region_named(p1, "R0").unwrap();
    let mut pool = pool;
    let coin = w.item_named(p1, "Coin").unwrap();
    w.add_location(r0, "Never", Some(999), Rule::False).unwrap();
    pool.push(coin);
    let start = State::new(&w);
    let err = fill(&mut w.clone(), pool.clone(), &start, &mut rng(1)).unwrap_err();
    assert!(matches!(err, FillError::Impossible { .. }), "{err}");

    w.set_accessibility(p1, Accessibility::Minimal).unwrap();
    for seed in 0..200 {
        let mut w = w.clone();
        let mut r = rng(seed);
        fill(&mut w, pool.clone(), &start, &mut r).unwrap();
        balance(&mut w, &[50, 50], &start, &mut r).unwrap();
        let never = location(&w, p1, "Never");
        let held = w.location(never).unwrap().item.unwrap();
        let kind = w.item_kind(held).unwrap();
        // Le lieu fermé ne garde rien d'important pour P0 (en `full`).
        assert!(
            !(kind.slot != p1 && (kind.classification.progression || kind.classification.useful)),
            "seed {seed}"
        );
        // Tout le reste de P0 est atteint, et les deux victoires aussi.
        let result = sweep(&w, start.clone());
        assert!(result.all_goals_reached(&w), "seed {seed}");
        for l in result.unreachable_locations(&w) {
            assert!(!w.must_reach(l), "seed {seed}");
            assert_eq!(w.location(l).unwrap().slot, p1);
        }
    }
}

#[test]
fn minimal_les_objets_inutiles_peuvent_rester_hors_d_atteinte() {
    // P0 en `minimal`, victoire avec la seule clé 0 : les clés 1 à 3 ne servent à rien.
    // Sur certaines seeds, une clé finit derrière sa propre porte et des lieux restent
    // fermés, ce que `full` n'accepterait jamais.
    let mut found = false;
    for seed in 0..200 {
        let (mut w, pool) = monde(1);
        let p = slot(&w, 0);
        w.set_accessibility(p, Accessibility::Minimal).unwrap();
        let key0 = w.item_named(p, "Key 0").unwrap();
        w.set_goal(p, Rule::has(key0)).unwrap();
        let start = State::new(&w);
        fill(&mut w, pool, &start, &mut rng(seed)).unwrap();
        let result = sweep(&w, start.clone());
        assert!(result.all_goals_reached(&w), "seed {seed}");
        if !result.unreachable_locations(&w).is_empty() {
            found = true;
        }
    }
    assert!(found, "jamais de lieu hors d'atteinte en minimal");
}
