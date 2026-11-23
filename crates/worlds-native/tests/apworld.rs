//! Export inverse : monde natif → apworld Python (ADR 0042, § 4).
//!
//! Ici, sans Python : fichiers produits, texte déterministe, noms hostiles échappés. Le vrai
//! Archipelago charge l'apworld dans `launcher-core/tests/conversion_real.rs` (`#[ignore]`).

#![allow(clippy::unwrap_used)]

use worlds_native::apworld::{module_name, to_apworld, ApworldConfig, ApworldError, ApworldFile};

const APQUEST: &str = include_str!("../../../spec/monde-natif/exemples/apquest/world.toml");
const MINIMAL: &str = include_str!("../../../spec/monde-natif/exemples/minimal/world.toml");

fn files(toml: &str) -> Vec<ApworldFile> {
    let world = worlds_native::parse_world(toml).unwrap();
    to_apworld(&world, &ApworldConfig::for_world(&world)).unwrap()
}

fn file<'a>(files: &'a [ApworldFile], name: &str) -> &'a str {
    &files.iter().find(|f| f.name == name).unwrap().contents
}

#[test]
fn fichiers_d_un_apworld() {
    let f = files(APQUEST);
    let names: Vec<&str> = f.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "archipelago.json",
            "__init__.py",
            "options.py",
            "items.py",
            "locations.py",
            "regions.py",
            "rules.py"
        ]
    );
    let manifest: serde_json::Value = serde_json::from_str(file(&f, "archipelago.json")).unwrap();
    assert_eq!(manifest["game"], "APQuest");
}

#[test]
fn apquest_en_python() {
    let f = files(APQUEST);
    let items = file(&f, "items.py");
    assert!(items.contains("\"Health Upgrade\": 5,"), "{items}");
    // Classification qui dépend d'une option : un `if` sur l'option.
    assert!(
        items.contains("if bool(w.options.hard_mode.value):"),
        "{items}"
    );
    // Remplissage avec pièges, comme `randint(0, 99) < trap_chance`.
    assert!(
        items.contains("w.random.randint(0, 99) < w.options.trap_chance.value"),
        "{items}"
    );
    assert!(items.contains("\"player_sprite\": w.options.player_sprite.value,"));
    let options = file(&f, "options.py");
    assert!(options.contains("\"option_cat\": 3,"), "{options}");
    assert!(options.contains("\"alias_kitty\": 3,"), "{options}");
    assert!(
        options.contains("    trap_chance: Option_trap_chance"),
        "{options}"
    );
    let regions = file(&f, "regions.py");
    assert!(regions
        .contains("    if bool(w.options.hammer.value):\n        add_region(\"Top Middle Room\")"));
    assert!(regions.contains("w.origin_region_name = \"Overworld\""));
    let rules = file(&f, "rules.py");
    assert!(
        rules.contains("(state.has(\"Sword\", w.player, 1) and ((not bool(w.options.hard_mode.value)) or state.has(\"Shield\", w.player, 1) or state.has(\"Health Upgrade\", w.player, 1)))"),
        "{rules}"
    );
}

#[test]
fn texte_deterministe() {
    assert_eq!(files(APQUEST), files(APQUEST));
    assert_eq!(files(MINIMAL), files(MINIMAL));
}

/// Un nom qui ressemble à du Python reste une chaîne : rien n'est exécuté.
#[test]
fn noms_hostiles_echappes() {
    let hostile = MINIMAL.replacen(
        "name = \"Minimal\"",
        "name = \"Evil\\\"); import os; os.system(\\\"x\\\") #\"",
        1,
    );
    assert_ne!(hostile, MINIMAL, "le fichier d'exemple a changé");
    let world = worlds_native::parse_world(&hostile).unwrap();
    let config = ApworldConfig {
        module: "evil".into(),
        world_version: "0.1.0".into(),
        authors: vec![],
    };
    let f = to_apworld(&world, &config).unwrap();
    let items = file(&f, "items.py");
    assert!(
        items.contains("GAME = \"Evil\\\"); import os; os.system(\\\"x\\\") #\""),
        "{items}"
    );
    // La docstring ne peut pas être refermée.
    let init = file(&f, "__init__.py");
    assert!(!init.contains("\"\"\"Monde natif Evil\""), "{init}");
}

#[test]
fn nom_de_module_verifie() {
    let world = worlds_native::parse_world(MINIMAL).unwrap();
    let mut config = ApworldConfig::for_world(&world);
    config.module = "../evil".into();
    assert_eq!(
        to_apworld(&world, &config),
        Err(ApworldError::BadModule("../evil".into()))
    );
    assert_eq!(module_name("Slime Rancher"), "slime_rancher");
}

#[test]
fn objets_poses_d_avance_en_python() {
    let toml = r#"
format_version = 3

[game]
name = "Doomish"
goal = 'has("E1M1 - Complete")'

[[options]]
name = "fast_exit"
type = "toggle"
default = false

[[items]]
name = "E1M1 - Complete"
id = 1
classification = "progression"

[[items]]
name = "Shells"
id = 2
classification = "filler"

[[regions]]
name = "Hangar"

[[locations]]
name = "E1M1 - Exit"
id = 10
region = "Hangar"
item = "E1M1 - Complete"

[[locations]]
name = "Shop"
id = 11
region = "Hangar"
item = [{ when = "option(fast_exit)", value = "E1M1 - Complete" }, { value = "Shells" }]

[filler]
item = "Shells"
"#;
    let f = files(toml);
    let regions = file(&f, "regions.py");
    assert!(
        regions.contains("    location.place_locked_item(w.create_item(\"E1M1 - Complete\"))\n"),
        "{regions}"
    );
    assert!(regions.contains("    if "), "{regions}");
    assert!(
        regions
            .contains("    else:\n        location.place_locked_item(w.create_item(\"Shells\"))\n"),
        "{regions}"
    );
}
