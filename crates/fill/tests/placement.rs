//! Tests de `fill` sur de petits mondes écrits à la main (ADR 0027, étape 3).

#![allow(clippy::unwrap_used)]

use fill::{below, check_feasible, fill, shuffle, FillError, Target};
use logic::{sweep, Classification, ItemKindId, LocationId, MultiWorld, Rule, State};
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;

const PROG: Classification = Classification::PROGRESSION;

fn rng(seed: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed)
}

/// `fill` avec un inventaire de départ vide. L'inventaire est créé avant l'appel : on ne
/// peut pas prêter `w` en lecture pendant qu'il est prêté en écriture à `fill`.
fn fill_zero(
    w: &mut MultiWorld,
    pool: Vec<ItemKindId>,
    rng: &mut ChaCha8Rng,
) -> Result<fill::FillReport, FillError> {
    let start = State::new(w);
    fill(w, pool, &start, rng)
}

/// Menu (Chest) → Forest [Sword] (Stump) → Castle [Key] (Boss, événement « Victory »).
/// Pool : Sword, Key. Victoire : avoir « Victory ».
fn monde_lineaire() -> (MultiWorld, Vec<ItemKindId>) {
    let mut w = MultiWorld::new();
    let p = w.add_slot("Alice", "Test").unwrap();
    let sword = w.add_item_kind(p, "Sword", Some(1), PROG).unwrap();
    let key = w.add_item_kind(p, "Key", Some(2), PROG).unwrap();
    let victory = w.add_item_kind(p, "Victory", None, PROG).unwrap();
    let menu = w.add_region(p, "Menu").unwrap();
    let forest = w.add_region(p, "Forest").unwrap();
    let castle = w.add_region(p, "Castle").unwrap();
    w.add_entrance(menu, forest, "Menu -> Forest", Rule::has(sword))
        .unwrap();
    w.add_entrance(forest, castle, "Forest -> Castle", Rule::has(key))
        .unwrap();
    w.add_location(menu, "Chest", Some(10), Rule::True).unwrap();
    w.add_location(forest, "Stump", Some(11), Rule::True)
        .unwrap();
    let boss = w.add_location(castle, "Boss", None, Rule::True).unwrap();
    w.place(boss, victory).unwrap();
    w.set_goal(p, Rule::has(victory)).unwrap();
    (w, vec![sword, key])
}

fn item_at(w: &MultiWorld, slot: u32, name: &str) -> String {
    let slot = w.slot_ids().nth(slot as usize).unwrap();
    let id = w.location_named(slot, name).unwrap();
    let item = w.location(id).unwrap().item.unwrap();
    w.item_kind(item).unwrap().name.clone()
}

#[test]
fn tirages_bornes_et_reproductibles() {
    let mut a = rng(1);
    for n in [1u64, 2, 3, 7, 1000, u64::MAX] {
        for _ in 0..100 {
            assert!(below(&mut a, n) < n);
        }
    }
    assert_eq!(below(&mut a, 0), 0);

    let mut x: Vec<u32> = (0..50).collect();
    let mut y = x.clone();
    shuffle(&mut rng(9), &mut x);
    shuffle(&mut rng(9), &mut y);
    assert_eq!(x, y);
    assert_ne!(x, (0..50).collect::<Vec<_>>());
    let mut sorted = x.clone();
    sorted.sort();
    assert_eq!(sorted, (0..50).collect::<Vec<_>>());
}

#[test]
fn monde_lineaire_toujours_jouable() {
    for seed in 0..200 {
        let (mut w, pool) = monde_lineaire();
        let report = fill_zero(&mut w, pool, &mut rng(seed)).unwrap();
        // Seul le coffre est atteignable au départ : l'épée y est forcément.
        assert_eq!(item_at(&w, 0, "Chest"), "Sword");
        assert_eq!(item_at(&w, 0, "Stump"), "Key");
        assert_eq!(report.spheres.len(), 3);
    }
}

