//! APQuest au format natif (ADR 0041) : identique au portage Rust.
//!
//! Le fichier `spec/monde-natif/exemples/apquest/world.toml` doit donner, pour toutes les
//! options, exactement le même monde que `generate::worlds::apquest`, donc la même seed
//! `.archipelago` à l'octet près. Si un de ces tests casse, le fichier natif et le portage
//! ne disent plus la même chose : c'est l'un des deux qu'il faut corriger, pas le test.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use generate::worlds::apquest::{ApQuestOptions, PlayerSprite};
use generate::{
    build, generate, GameOptions, GenerateRequest, NativeOptions, NativeWorld, PlayerRequest,
};
use sha2::{Digest, Sha256};

const APQUEST: &str = include_str!("../../../spec/monde-natif/exemples/apquest/world.toml");

/// Même empreinte que `tests/empreinte.rs` : la seed du portage.
const EXPECTED: &str = "cc6c6c409803ae75477b56b5fb02bebcf0dbc4070da94977db5f135988024085";

fn world() -> Arc<NativeWorld> {
    Arc::new(worlds_native::parse_world(APQUEST).unwrap())
}

/// Les mêmes options, au format natif.
fn native(world: &Arc<NativeWorld>, o: &ApQuestOptions) -> GameOptions {
    let mut n = NativeOptions::new(world.clone());
    n.set("hard_mode", i64::from(o.hard_mode)).unwrap();
    n.set("hammer", i64::from(o.hammer)).unwrap();
    n.set("extra_starting_chest", i64::from(o.extra_starting_chest))
        .unwrap();
    n.set("trap_chance", i64::from(o.trap_chance)).unwrap();
    n.set(
        "start_with_one_confetti_cannon",
        i64::from(o.start_with_one_confetti_cannon),
    )
    .unwrap();
    n.set(
        "confetti_explosiveness",
        i64::from(o.confetti_explosiveness),
    )
    .unwrap();
    n.set("player_sprite", o.player_sprite as i64).unwrap();
    GameOptions::Native(n)
}

/// Toutes les combinaisons des quatre options oui/non, avec quelques valeurs des autres.
fn all_options() -> Vec<ApQuestOptions> {
    let mut out = Vec::new();
    for bits in 0..16u8 {
        for (trap_chance, explosiveness, sprite) in [
            (0, 3, PlayerSprite::Human),
            (50, 0, PlayerSprite::Cat),
            (100, 10, PlayerSprite::Duck),
        ] {
            out.push(ApQuestOptions {
                hard_mode: bits & 1 != 0,
                hammer: bits & 2 != 0,
                extra_starting_chest: bits & 4 != 0,
                start_with_one_confetti_cannon: bits & 8 != 0,
                trap_chance,
                confetti_explosiveness: explosiveness,
                player_sprite: sprite,
            });
        }
    }
    out
}

/// Deux joueurs : le premier aux options par défaut, le second avec `o`.
fn request(seed: u64, first: GameOptions, second: GameOptions) -> GenerateRequest {
    GenerateRequest::new(
        seed,
        vec![
            PlayerRequest::new("Alice", first),
            PlayerRequest::new("Bob", second),
        ],
    )
}

#[test]
fn meme_monde_que_le_portage_pour_toutes_les_options() {
    let world = world();
    let default = ApQuestOptions::default();
    for (i, o) in all_options().iter().enumerate() {
        let seed = 1_000 + i as u64;
        let port = build(&request(
            seed,
            GameOptions::ApQuest(default),
            GameOptions::ApQuest(*o),
        ))
        .unwrap();
        let natif = build(&request(seed, native(&world, &default), native(&world, o))).unwrap();
        let (a, b) = (&port.world, &natif.world);
        assert_eq!(a.slots(), b.slots(), "joueurs, options {o:?}");
        assert_eq!(a.item_kinds(), b.item_kinds(), "objets, options {o:?}");
        assert_eq!(a.item_groups(), b.item_groups(), "groupes, options {o:?}");
        assert_eq!(a.regions(), b.regions(), "régions, options {o:?}");
        assert_eq!(a.entrances(), b.entrances(), "entrées, options {o:?}");
        assert_eq!(a.locations(), b.locations(), "lieux, options {o:?}");
        assert_eq!(port.pool, natif.pool, "pool, options {o:?}");
        assert_eq!(port.start_items, natif.start_items, "départ, options {o:?}");
        assert_eq!(port.slot_data, natif.slot_data, "slot_data, options {o:?}");
    }
}

