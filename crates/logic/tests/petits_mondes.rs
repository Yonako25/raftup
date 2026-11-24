//! Tests de `logic` sur de petits mondes écrits à la main (ADR 0027, étape 1).

// Les fonctions d'aide (hors `#[test]`) paniquent aussi en cas d'échec : c'est voulu.
#![allow(clippy::unwrap_used)]

use logic::{sweep, Classification, LogicError, MultiWorld, Reachability, Rule, State};

const PROG: Classification = Classification::PROGRESSION;

/// Monde linéaire : Menu (coffre : épée) → Forêt (épée ; lieu : clé) → Château (clé ;
/// lieu : boss). Victoire : atteindre le boss.
fn monde_lineaire() -> (MultiWorld, [logic::LocationId; 3]) {
    let mut w = MultiWorld::new();
    let p = w.add_slot("Alice", "Test").unwrap();
    let sword = w.add_item_kind(p, "Sword", Some(1), PROG).unwrap();
    let key = w.add_item_kind(p, "Key", Some(2), PROG).unwrap();
    let menu = w.add_region(p, "Menu").unwrap();
    let forest = w.add_region(p, "Forest").unwrap();
    let castle = w.add_region(p, "Castle").unwrap();
    w.add_entrance(menu, forest, "Menu -> Forest", Rule::has(sword))
        .unwrap();
    w.add_entrance(forest, castle, "Forest -> Castle", Rule::has(key))
        .unwrap();
    let chest = w.add_location(menu, "Chest", Some(10), Rule::True).unwrap();
    let stump = w
        .add_location(forest, "Stump", Some(11), Rule::True)
        .unwrap();
    let boss = w.add_location(castle, "Boss", None, Rule::True).unwrap();
    w.place(chest, sword).unwrap();
    w.place(stump, key).unwrap();
    w.set_goal(p, Rule::CanReachLocation(boss)).unwrap();
    w.validate().unwrap();
    (w, [chest, stump, boss])
}

#[test]
fn spheres_d_un_monde_lineaire() {
    let (w, [chest, stump, boss]) = monde_lineaire();
    let result = sweep(&w, State::new(&w));
    assert_eq!(result.spheres, vec![vec![chest], vec![stump], vec![boss]]);
    assert!(result.unreachable_locations(&w).is_empty());
    assert!(result.all_goals_reached(&w));
}

#[test]
fn objet_enferme_derriere_lui_meme() {
    let (mut w, [chest, stump, _]) = monde_lineaire();
    // On échange : la clé dans le coffre, l'épée derrière l'épée.
    let sword = w.take(chest).unwrap().unwrap();
    let key = w.take(stump).unwrap().unwrap();
    w.place(chest, key).unwrap();
    w.place(stump, sword).unwrap();
    let result = sweep(&w, State::new(&w));
    assert_eq!(result.spheres, vec![vec![chest]]);
    assert_eq!(result.unreachable_locations(&w).len(), 2);
    assert!(!result.all_goals_reached(&w));
}

#[test]
fn inventaire_de_depart() {
    let (w, [chest, stump, boss]) = monde_lineaire();
    let p = w.slot_ids().next().unwrap();
    let mut start = State::new(&w);
    assert!(start.collect(&w, w.item_named(p, "Key").unwrap()));
    assert!(start.collect(&w, w.item_named(p, "Sword").unwrap()));
    let result = sweep(&w, start);
    assert_eq!(result.spheres, vec![vec![chest, stump, boss]]);
}

#[test]
fn seuls_les_objets_de_progression_comptent() {
    let mut w = MultiWorld::new();
    let p = w.add_slot("Alice", "Test").unwrap();
    let coin = w
        .add_item_kind(p, "Coin", Some(1), Classification::FILLER)
        .unwrap();
    let menu = w.add_region(p, "Menu").unwrap();
    let a = w.add_location(menu, "A", Some(1), Rule::True).unwrap();
    let b = w.add_location(menu, "B", Some(2), Rule::has(coin)).unwrap();
    w.place(a, coin).unwrap();
    let result = sweep(&w, State::new(&w));
    assert_eq!(result.spheres, vec![vec![a]]);
    assert!(!result.reach.location(b));
    assert_eq!(result.state.count(coin), 0);

    let mut state = State::new(&w);
    assert!(!state.collect(&w, coin));
    assert!(!state.remove(&w, coin));
}

