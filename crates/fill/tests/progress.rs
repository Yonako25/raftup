//! Lieux prioritaires et exclus (`LocationProgressType`, ADR 0028, étape 3).

#![allow(clippy::unwrap_used)]

use fill::{balance, fill, FillError};
use logic::{sweep, Classification, ItemKindId, MultiWorld, Progress, Rule, State};
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;

const PROG: Classification = Classification::PROGRESSION;

fn rng(seed: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed)
}

/// Chaîne de 5 régions derrière des clés, 4 lieux par région, par joueur. Par région `r` :
/// le lieu `L1` est exclu si `r` est pair, le lieu `L2` est prioritaire si `r` est impair.
/// Pool : les 5 clés, 2 utiles, le reste en remplissage.
fn monde(players: u32) -> (MultiWorld, Vec<ItemKindId>) {
    let mut w = MultiWorld::new();
    let mut pool = Vec::new();
    for n in 0..players {
        let p = w.add_slot(format!("P{n}"), "Test").unwrap();
        let boots = w
            .add_item_kind(p, "Boots", Some(100), Classification::USEFUL)
            .unwrap();
        let rupee = w
            .add_item_kind(p, "Rupee", Some(101), Classification::FILLER)
            .unwrap();
        let victory = w.add_item_kind(p, "Victory", None, PROG).unwrap();
        let mut prev = w.add_region(p, "R0").unwrap();
        for r in 0..5u32 {
            for l in 0..4u32 {
                let id = w
                    .add_location(
                        prev,
                        format!("R{r} L{l}"),
                        Some(i64::from(r * 10 + l)),
                        Rule::True,
                    )
                    .unwrap();
                if l == 1 && r % 2 == 0 {
                    w.set_progress(id, Progress::Excluded).unwrap();
                }
                if l == 2 && r % 2 == 1 {
                    w.set_progress(id, Progress::Priority).unwrap();
                }
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
        pool.extend([boots, boots]);
        pool.extend(std::iter::repeat_n(rupee, 20 - 5 - 2));
    }
    (w, pool)
}

/// Aucun lieu exclu ne contient de progression ni d'objet utile.
fn excluded_clean(w: &MultiWorld) -> bool {
    w.locations()
        .iter()
        .filter(|l| l.progress == Progress::Excluded)
        .all(|l| {
            let class = w.item_kind(l.item.unwrap()).unwrap().classification;
            !class.progression && !class.useful
        })
}

#[test]
fn exclus_sans_progression_ni_utile() {
    let (base, pool) = monde(2);
    for seed in 0..300 {
        let mut w = base.clone();
        let start = State::new(&w);
        let mut r = rng(seed);
        fill(&mut w, pool.clone(), &start, &mut r).unwrap();
        assert!(excluded_clean(&w), "seed {seed} après placement");
        let before: Vec<_> = w
            .locations()
            .iter()
            .filter(|l| l.progress == Progress::Priority)
            .map(|l| l.item)
            .collect();
        balance(&mut w, &[99, 99], &start, &mut r).unwrap();
        assert!(excluded_clean(&w), "seed {seed} après équilibrage");
        // L'équilibrage ne déplace pas les objets des lieux prioritaires.
        let after: Vec<_> = w
            .locations()
            .iter()
            .filter(|l| l.progress == Progress::Priority)
            .map(|l| l.item)
            .collect();
        assert_eq!(before, after, "seed {seed}");
        let result = sweep(&w, State::new(&w));
        assert!(result.unreachable_locations(&w).is_empty(), "seed {seed}");
        assert!(result.all_goals_reached(&w), "seed {seed}");
    }
}

#[test]
fn prioritaires_recoivent_la_progression() {
    // 2 lieux prioritaires et 5 clés par joueur : les prioritaires reçoivent de la
    // progression dès qu'ils sont atteignables au moment du placement, ce qui arrive le
    // plus souvent. Sans la règle, 1 lieu sur 4 environ en recevrait (10 clés, 40 lieux).
    let (base, pool) = monde(2);
    let (mut with_progression, mut total) = (0, 0);
    for seed in 0..300 {
        let mut w = base.clone();
        let start = State::new(&w);
        fill(&mut w, pool.clone(), &start, &mut rng(seed)).unwrap();
        for l in w
            .locations()
            .iter()
            .filter(|l| l.progress == Progress::Priority)
        {
            total += 1;
            if w.item_kind(l.item.unwrap())
                .unwrap()
                .classification
                .progression
            {
                with_progression += 1;
            }
        }
    }
    assert!(
        with_progression * 10 >= total * 9,
        "{with_progression} prioritaires avec progression sur {total}"
    );
}

#[test]
fn trop_de_lieux_exclus() {
    let (mut w, mut pool) = monde(1);
    // On remplace le remplissage par des objets utiles : plus assez pour les 3 exclus.
    let slot = w.slot_ids().next().unwrap();
    let boots = w.item_named(slot, "Boots").unwrap();
    let rupee = w.item_named(slot, "Rupee").unwrap();
    let mut kept = 0;
    for item in &mut pool {
        if *item == rupee {
            if kept < 2 {
                kept += 1;
            } else {
                *item = boots;
            }
        }
    }
    let start = State::new(&w);
    let err = fill(&mut w, pool, &start, &mut rng(1)).unwrap_err();
    assert_eq!(
        err,
        FillError::TooManyExcluded {
            excluded: 3,
            filler: 2
        }
    );
}

#[test]
fn reproductible() {
    let (base, pool) = monde(2);
    let run = |seed| {
        let mut w = base.clone();
        let start = State::new(&w);
        let mut r = rng(seed);
        fill(&mut w, pool.clone(), &start, &mut r).unwrap();
        balance(&mut w, &[50, 50], &start, &mut r).unwrap();
        w.locations().iter().map(|l| l.item).collect::<Vec<_>>()
    };
    for seed in 0..20 {
        assert_eq!(run(seed), run(seed));
    }
}
