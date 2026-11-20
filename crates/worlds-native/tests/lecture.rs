//! Lecture des mondes valides et compilation vers `logic`.

#![allow(clippy::unwrap_used)]

use logic::{Classification, MultiWorld, Progress, Rule};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use worlds_native::model::{Count, Expr, OptionKind};
use worlds_native::{parse_world, Built, NativeWorld};

const MINIMAL: &str = include_str!("../../../spec/monde-natif/exemples/minimal/world.toml");
const APQUEST: &str = include_str!("../../../spec/monde-natif/exemples/apquest/world.toml");

/// Construit le monde pour un joueur avec des options (nom, valeur).
fn build(world: &NativeWorld, options: &[(&str, i64)]) -> (MultiWorld, Built) {
    let mut values = world.default_values();
    for (name, value) in options {
        world.set_option(&mut values, name, *value).unwrap();
    }
    let mut w = MultiWorld::new();
    let slot = w.add_slot("Alice", world.game.clone()).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(1);
    let built = world.build(&values, &mut w, slot, &mut rng).unwrap();
    (w, built)
}

fn rule_of(w: &MultiWorld, location: &str) -> Rule {
    w.locations()
        .iter()
        .find(|l| l.name == location)
        .unwrap()
        .rule
        .clone()
}

#[test]
fn exemple_minimal() {
    let world = parse_world(MINIMAL).unwrap();
    assert_eq!(world.game, "Minimal");
    let (w, built) = build(&world, &[]);
    assert_eq!(w.regions().len(), 2);
    assert_eq!(w.locations().len(), 2);
    // Une clé, puis une pièce pour compléter jusqu'aux deux lieux.
    let names: Vec<&str> = built
        .pool
        .iter()
        .map(|id| w.item_kind(*id).unwrap().name.as_str())
        .collect();
    assert_eq!(names, ["Key", "Coin"]);
    let treasure = w
        .locations()
        .iter()
        .position(|l| l.name == "Treasure")
        .unwrap();
    assert!(matches!(w.slots()[0].goal, Rule::CanReachLocation(l) if l.index() == treasure));
}

#[test]
fn exemple_apquest_lu() {
    let world = parse_world(APQUEST).unwrap();
    assert_eq!(world.options.len(), 7);
    assert_eq!(world.items.len(), 7);
    assert_eq!(world.events.len(), 2);
    let sprite = &world.options[6];
    assert_eq!(sprite.display, "Player Sprite");
    assert!(
        matches!(&sprite.kind, OptionKind::Choice { choices, aliases }
        if choices.len() == 4 && aliases == &[("kitty".to_string(), 3)])
    );
    assert_eq!(world.options[3].display, "Trap Chance");
}

#[test]
fn datapackage_trie_par_numero_et_complet() {
    let world = parse_world(APQUEST).unwrap();
    let locations = world.package_locations();
    let ids: Vec<i64> = locations.iter().map(|(_, id)| *id).collect();
    // Tous les lieux, même ceux qu'une option retire (coffre en plus).
    assert_eq!(ids, [1, 2, 3, 4, 5, 6, 10]);
    assert_eq!(world.package_items().len(), 7);
}

#[test]
fn simplification_des_constantes() {
    let world = parse_world(APQUEST).unwrap();
    let (w, _) = build(&world, &[]);
    let sword = w.item_named(w.slot_ids().next().unwrap(), "Sword").unwrap();
    assert_eq!(rule_of(&w, "Right Room Enemy Drop"), Rule::has(sword));
    let (w, _) = build(&world, &[("hard_mode", 1)]);
    assert!(matches!(rule_of(&w, "Right Room Enemy Drop"), Rule::And(parts) if parts.len() == 2));
}

