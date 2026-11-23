//! Tests de l'équilibrage de progression (ADR 0027, étape 5).

#![allow(clippy::unwrap_used)]

use fill::{balance, fill, BalanceReport};
use logic::{sweep, Classification, ItemKindId, LocationId, MultiWorld, Rule, State};
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;

const PROG: Classification = Classification::PROGRESSION;

fn rng(seed: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed)
}

fn placement(w: &MultiWorld) -> Vec<Option<ItemKindId>> {
    w.locations().iter().map(|l| l.item).collect()
}

/// Numéro de la sphère (depuis le départ) où se trouve le lieu.
fn sphere_of(w: &MultiWorld, location: LocationId) -> usize {
    let result = sweep(w, State::new(w));
    result
        .spheres
        .iter()
        .position(|s| s.contains(&location))
        .unwrap()
}

/// Deux joueurs, placement écrit à la main pour que Bob soit bloqué :
///
/// - Alice : 10 lieux ouverts (A1..A10), une porte « Key A » vers 10 lieux (D1..D10) ;
/// - Bob : 1 lieu ouvert (S), une porte « Key B » vers 19 lieux (E1..E19) ;
/// - « Key A » est en A1 (sphère 0) ; « Key B », la clé de Bob, est au fond du monde
///   d'Alice, en D10 (sphère 1). Tout le reste est du remplissage.
///
/// Sans équilibrage, Bob ne peut visiter qu'un lieu sur 20 tant qu'Alice n'a pas fini.
/// Renvoie le monde et le lieu D10.
fn bob_bloque() -> (MultiWorld, LocationId) {
    bob_bloque_avec(PROG)
}

/// Comme [`bob_bloque`], avec la classification de « Key B » au choix.
fn bob_bloque_avec(key_b_class: Classification) -> (MultiWorld, LocationId) {
    let mut w = MultiWorld::new();
    let alice = w.add_slot("Alice", "Test").unwrap();
    let bob = w.add_slot("Bob", "Test").unwrap();
    let key_a = w.add_item_kind(alice, "Key A", Some(1), PROG).unwrap();
    let rupee_a = w
        .add_item_kind(alice, "Rupee", Some(2), Classification::FILLER)
        .unwrap();
    let win_a = w.add_item_kind(alice, "Victory", None, PROG).unwrap();
    let key_b = w.add_item_kind(bob, "Key B", Some(1), key_b_class).unwrap();
    let rupee_b = w
        .add_item_kind(bob, "Rupee", Some(2), Classification::FILLER)
        .unwrap();
    let win_b = w.add_item_kind(bob, "Victory", None, PROG).unwrap();

    let menu_a = w.add_region(alice, "Menu").unwrap();
    let deep_a = w.add_region(alice, "Deep").unwrap();
    w.add_entrance(menu_a, deep_a, "Gate", Rule::has(key_a))
        .unwrap();
    let mut deep_last = None;
    for i in 1..=10 {
        let l = w
            .add_location(menu_a, format!("A{i}"), Some(i), Rule::True)
            .unwrap();
        w.place(l, if i == 1 { key_a } else { rupee_a }).unwrap();
    }
    for i in 1..=10 {
        let l = w
            .add_location(deep_a, format!("D{i}"), Some(100 + i), Rule::True)
            .unwrap();
        w.place(l, if i == 10 { key_b } else { rupee_a }).unwrap();
        deep_last = Some(l);
    }
    let end_a = w.add_location(deep_a, "End", None, Rule::True).unwrap();
    w.place(end_a, win_a).unwrap();
    w.set_goal(alice, Rule::has(win_a)).unwrap();

    let menu_b = w.add_region(bob, "Menu").unwrap();
    let deep_b = w.add_region(bob, "Deep").unwrap();
    w.add_entrance(menu_b, deep_b, "Gate", Rule::has(key_b))
        .unwrap();
    let s = w.add_location(menu_b, "S", Some(1), Rule::True).unwrap();
    w.place(s, rupee_b).unwrap();
    for i in 1..=19 {
        let l = w
            .add_location(deep_b, format!("E{i}"), Some(100 + i), Rule::True)
            .unwrap();
        w.place(l, rupee_b).unwrap();
    }
    let end_b = w.add_location(deep_b, "End", None, Rule::True).unwrap();
    w.place(end_b, win_b).unwrap();
    w.set_goal(bob, Rule::has(win_b)).unwrap();
    w.validate().unwrap();
    (w, deep_last.unwrap())
}