#[test]
fn inventaire_de_depart_pris_en_compte() {
    // Avec l'épée au départ, la clé peut aussi tomber dans le coffre ; le pool n'a plus
    // l'épée mais une pièce.
    let (mut w, _) = monde_lineaire();
    let p = w.slot_ids().next().unwrap();
    let coin = w
        .add_item_kind(p, "Coin", Some(3), Classification::FILLER)
        .unwrap();
    let key = w.item_named(p, "Key").unwrap();
    let mut start = State::new(&w);
    start.collect(&w, w.item_named(p, "Sword").unwrap());
    let mut key_in_chest = false;
    for seed in 0..50 {
        let mut w = w.clone();
        fill(&mut w, vec![key, coin], &start, &mut rng(seed)).unwrap();
        key_in_chest |= item_at(&w, 0, "Chest") == "Key";
    }
    assert!(key_in_chest);
}

#[test]
fn options_impossibles_expliquees() {
    let (mut w, pool) = monde_lineaire();
    let p = w.slot_ids().next().unwrap();
    let gem = w.add_item_kind(p, "Gem", Some(4), PROG).unwrap();
    let castle = w.region_named(p, "Castle").unwrap();
    w.add_location(castle, "Vault", Some(12), Rule::has_count(gem, 2))
        .unwrap();
    let mut pool = pool;
    pool.push(gem); // un seul exemplaire, il en faut deux
    let err = check_feasible(&w, &pool, &State::new(&w)).unwrap_err();
    let FillError::Impossible { blocked } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].target, Target::Location("Vault".into()));
    assert_eq!(blocked[0].clause, "Has(\"Gem\", 2)");
    assert!(err.to_string().contains("Alice : lieu « Vault »"));
    // `fill` refuse aussi, sans rien placer.
    let before = w.clone();
    assert_eq!(fill_zero(&mut w, pool, &mut rng(0)).unwrap_err(), err);
    assert_eq!(w.locations(), before.locations());
}

#[test]
fn region_bloquee_par_une_entree() {
    let (w, _) = monde_lineaire();
    let p = w.slot_ids().next().unwrap();
    // Sans la clé dans le pool : le château et l'objectif sont bloqués.
    let sword = w.item_named(p, "Sword").unwrap();
    let err = check_feasible(&w, &[sword], &State::new(&w)).unwrap_err();
    let FillError::Impossible { blocked } = err else {
        panic!()
    };
    assert_eq!(blocked.len(), 2);
    assert_eq!(blocked[0].target, Target::Location("Boss".into()));
    assert_eq!(
        blocked[0].clause,
        "région « Castle » : entrée « Forest -> Castle » : Has(\"Key\")"
    );
    assert_eq!(blocked[1].target, Target::Goal);
    assert_eq!(blocked[1].clause, "Has(\"Victory\")");
}

#[test]
fn premiere_clause_fausse_d_un_et() {
    let mut w = MultiWorld::new();
    let p = w.add_slot("Bob", "Test").unwrap();
    let a = w.add_item_kind(p, "A", Some(1), PROG).unwrap();
    let b = w.add_item_kind(p, "B", Some(2), PROG).unwrap();
    let menu = w.add_region(p, "Menu").unwrap();
    let rule = Rule::And(vec![
        Rule::has(a),
        Rule::Or(vec![Rule::has(b), Rule::has_count(a, 3)]),
    ]);
    w.add_location(menu, "L", Some(1), rule).unwrap();
    let err = check_feasible(&w, &[a], &State::new(&w)).unwrap_err();
    let FillError::Impossible { blocked } = err else {
        panic!()
    };
    assert_eq!(blocked[0].clause, "(Has(\"B\") | Has(\"A\", 3))");
}

#[test]
fn nombre_d_objets_et_de_lieux_differents() {
    let (mut w, mut pool) = monde_lineaire();
    pool.pop();
    let err = fill_zero(&mut w, pool, &mut rng(0)).unwrap_err();
    assert_eq!(
        err,
        FillError::CountMismatch {
            items: 1,
            locations: 2
        }
    );
}