#[test]
fn elements_conditionnels() {
    let world = parse_world(APQUEST).unwrap();
    let (w, _) = build(&world, &[]);
    assert!(w.regions().iter().all(|r| r.name != "Top Middle Room"));
    let chest = w
        .locations()
        .iter()
        .find(|l| l.name == "Top Middle Chest")
        .unwrap();
    assert_eq!(w.region(chest.region).unwrap().name, "Overworld");
    let (w, built) = build(&world, &[("hammer", 1), ("extra_starting_chest", 1)]);
    let chest = w
        .locations()
        .iter()
        .find(|l| l.name == "Top Middle Chest")
        .unwrap();
    assert_eq!(w.region(chest.region).unwrap().name, "Top Middle Room");
    assert!(w
        .locations()
        .iter()
        .any(|l| l.name == "Bottom Left Extra Chest"));
    // Sept lieux à numéro, donc sept objets à placer.
    assert_eq!(built.pool.len(), 7);
}

#[test]
fn classification_selon_les_options() {
    let world = parse_world(APQUEST).unwrap();
    let class = |options: &[(&str, i64)]| {
        let (w, _) = build(&world, options);
        w.item_kinds()
            .iter()
            .find(|i| i.name == "Health Upgrade")
            .unwrap()
            .classification
    };
    assert_eq!(class(&[]), Classification::USEFUL);
    assert_eq!(class(&[("hard_mode", 1)]), Classification::PROGRESSION);
}

#[test]
fn slot_data_et_spoiler() {
    let world = parse_world(APQUEST).unwrap();
    let (_, built) = build(&world, &[("player_sprite", 3), ("hard_mode", 1)]);
    assert_eq!(
        built.slot_data.to_string(),
        r#"{"hard_mode":1,"hammer":0,"extra_starting_chest":0,"confetti_explosiveness":3,"player_sprite":3}"#
    );
    let mut values = world.default_values();
    world.set_option(&mut values, "player_sprite", 1).unwrap();
    let spoiler = world.spoiler_options(&values);
    assert_eq!(spoiler[0], ("Hard Mode".to_string(), "No".to_string()));
    assert_eq!(
        spoiler[6],
        ("Player Sprite".to_string(), "Duck".to_string())
    );
}

/// Un monde de démonstration qui touche à tout le reste du langage.
const COMPLET: &str = r#"
format_version = 1

[game]
name = "Complet"
start_region = "Hub"
goal = 'has_group(Gems, option(gems_needed)) and can_reach_entrance("Hub to Sky")'

[[options]]
name = "gems_needed"
type = "range"
min = 1
max = 3
default = 2

[[options]]
name = "mode"
type = "choice"
choices = ["easy", "normal", "hard"]
default = "normal"

[[items]]
name = "Red Gem"
id = 10
classification = "progression"

[[items]]
name = "Blue Gem"
id = 11
classification = "progression_skip_balancing"

[[items]]
name = "Wings"
id = 12
classification = "progression+useful"

[[items]]
name = "Dust"
id = 13
classification = "filler"

[[item_groups]]
name = "Gems"
items = ["Red Gem", "Blue Gem"]

[[regions]]
name = "Hub"

[[regions]]
name = "Sky"

[[entrances]]
name = "Hub to Sky"
from = "Hub"
to = "Sky"
rule = 'has(Wings) and (option(mode) <= normal or has_any("Red Gem", "Blue Gem"))'

[[locations]]
name = "Altar"
id = 1
region = "Hub"
progress = "priority"

[[locations]]
name = "Cloud"
id = 2
region = "Sky"
rule = "can_reach(Altar) and option(mode) != hard"
progress = "excluded"

[[locations]]
name = "Peak"
id = 3
region = "Sky"
rule = "has_all(Wings)"

[[location_groups]]
name = "High"
locations = ["Cloud", "Peak"]

[[pool]]
item = "Red Gem"

[[pool]]
item = "Blue Gem"

[[pool]]
item = "Wings"

[[start]]
item = "Dust"
count = "option(gems_needed)"

[[slot_data]]
key = "mode"
option = "mode"

[[slot_data]]
key = "version"
value = { major = 1, tags = ["a", "b"] }
"#;

