//! APQuest en Rust comparé aux seeds du vrai Archipelago 0.6.8 (ADR 0027, § 8).
//!
//! Les seeds de référence sont dans `tests/data` (voir le README) : on compare tout sauf
//! le placement, qui dépend du hasard de chaque moteur.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use generate::worlds::apquest::{ApQuestOptions, PlayerSprite};
use generate::{generate, GameOptions, GenerateError, GenerateRequest, PlayerRequest};
use multidata::{load_archipelago, ArchiveLimits, MultiData};

fn official(name: &str) -> MultiData {
    let path = format!(
        "{}/tests/data/apquest-{name}.archipelago",
        env!("CARGO_MANIFEST_DIR")
    );
    load_archipelago(&std::fs::read(path).unwrap(), &ArchiveLimits::default()).unwrap()
}

fn player(name: &str, options: ApQuestOptions) -> PlayerRequest {
    PlayerRequest::new(name, GameOptions::ApQuest(options))
}

fn tout() -> ApQuestOptions {
    ApQuestOptions {
        hard_mode: true,
        hammer: true,
        extra_starting_chest: true,
        start_with_one_confetti_cannon: true,
        trap_chance: 50,
        confetti_explosiveness: 10,
        player_sprite: PlayerSprite::Duck,
    }
}

fn mixte() -> ApQuestOptions {
    ApQuestOptions {
        hard_mode: true,
        trap_chance: 100,
        player_sprite: PlayerSprite::Cat,
        ..ApQuestOptions::default()
    }
}

/// Objets placés pour un slot : (objet, joueur, drapeaux) → nombre, les deux sortes de
/// remplissage (6 et 7) confondues si `merge_filler`.
fn pool(md: &MultiData, merge_filler: bool) -> BTreeMap<(i64, u32, u8), usize> {
    let mut counts = BTreeMap::new();
    for table in md.locations.values() {
        for placed in table.values() {
            let (item, flags) = if merge_filler && (placed.item == 6 || placed.item == 7) {
                (6, 0)
            } else {
                (placed.item, placed.flags.0)
            };
            *counts.entry((item, placed.player, flags)).or_insert(0) += 1;
        }
    }
    counts
}

/// Compare tout ce qui ne dépend pas du placement.
fn same_as_official(ours: &MultiData, theirs: &MultiData, merge_filler: bool) {
    assert_eq!(ours.datapackage, theirs.datapackage, "datapackage");
    assert_eq!(ours.slots, theirs.slots, "slot_info");
    assert_eq!(ours.connect_names, theirs.connect_names);
    assert_eq!(ours.slot_data, theirs.slot_data, "slot_data");
    assert_eq!(ours.precollected_items, theirs.precollected_items);
    assert_eq!(ours.precollected_hints, theirs.precollected_hints);
    assert_eq!(ours.er_hint_data, theirs.er_hint_data);
    assert_eq!(ours.generator_version, theirs.generator_version);
    assert_eq!(ours.minimum_server_version, theirs.minimum_server_version);
    assert_eq!(ours.minimum_client_versions, theirs.minimum_client_versions);
    assert_eq!(ours.tags, theirs.tags);
    assert_eq!(ours.race_mode, theirs.race_mode);
    assert_eq!(ours.server_options, theirs.server_options);
    // Mêmes lieux, slot par slot.
    let keys = |md: &MultiData| -> BTreeMap<u32, Vec<i64>> {
        md.locations
            .iter()
            .map(|(slot, table)| (*slot, table.keys().copied().collect()))
            .collect()
    };
    assert_eq!(keys(ours), keys(theirs), "lieux");
    assert_eq!(pool(ours, merge_filler), pool(theirs, merge_filler), "pool");
    // Chaque lieu envoyable apparaît dans une sphère et une seule.
    let mut in_spheres: Vec<(u32, i64)> = ours
        .spheres
        .iter()
        .flat_map(|s| s.iter().flat_map(|(slot, l)| l.iter().map(|l| (*slot, *l))))
        .collect();
    in_spheres.sort();
    let mut all: Vec<(u32, i64)> = keys(ours)
        .into_iter()
        .flat_map(|(slot, l)| l.into_iter().map(move |l| (slot, l)))
        .collect();
    all.sort();
    assert_eq!(in_spheres, all, "sphères");
}

fn run(players: Vec<PlayerRequest>, seed: u64) -> MultiData {
    let generated = generate(&GenerateRequest::new(seed, players)).unwrap();
    // L'archive écrite se relit et redonne le même modèle.
    let back = load_archipelago(&generated.archipelago, &ArchiveLimits::default()).unwrap();
    assert_eq!(back, generated.multidata);
    generated.multidata
}