#[test]
fn aucun_lieu_libre_et_monde_remis_en_etat() {
    // Les deux lieux demandent l'épée : faisable « avec tout en main », mais l'épée n'a
    // nulle part où aller.
    let mut w = MultiWorld::new();
    let p = w.add_slot("Alice", "Test").unwrap();
    let sword = w.add_item_kind(p, "Sword", Some(1), PROG).unwrap();
    let coin = w
        .add_item_kind(p, "Coin", Some(2), Classification::FILLER)
        .unwrap();
    let menu = w.add_region(p, "Menu").unwrap();
    w.add_location(menu, "A", Some(1), Rule::has(sword))
        .unwrap();
    w.add_location(menu, "B", Some(2), Rule::has(sword))
        .unwrap();
    let before = w.clone();
    let err = fill_zero(&mut w, vec![coin, sword], &mut rng(3)).unwrap_err();
    assert_eq!(
        err,
        FillError::NoReachableLocation {
            slot: "Alice".into(),
            item: "Sword".into()
        }
    );
    assert_eq!(w.locations(), before.locations());
}

/// Monde plus gros, pour deux joueurs : chaîne de `regions` régions, chacune derrière une
/// clé ; 4 lieux par région ; une porte qui demande 2 gemmes ; utiles et remplissage pour
/// compléter le pool.
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
        let trap = w
            .add_item_kind(p, "Trap", Some(103), Classification::TRAP)
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
        pool.extend([gem, gem, boots, trap]);
        let missing = locations - (regions + 4);
        pool.extend(std::iter::repeat_n(rupee, missing as usize));
    }
    (w, pool)
}

fn placement(w: &MultiWorld) -> Vec<Option<ItemKindId>> {
    w.locations().iter().map(|l| l.item).collect()
}

#[test]
fn proprietes_sur_mille_seeds() {
    let (base, pool) = gros_monde(2, 6);
    let mut sorted_pool = pool.clone();
    sorted_pool.sort();
    let mut different = false;
    let mut first: Option<Vec<Option<ItemKindId>>> = None;
    for seed in 0..1000 {
        let mut w = base.clone();
        fill_zero(&mut w, pool.clone(), &mut rng(seed)).unwrap();
        // Chaque lieu rempli, chaque objet du pool placé une fois et une seule.
        let mut placed: Vec<ItemKindId> = w
            .locations()
            .iter()
            .filter(|l| l.address.is_some())
            .map(|l| l.item.unwrap())
            .collect();
        placed.sort();
        assert_eq!(placed, sorted_pool, "seed {seed}");
        // Jouable depuis le départ.
        let result = sweep(&w, State::new(&w));
        assert!(result.unreachable_locations(&w).is_empty(), "seed {seed}");
        assert!(result.all_goals_reached(&w), "seed {seed}");
        // Même seed ⇒ même placement.
        let mut again = base.clone();
        fill_zero(&mut again, pool.clone(), &mut rng(seed)).unwrap();
        assert_eq!(placement(&w), placement(&again), "seed {seed}");
        match &first {
            None => first = Some(placement(&w)),
            Some(p) => different |= *p != placement(&w),
        }
    }
    assert!(different, "toutes les seeds donnent le même placement");
}

#[test]
fn objets_d_un_joueur_chez_l_autre() {
    let (base, pool) = gros_monde(2, 3);
    let mut crossed = false;
    for seed in 0..50 {
        let mut w = base.clone();
        fill_zero(&mut w, pool.clone(), &mut rng(seed)).unwrap();
        crossed |= w.locations().iter().any(|l| {
            let item = l.item.and_then(|i| w.item_kind(i)).unwrap();
            item.slot != l.slot
        });
    }
    assert!(crossed);
}

#[test]
fn lieux_sans_objet_cites_dans_le_rapport() {
    // Garde-fou : `LocationId` reste utilisable par l'appelant à partir du rapport.
    let (mut w, pool) = monde_lineaire();
    let report = fill_zero(&mut w, pool, &mut rng(0)).unwrap();
    let first: &LocationId = &report.spheres[0][0];
    assert_eq!(w.location(*first).unwrap().name, "Chest");
}