#[test]
fn monde_complet() {
    let world = parse_world(COMPLET).unwrap();
    // Règle de l'objectif, analysée.
    assert!(matches!(&world.goal, Expr::And(parts) if matches!(parts[0],
        Expr::HasGroup { group: 0, count: Count::Option(0) })));
    assert!(world.items[1].classification.cases[0].1.skip_balancing);
    let (w, built) = build(&world, &[]);
    let slot = w.slot_ids().next().unwrap();
    // mode = normal (1) <= normal : la partie « or » est vraie, il reste has(Wings).
    let wings = w.item_named(slot, "Wings").unwrap();
    let entrance = w.entrance_named(slot, "Hub to Sky").unwrap();
    assert_eq!(w.entrance(entrance).unwrap().rule, Rule::has(wings));
    let cloud = w.location_named(slot, "Cloud").unwrap();
    assert_eq!(w.location(cloud).unwrap().progress, Progress::Excluded);
    assert!(matches!(
        w.location(cloud).unwrap().rule,
        Rule::CanReachLocation(_)
    ));
    assert_eq!(built.start.len(), 2);
    assert_eq!(
        built.slot_data.to_string(),
        r#"{"mode":1,"version":{"major":1,"tags":["a","b"]}}"#
    );
    // Mode difficile : le lieu « Cloud » devient inaccessible, l'entrée demande une gemme.
    let (w, _) = build(&world, &[("mode", 2), ("gems_needed", 3)]);
    let cloud = w.location_named(slot, "Cloud").unwrap();
    assert_eq!(w.location(cloud).unwrap().rule, Rule::False);
    let entrance = w.entrance_named(slot, "Hub to Sky").unwrap();
    assert!(matches!(w.entrance(entrance).unwrap().rule, Rule::And(_)));
    assert!(
        matches!(w.slots()[0].goal, Rule::And(ref p) if matches!(p[0], Rule::HasGroup { count: 3, .. }))
    );
    // Datapackage : groupes déclarés.
    assert_eq!(
        world.package_item_groups(),
        [("Gems", vec!["Red Gem", "Blue Gem"])]
    );
    assert_eq!(
        world.package_location_groups(),
        [("High", vec!["Cloud", "Peak"])]
    );
}

#[test]
fn meme_fichier_meme_monde() {
    let a = parse_world(APQUEST).unwrap();
    let b = parse_world(APQUEST).unwrap();
    assert_eq!(a, b);
    let (wa, ba) = build(&a, &[("hammer", 1), ("trap_chance", 40)]);
    let (wb, bb) = build(&b, &[("hammer", 1), ("trap_chance", 40)]);
    assert_eq!(ba, bb);
    assert_eq!(wa.locations(), wb.locations());
}

#[test]
fn chemins_de_fichier() {
    let dir = std::env::temp_dir().join(format!("raftup-natif-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("world.toml"), MINIMAL).unwrap();
    // Un dossier se lit par son `world.toml`, un fichier directement.
    assert_eq!(worlds_native::load_path(&dir).unwrap().game, "Minimal");
    assert_eq!(
        worlds_native::load_path(&dir.join("world.toml"))
            .unwrap()
            .game,
        "Minimal"
    );
    assert!(matches!(
        worlds_native::load_path(&dir.join("absent.toml")),
        Err(worlds_native::LoadError::Io { .. })
    ));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn has_avec_option_et_zero() {
    let text = MINIMAL.replace("rule = \"has(Key)\"", "rule = \"has(Key, option(n))\"")
        + "\n[[options]]\nname = \"n\"\ntype = \"range\"\nmin = 0\nmax = 5\ndefault = 0\n";
    let world = parse_world(&text).unwrap();
    // n = 0 : « au moins zéro clé » est toujours vrai.
    let (w, _) = build(&world, &[]);
    assert_eq!(w.entrances()[0].rule, Rule::True);
    let (w, _) = build(&world, &[("n", 2)]);
    assert!(matches!(w.entrances()[0].rule, Rule::Has { count: 2, .. }));
    assert!(matches!(world.goal, Expr::CanReachLocation(_)));
}