#[test]
fn meme_seed_archipelago_que_le_portage() {
    let world = world();
    let default = ApQuestOptions::default();
    for (i, o) in all_options().iter().enumerate() {
        let seed = 77_000 + i as u64;
        let port = generate(&request(
            seed,
            GameOptions::ApQuest(default),
            GameOptions::ApQuest(*o),
        ))
        .unwrap();
        let natif = generate(&request(seed, native(&world, &default), native(&world, o))).unwrap();
        assert!(
            port.archipelago == natif.archipelago,
            "seed différente, options {o:?}"
        );
        assert_eq!(port.spoiler, natif.spoiler, "spoiler, options {o:?}");
    }
}

#[test]
fn empreinte_identique_au_portage() {
    let world = world();
    let hard = ApQuestOptions {
        hard_mode: true,
        hammer: true,
        trap_chance: 50,
        ..ApQuestOptions::default()
    };
    // La demande de `tests/empreinte.rs`, au format natif.
    let request = GenerateRequest::new(
        20_261_119,
        vec![
            PlayerRequest::new("Alice", native(&world, &ApQuestOptions::default())),
            PlayerRequest::new("Bob", native(&world, &hard)),
        ],
    );
    let generated = generate(&request).unwrap();
    let digest: String = Sha256::digest(&generated.archipelago)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(digest, EXPECTED);
}

#[test]
fn meme_datapackage_que_le_portage() {
    let world = world();
    let port = GameOptions::ApQuest(ApQuestOptions::default()).package();
    let natif = native(&world, &ApQuestOptions::default()).package();
    assert_eq!(port, natif);
}

#[test]
fn option_hors_bornes_refusee_par_generate() {
    let world = world();
    let mut n = NativeOptions::new(world);
    assert!(n.set("trap_chance", 101).is_err());
    assert!(n.set("inconnue", 1).is_err());
    n.set("trap_chance", 100).unwrap();
}

/// Les mêmes YAML (poids et `random` compris) donnent la même seed, lus pour le portage
/// ou pour le monde natif : les tirages des options sont faits dans le même ordre.
#[test]
fn memes_yaml_meme_seed() {
    let world = world();
    let yamls = [
        "name: Alice\ngame: APQuest\nAPQuest:\n  hard_mode: true\n  hammer: random\n  trap_chance: random\n  player_sprite: kitty\n",
        "name: Bob\ngame: APQuest\nAPQuest:\n  hard_mode: {true: 1, false: 3}\n  extra_starting_chest: on\n  confetti_explosiveness: random\n  player_sprite: random\n  progression_balancing: 20\n",
        "name: Carol\ngame: APQuest\nAPQuest:\n  start_with_one_confetti_cannon: yes\n  trap_chance: {10: 1, 90: 1}\n  player_sprite: 2\n  accessibility: full\n  local_items: []\n",
    ];
    let files: Vec<(String, String)> = yamls
        .iter()
        .enumerate()
        .map(|(i, t)| (format!("p{i}.yaml"), t.to_string()))
        .collect();
    for seed in [1u64, 2, 3, 99, 2026] {
        let port = generate::read_players(&files, seed).unwrap();
        let natif =
            generate::read_players_with_worlds(&files, None, seed, std::slice::from_ref(&world))
                .unwrap();
        assert!(natif
            .iter()
            .all(|p| matches!(p.options, GameOptions::Native(_))));
        let a = generate(&GenerateRequest::new(seed, port)).unwrap();
        let b = generate(&GenerateRequest::new(seed, natif)).unwrap();
        assert!(a.archipelago == b.archipelago, "seed {seed}");
        assert_eq!(a.spoiler, b.spoiler, "seed {seed}");
    }
}

#[test]
fn yaml_natif_refuse_les_options_inconnues() {
    let world = world();
    let files = vec![(
        "p.yaml".to_string(),
        "name: Alice\ngame: APQuest\nAPQuest:\n  hard_mod: true\n".to_string(),
    )];
    let e = generate::read_players_with_worlds(&files, None, 0, std::slice::from_ref(&world))
        .unwrap_err();
    assert!(e.0.contains("hard_mod"), "{e}");
    let files = vec![(
        "p.yaml".to_string(),
        "name: Alice\ngame: APQuest\nAPQuest:\n  trap_chance: 300\n".to_string(),
    )];
    let e = generate::read_players_with_worlds(&files, None, 0, &[world]).unwrap_err();
    assert!(e.0.contains("0 à 100"), "{e}");
}