#[test]
fn regles_composees() {
    let mut w = MultiWorld::new();
    let p = w.add_slot("Alice", "Test").unwrap();
    let red = w.add_item_kind(p, "Red", Some(1), PROG).unwrap();
    let blue = w.add_item_kind(p, "Blue", Some(2), PROG).unwrap();
    let gem = w.add_item_kind(p, "Gem", Some(3), PROG).unwrap();
    let keys = w.add_item_group(p, "Keys", vec![red, blue]).unwrap();
    let menu = w.add_region(p, "Menu").unwrap();
    let reach = |w: &MultiWorld, s: &State, rule: Rule| {
        let r = Reachability::compute(w, s);
        rule.eval(w, s, &r)
    };

    let mut s = State::new(&w);
    assert!(reach(&w, &s, Rule::And(vec![])));
    assert!(!reach(&w, &s, Rule::Or(vec![])));
    assert!(reach(&w, &s, Rule::HasAll(vec![])));
    assert!(!reach(&w, &s, Rule::HasAny(vec![])));
    assert!(reach(&w, &s, Rule::HasAllCounts(vec![])));
    assert!(reach(&w, &s, Rule::CanReachRegion(menu)));

    s.collect(&w, red);
    s.collect(&w, gem);
    s.collect(&w, gem);
    assert!(reach(&w, &s, Rule::HasAny(vec![red, blue])));
    assert!(!reach(&w, &s, Rule::HasAll(vec![red, blue])));
    assert!(reach(&w, &s, Rule::HasAllCounts(vec![(red, 1), (gem, 2)])));
    assert!(!reach(&w, &s, Rule::has_count(gem, 3)));
    assert!(reach(
        &w,
        &s,
        Rule::HasGroup {
            group: keys,
            count: 1
        }
    ));
    assert!(!reach(
        &w,
        &s,
        Rule::HasGroup {
            group: keys,
            count: 2
        }
    ));
    assert!(reach(
        &w,
        &s,
        Rule::Or(vec![
            Rule::False,
            Rule::And(vec![Rule::has(red), Rule::True])
        ])
    ));

    s.collect(&w, blue);
    assert!(reach(
        &w,
        &s,
        Rule::HasGroup {
            group: keys,
            count: 2
        }
    ));
    assert!(s.remove(&w, blue));
    assert!(!reach(
        &w,
        &s,
        Rule::HasGroup {
            group: keys,
            count: 2
        }
    ));
}

/// Une entrée dont la règle cite une région créée **après** elle (condition indirecte) :
/// il faut un deuxième tour du point fixe pour l'ouvrir.
#[test]
fn condition_indirecte_sur_une_region_plus_loin() {
    let mut w = MultiWorld::new();
    let p = w.add_slot("Alice", "Test").unwrap();
    let menu = w.add_region(p, "Menu").unwrap();
    let door = w.add_region(p, "Door").unwrap();
    let lever = w.add_region(p, "Lever").unwrap();
    // Créée en premier, mais ne s'ouvre qu'une fois « Lever » atteinte.
    w.add_entrance(menu, door, "Menu -> Door", Rule::CanReachRegion(lever))
        .unwrap();
    let to_lever = w
        .add_entrance(menu, lever, "Menu -> Lever", Rule::True)
        .unwrap();
    let prize = w.add_location(door, "Prize", Some(1), Rule::True).unwrap();
    let hint = w
        .add_location(menu, "Hint", Some(2), Rule::CanReachLocation(prize))
        .unwrap();
    let event = w
        .add_location(menu, "Event", None, Rule::CanReachEntrance(to_lever))
        .unwrap();
    w.validate().unwrap();

    let result = sweep(&w, State::new(&w));
    assert_eq!(result.spheres, vec![vec![prize, hint, event]]);
}

#[test]
fn objet_d_un_autre_joueur() {
    let mut w = MultiWorld::new();
    let a = w.add_slot("Alice", "Test").unwrap();
    let b = w.add_slot("Bob", "Autre").unwrap();
    let a_key = w.add_item_kind(a, "Key", Some(1), PROG).unwrap();
    let b_key = w.add_item_kind(b, "Key", Some(1), PROG).unwrap();
    let a_menu = w.add_region(a, "Menu").unwrap();
    let b_menu = w.add_region(b, "Menu").unwrap();
    let a_free = w.add_location(a_menu, "Free", Some(1), Rule::True).unwrap();
    let a_locked = w
        .add_location(a_menu, "Locked", Some(2), Rule::has(a_key))
        .unwrap();
    let b_free = w.add_location(b_menu, "Free", Some(1), Rule::True).unwrap();
    let b_locked = w
        .add_location(b_menu, "Locked", Some(2), Rule::has(b_key))
        .unwrap();
    // La clé de Bob chez Alice, celle d'Alice chez Bob (enfermée derrière la clé de Bob).
    w.place(a_free, b_key).unwrap();
    w.place(b_locked, a_key).unwrap();
    w.set_goal(a, Rule::has(a_key)).unwrap();
    w.set_goal(b, Rule::has(b_key)).unwrap();

    let result = sweep(&w, State::new(&w));
    assert_eq!(
        result.spheres,
        vec![vec![a_free, b_free], vec![b_locked], vec![a_locked]]
    );
    assert!(result.goal_reached(&w, a));
    assert!(result.goal_reached(&w, b));
    // Même nom d'objet chez deux joueurs : deux sortes distinctes.
    assert_ne!(a_key, b_key);
    assert_eq!(w.item_named(b, "Key"), Some(b_key));
}