/// Lieu où se trouve l'objet nommé du joueur `slot`.
fn where_is(w: &MultiWorld, slot: u32, item: &str) -> LocationId {
    let slot = w.slot_ids().nth(slot as usize).unwrap();
    let kind = w.item_named(slot, item).unwrap();
    w.location_ids()
        .find(|id| w.location(*id).unwrap().item == Some(kind))
        .unwrap()
}

#[test]
fn la_cle_de_bob_avance() {
    let (mut w, d10) = bob_bloque();
    assert_eq!(sphere_of(&w, d10), 1);
    let start = State::new(&w);
    let report = balance(&mut w, &[0, 50], &start, &mut rng(1)).unwrap();
    assert_eq!(
        report,
        BalanceReport {
            moved: 1,
            not_moved: 0
        }
    );
    // La clé de Bob est maintenant dans un lieu de la sphère 0, et D10 reçoit à la place
    // l'objet sans importance qui s'y trouvait.
    let key_b = where_is(&w, 1, "Key B");
    assert_eq!(sphere_of(&w, key_b), 0);
    assert_ne!(key_b, d10);
    let result = sweep(&w, State::new(&w));
    assert!(result.unreachable_locations(&w).is_empty());
    assert!(result.all_goals_reached(&w));
}

#[test]
fn sans_reglage_rien_ne_bouge() {
    let (mut w, _) = bob_bloque();
    let before = placement(&w);
    let start = State::new(&w);
    let report = balance(&mut w, &[0, 0], &start, &mut rng(1)).unwrap();
    assert_eq!(report, BalanceReport::default());
    assert_eq!(placement(&w), before);
    // Un slot absent de la liste vaut 0.
    let report = balance(&mut w, &[], &start, &mut rng(1)).unwrap();
    assert_eq!(report, BalanceReport::default());
    assert_eq!(placement(&w), before);
}

#[test]
fn seul_le_joueur_en_retard_est_aide() {
    // Alice a l'équilibrage, pas Bob : Alice n'est jamais en retard, rien ne bouge.
    let (mut w, _) = bob_bloque();
    let before = placement(&w);
    let start = State::new(&w);
    let report = balance(&mut w, &[99, 0], &start, &mut rng(1)).unwrap();
    assert_eq!(report.moved, 0);
    assert_eq!(placement(&w), before);
}

#[test]
fn objet_marque_sans_equilibrage_laisse_en_place() {
    // Même monde, mais « Key B » est marquée `skip_balancing` : elle ne bouge pas.
    let (mut w, d10) = bob_bloque_avec(Classification {
        skip_balancing: true,
        ..PROG
    });
    let start = State::new(&w);
    let report = balance(&mut w, &[0, 99], &start, &mut rng(1)).unwrap();
    assert_eq!(report.moved, 0);
    assert_eq!(where_is(&w, 1, "Key B"), d10);
}