#[test]
fn options_par_defaut_comme_l_officiel() {
    let ours = run(vec![player("Alice", ApQuestOptions::default())], 1);
    same_as_official(&ours, &official("defaut"), false);
}

#[test]
fn toutes_les_options_comme_l_officiel() {
    // trap_chance 50 : la part de pièges dépend du hasard, on confond les deux remplissages.
    let ours = run(vec![player("Alice", tout())], 1);
    same_as_official(&ours, &official("tout"), true);
}

#[test]
fn options_melangees_comme_l_officiel() {
    let ours = run(vec![player("Alice", mixte())], 1);
    same_as_official(&ours, &official("mixte"), false);
}

#[test]
fn deux_joueurs_comme_l_officiel() {
    let ours = run(
        vec![
            player("Alice", tout()),
            player("Bob", ApQuestOptions::default()),
        ],
        1,
    );
    same_as_official(&ours, &official("deux"), true);
}

#[test]
fn checksums_du_datapackage() {
    let md = official("defaut");
    assert_eq!(
        generate::game_package(&generate::worlds::apquest::PACKAGE),
        md.datapackage["APQuest"]
    );
    assert_eq!(
        generate::archipelago_package(),
        md.datapackage["Archipelago"]
    );
}

/// Toutes les combinaisons des options qui changent la logique, sur beaucoup de seeds :
/// toujours jouable, mêmes octets deux fois de suite.
#[test]
fn toutes_les_combinaisons_et_reproductible() {
    for bits in 0u8..16 {
        let options = ApQuestOptions {
            hard_mode: bits & 1 != 0,
            hammer: bits & 2 != 0,
            extra_starting_chest: bits & 4 != 0,
            start_with_one_confetti_cannon: bits & 8 != 0,
            trap_chance: 30,
            ..ApQuestOptions::default()
        };
        for seed in 0..60 {
            let request = GenerateRequest {
                seed,
                players: vec![player("A", options), player("B", tout())],
                time_balance: Default::default(),
            };
            // `generate` vérifie lui-même la faisabilité ; une erreur ferait échouer ici.
            let first = generate(&request).unwrap();
            let second = generate(&request).unwrap();
            assert_eq!(first.archipelago, second.archipelago, "seed {seed}");
        }
    }
}

#[test]
fn seeds_differentes_placements_differents() {
    let a = run(vec![player("Alice", tout())], 1);
    let b = run(vec![player("Alice", tout())], 2);
    assert_ne!(a.locations, b.locations);
    assert_eq!(a.seed_name, "00000000000000000001");
}

#[test]
fn noms_et_options_refuses() {
    let err = |players| generate(&GenerateRequest::new(0, players)).unwrap_err();
    assert!(matches!(err(vec![]), GenerateError::NoPlayers));
    assert!(matches!(
        err(vec![player("", ApQuestOptions::default())]),
        GenerateError::InvalidName { .. }
    ));
    assert!(matches!(
        err(vec![player(
            "Un nom bien trop long",
            ApQuestOptions::default()
        )]),
        GenerateError::InvalidName { .. }
    ));
    assert!(matches!(
        err(vec![
            player("Alice", ApQuestOptions::default()),
            player("alice", ApQuestOptions::default())
        ]),
        GenerateError::DuplicateName(_)
    ));
    let bad = ApQuestOptions {
        trap_chance: 101,
        ..ApQuestOptions::default()
    };
    assert!(matches!(
        err(vec![player("Alice", bad)]),
        GenerateError::InvalidOption {
            option: "trap_chance",
            ..
        }
    ));
}

/// L'équilibrage change le placement de certaines seeds, sans rien casser : toujours
/// jouable (vérifié par `generate`), mêmes octets deux fois de suite.
#[test]
fn equilibrage_de_progression() {
    let with = |value: u8| {
        let mut players = vec![
            player("A", tout()),
            player("B", tout()),
            player("C", tout()),
        ];
        for p in &mut players {
            p.progression_balancing = value;
        }
        players
    };
    let mut changed = false;
    for seed in 0..60 {
        let without = generate(&GenerateRequest {
            seed,
            players: with(0),
            time_balance: Default::default(),
        })
        .unwrap();
        let request = GenerateRequest {
            seed,
            players: with(99),
            time_balance: Default::default(),
        };
        let balanced = generate(&request).unwrap();
        assert_eq!(
            generate(&request).unwrap().archipelago,
            balanced.archipelago
        );
        changed |= without.multidata.locations != balanced.multidata.locations;
    }
    assert!(changed, "l'équilibrage n'a jamais rien changé");
}