#[test]
fn meme_resultat_deux_fois() {
    let (w, _) = monde_lineaire();
    assert_eq!(sweep(&w, State::new(&w)), sweep(&w, State::new(&w)));
}

#[test]
fn depart_par_defaut_et_change() {
    let mut w = MultiWorld::new();
    let p = w.add_slot("Alice", "Test").unwrap();
    let menu = w.add_region(p, "Menu").unwrap();
    let other = w.add_region(p, "Other").unwrap();
    assert_eq!(w.slot(p).unwrap().origin, Some(menu));
    let here = w.add_location(other, "Here", Some(1), Rule::True).unwrap();
    assert!(sweep(&w, State::new(&w)).spheres.is_empty());
    w.set_origin(p, other).unwrap();
    assert_eq!(sweep(&w, State::new(&w)).spheres, vec![vec![here]]);
}

#[test]
fn erreurs_de_construction() {
    let mut w = MultiWorld::new();
    let a = w.add_slot("Alice", "Test").unwrap();
    let b = w.add_slot("Bob", "Test").unwrap();
    let a_menu = w.add_region(a, "Menu").unwrap();
    let b_menu = w.add_region(b, "Menu").unwrap();
    let b_item = w.add_item_kind(b, "Thing", Some(1), PROG).unwrap();

    assert!(matches!(
        w.add_region(a, "Menu"),
        Err(LogicError::DuplicateName {
            kind: "région", ..
        })
    ));
    assert!(matches!(
        w.add_entrance(a_menu, b_menu, "Pont", Rule::True),
        Err(LogicError::CrossSlotEntrance { .. })
    ));
    assert!(matches!(
        w.add_item_group(a, "G", vec![b_item]),
        Err(LogicError::CrossSlotGroup { .. })
    ));

    let spot = w.add_location(a_menu, "Spot", Some(1), Rule::True).unwrap();
    assert!(matches!(
        w.add_location(a_menu, "Spot", Some(2), Rule::True),
        Err(LogicError::DuplicateName { kind: "lieu", .. })
    ));
    w.place(spot, b_item).unwrap();
    assert!(matches!(
        w.place(spot, b_item),
        Err(LogicError::LocationFilled { .. })
    ));
    assert_eq!(w.take(spot).unwrap(), Some(b_item));
    assert_eq!(w.take(spot).unwrap(), None);
}

#[test]
fn validation_d_un_identifiant_inconnu() {
    // Un identifiant d'un autre monde, plus grand que tout ce qu'a celui-ci.
    let mut big = MultiWorld::new();
    let p = big.add_slot("X", "Test").unwrap();
    let r0 = big.add_region(p, "R0").unwrap();
    let r1 = big.add_region(p, "R1").unwrap();
    let mut far_location = None;
    for i in 0..3 {
        far_location = Some(
            big.add_location(r1, format!("L{i}"), None, Rule::True)
                .unwrap(),
        );
    }
    let far_location = far_location.unwrap();
    let _ = r0;

    let mut w = MultiWorld::new();
    let a = w.add_slot("Alice", "Test").unwrap();
    let menu = w.add_region(a, "Menu").unwrap();
    w.add_location(
        menu,
        "Odd",
        Some(1),
        Rule::And(vec![Rule::True, Rule::CanReachLocation(far_location)]),
    )
    .unwrap();
    assert_eq!(
        w.validate(),
        Err(LogicError::UnknownLocation(far_location.raw()))
    );
    // Sans panique : la règle est simplement fausse.
    let result = sweep(&w, State::new(&w));
    assert!(result.spheres.is_empty());
    assert!(matches!(
        w.set_origin(a, r1),
        Err(LogicError::UnknownRegion(1))
    ));
}