/// Monde de `fill` pour deux joueurs : chaîne de `regions` régions derrière des clés, 4
/// lieux par région, un coffre derrière 2 gemmes.
fn gros_monde(players: u32, regions: u32) -> (MultiWorld, Vec<ItemKindId>) {
    let mut w = MultiWorld::new();
    let mut pool = Vec::new();
    for n in 0..players {
        let p = w.add_slot(format!("P{n}"), "Test").unwrap();
        let gem = w.add_item_kind(p, "Gem", Some(100), PROG).unwrap();
        let boots = w
            .add_item_kind(p, "Boots", Some(101), Classification::USEFUL)
            .unwrap();
        let rupee = w
            .add_item_kind(p, "Rupee", Some(102), Classification::FILLER)
            .unwrap();
        let victory = w.add_item_kind(p, "Victory", None, PROG).unwrap();
        let mut prev = w.add_region(p, "R0").unwrap();
        let mut locations = 0;
        for r in 0..regions {
            for l in 0..4 {
                w.add_location(
                    prev,
                    format!("R{r} L{l}"),
                    Some(i64::from(r * 10 + l)),
                    Rule::True,
                )
                .unwrap();
                locations += 1;
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
        let vault = w.add_region(p, "Vault").unwrap();
        w.add_entrance(prev, vault, "Vault door", Rule::has_count(gem, 2))
            .unwrap();
        w.add_location(vault, "Treasure", Some(999), Rule::True)
            .unwrap();
        let end = w.add_location(vault, "End", None, Rule::True).unwrap();
        w.place(end, victory).unwrap();
        w.set_goal(p, Rule::has(victory)).unwrap();
        locations += 1;
        pool.extend([gem, gem, boots]);
        let missing = locations - (regions + 3);
        pool.extend(std::iter::repeat_n(rupee, missing as usize));
    }
    (w, pool)
}

#[test]
fn proprietes_sur_deux_cents_seeds() {
    let (base, pool) = gros_monde(3, 6);
    let mut moved_somewhere = false;
    for seed in 0..200 {
        let mut w = base.clone();
        let start = State::new(&w);
        let mut r = rng(seed);
        fill(&mut w, pool.clone(), &start, &mut r).unwrap();
        let mut before: Vec<ItemKindId> = w.locations().iter().map(|l| l.item.unwrap()).collect();
        let report = balance(&mut w, &[99, 50, 99], &start, &mut r).unwrap();
        moved_somewhere |= report.moved > 0;
        // Le pool ne change pas : les objets échangent seulement leur place.
        let mut after: Vec<ItemKindId> = w.locations().iter().map(|l| l.item.unwrap()).collect();
        before.sort();
        after.sort();
        assert_eq!(before, after, "seed {seed}");
        // Les événements (lieux sans numéro) ne bougent pas.
        for l in w.locations().iter().filter(|l| l.address.is_none()) {
            assert_eq!(w.item_kind(l.item.unwrap()).unwrap().name, "Victory");
        }
        // Toujours jouable.
        let result = sweep(&w, State::new(&w));
        assert!(result.unreachable_locations(&w).is_empty(), "seed {seed}");
        assert!(result.all_goals_reached(&w), "seed {seed}");
        // Reproductible.
        let mut again = base.clone();
        let mut r = rng(seed);
        fill(&mut again, pool.clone(), &start, &mut r).unwrap();
        balance(&mut again, &[99, 50, 99], &start, &mut r).unwrap();
        assert_eq!(placement(&w), placement(&again), "seed {seed}");
    }
    assert!(moved_somewhere, "l'équilibrage n'a jamais rien avancé");
}

/// Sphère à partir de laquelle chaque joueur peut visiter au moins la moitié de ses lieux ;
/// on renvoie la plus tardive (le joueur le plus en retard).
fn worst_half_sphere(w: &MultiWorld) -> usize {
    let result = sweep(w, State::new(w));
    let mut worst = 0;
    for slot in w.slot_ids() {
        let total = w
            .locations()
            .iter()
            .filter(|l| l.slot == slot && l.address.is_some())
            .count();
        let mut seen = 0;
        for (i, sphere) in result.spheres.iter().enumerate() {
            seen += sphere
                .iter()
                .filter(|id| {
                    let l = w.location(**id).unwrap();
                    l.slot == slot && l.address.is_some()
                })
                .count();
            if seen * 2 >= total {
                worst = worst.max(i);
                break;
            }
        }
    }
    worst
}

#[test]
fn le_joueur_le_plus_en_retard_avance_plus_tot() {
    // Mesure d'effet : en moyenne sur 200 seeds, le joueur le plus en retard atteint la
    // moitié de ses lieux plus tôt avec l'équilibrage qu'avec le placement seul.
    let (base, pool) = gros_monde(3, 6);
    let (mut without, mut with) = (0usize, 0usize);
    for seed in 0..200 {
        let mut w = base.clone();
        let start = State::new(&w);
        let mut r = rng(seed);
        fill(&mut w, pool.clone(), &start, &mut r).unwrap();
        without += worst_half_sphere(&w);
        balance(&mut w, &[99, 99, 99], &start, &mut r).unwrap();
        with += worst_half_sphere(&w);
    }
    assert!(with < without, "avec {with}, sans {without}");
}

#[test]
fn objet_pose_d_avance_laisse_en_place() {
    // Bogue corrigé par l'ADR 0054 : « Key B » posée d'avance par le monde (lieu à numéro,
    // verrouillé) était échangée comme un objet placé, parce que seul un lieu sans
    // numéro comptait comme verrouillé.
    let (mut w, d10) = bob_bloque();
    let key_b = w.take(d10).unwrap().unwrap();
    w.place_locked(d10, key_b).unwrap();
    assert!(w.is_locked(d10));
    let start = State::new(&w);
    let report = balance(&mut w, &[0, 99], &start, &mut rng(1)).unwrap();
    assert_eq!(report.moved, 0);
    assert_eq!(where_is(&w, 1, "Key B"), d10);
    assert!(w.is_locked(d10));
}