#[test]
fn regles_d_objet_et_accessibilite() {
    use logic::{Accessibility, ItemRule, Progress};
    let mut w = MultiWorld::new();
    let a = w.add_slot("A", "Test").unwrap();
    let b = w.add_slot("B", "Test").unwrap();
    let key = w.add_item_kind(a, "Key", Some(1), PROG).unwrap();
    let boots = w
        .add_item_kind(a, "Boots", Some(2), Classification::USEFUL)
        .unwrap();
    let coin = w
        .add_item_kind(b, "Coin", Some(3), Classification::FILLER)
        .unwrap();
    let ra = w.add_region(a, "Menu").unwrap();
    let rb = w.add_region(b, "Menu").unwrap();
    let la = w.add_location(ra, "Chest", Some(10), Rule::True).unwrap();
    let lb = w.add_location(rb, "Chest", Some(20), Rule::True).unwrap();
    // Sans règle : tout passe.
    assert!(w.can_hold(la, key) && w.can_hold(la, coin));
    // Règle « refuse la clé », sauf qu'elle est aussi « toujours acceptée » : acceptée.
    let rule = w
        .add_item_rule(ItemRule::forbid(vec![key, boots], vec![key]))
        .unwrap();
    w.set_item_rule(la, Some(rule)).unwrap();
    assert!(w.can_hold(la, key));
    assert!(!w.can_hold(la, boots));
    assert!(w.can_hold(la, coin));
    // Lieu exclu : ni progression ni utile, sauf `always`.
    w.set_progress(lb, Progress::Excluded).unwrap();
    assert!(!w.can_hold(lb, key) && !w.can_hold(lb, boots) && w.can_hold(lb, coin));
    // Règle qui cite un objet inconnu, ou index inconnu : refusés.
    let mut other = MultiWorld::new();
    let o = other.add_slot("O", "Test").unwrap();
    let r = other.add_region(o, "Menu").unwrap();
    let l = other.add_location(r, "X", Some(1), Rule::True).unwrap();
    assert!(other
        .add_item_rule(ItemRule::forbid(vec![key], vec![]))
        .is_err());
    assert!(other.set_item_rule(l, Some(rule)).is_err());
    // Accessibilité : en `full`, tout lieu compte ; en `minimal`, seulement s'il garde la
    // progression d'un joueur qui n'est pas en `minimal`.
    assert!(w.must_reach(lb));
    w.set_accessibility(b, Accessibility::Minimal).unwrap();
    assert!(!w.must_reach(lb), "lieu vide d'un joueur minimal");
    w.place(lb, key).unwrap();
    assert!(w.must_reach(lb), "il garde la clé de A (full)");
    w.set_accessibility(a, Accessibility::Minimal).unwrap();
    assert!(
        !w.must_reach(lb),
        "la clé de A (minimal) peut rester hors d'atteinte"
    );
    assert!(w.validate().is_ok());
}

/// Somme pondérée (ADR 0055) : `2 * count(Gem) + count(Coin) >= 5`, et le nombre
/// d'objets différents (`unique`).
#[test]
fn somme_ponderee_de_compteurs() {
    use logic::SumTerm;
    let mut w = MultiWorld::new();
    let p = w.add_slot("Alice", "Test").unwrap();
    let gem = w.add_item_kind(p, "Gem", Some(1), PROG).unwrap();
    let coin = w.add_item_kind(p, "Coin", Some(2), PROG).unwrap();
    let menu = w.add_region(p, "Menu").unwrap();
    let a = w.add_location(menu, "A", Some(1), Rule::True).unwrap();
    w.place(a, gem).unwrap();
    let term = |item, weight, unique| SumTerm {
        item,
        weight,
        unique,
    };
    let weighted = Rule::Sum {
        terms: vec![term(gem, 2, false), term(coin, 1, false)],
        at_least: 5,
    };
    let distinct = Rule::Sum {
        terms: vec![term(gem, 1, true), term(coin, 1, true)],
        at_least: 2,
    };
    w.validate().unwrap();
    let eval = |w: &MultiWorld, s: &State, rule: &Rule| {
        let r = Reachability::compute(w, s);
        rule.eval(w, s, &r)
    };

    let mut s = State::new(&w);
    assert!(eval(
        &w,
        &s,
        &Rule::Sum {
            terms: vec![],
            at_least: 0
        }
    ));
    assert!(!eval(&w, &s, &weighted));
    s.collect(&w, gem);
    s.collect(&w, gem);
    assert!(!eval(&w, &s, &weighted), "2 * 2 + 0 = 4");
    assert!(!eval(&w, &s, &distinct), "une seule sorte d'objet");
    s.collect(&w, coin);
    assert!(eval(&w, &s, &weighted), "2 * 2 + 1 = 5");
    assert!(eval(&w, &s, &distinct));

    // Un objet inconnu dans un terme est refusé par la validation.
    let mut bad = MultiWorld::new();
    let q = bad.add_slot("Bob", "Test").unwrap();
    let r = bad.add_region(q, "Menu").unwrap();
    bad.add_location(
        r,
        "B",
        Some(1),
        Rule::Sum {
            terms: vec![term(gem, 1, false)],
            at_least: 1,
        },
    )
    .unwrap();
    assert!(matches!(bad.validate(), Err(LogicError::UnknownItem(_))));
}
