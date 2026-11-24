//! Conversion d'un apworld en monde natif (ADR 0042), sur le vrai export d'APQuest
//! (`tests/data/apquest-export.json`, options par défaut, écrit par Archipelago 0.6.8).
//!
//! Le monde converti doit redonner le portage Rust d'APQuest aux options par défaut :
//! mêmes structures, même datapackage, et même seed `.archipelago` à l'octet près, sans
//! retouche à la main.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use generate::convert::{convert, Conversion, ConvertError};
use generate::imported::read_export;
use generate::worlds::apquest::ApQuestOptions;
use generate::{build, generate, GameOptions, GenerateRequest, NativeOptions, PlayerRequest};
use logic::Rule;
use serde_json::Value;

const EXPORT: &[u8] = include_bytes!("data/apquest-export.json");

fn converted() -> Conversion {
    let export = read_export(EXPORT).unwrap();
    let slot_data = ApQuestOptions::default().slot_data();
    convert(&export, 1, Some(&slot_data)).unwrap()
}

fn native(toml: &str) -> GameOptions {
    let world = Arc::new(worlds_native::parse_world(toml).unwrap());
    GameOptions::Native(NativeOptions::new(world))
}

fn request(seed: u64, a: GameOptions, b: GameOptions) -> GenerateRequest {
    GenerateRequest::new(
        seed,
        vec![PlayerRequest::new("Alice", a), PlayerRequest::new("Bob", b)],
    )
}

fn port() -> GameOptions {
    GameOptions::ApQuest(ApQuestOptions::default())
}

/// Règle aux listes triées, pour comparer sans tenir compte de leur ordre.
fn normalized(rule: Rule) -> Rule {
    match rule {
        Rule::HasAll(mut items) => {
            items.sort_by_key(|i| i.index());
            Rule::HasAll(items)
        }
        Rule::HasAny(mut items) => {
            items.sort_by_key(|i| i.index());
            Rule::HasAny(items)
        }
        Rule::And(parts) => Rule::And(parts.into_iter().map(normalized).collect()),
        Rule::Or(parts) => Rule::Or(parts.into_iter().map(normalized).collect()),
        other => other,
    }
}

#[test]
fn conversion_relue_et_points_a_verifier() {
    let c = converted();
    let world = worlds_native::parse_world(&c.toml).unwrap();
    assert_eq!(world.game, "APQuest");
    // 6 lieux présents, et « Bottom Left Extra Chest », absent sans l'option mais gardé
    // pour le datapackage.
    assert_eq!(world.locations.len(), 7);
    assert_eq!(world.events.len(), 2);
    assert!(world.options.is_empty(), "options en commentaire seulement");
    // Ce que l'outil n'a pas pu savoir est dit, en tête du fichier.
    assert!(
        c.notes.iter().any(|n| n.contains("[filler]")),
        "{:?}",
        c.notes
    );
    assert!(
        c.notes
            .iter()
            .any(|n| n.contains("Hammer") && n.contains("Math Trap")),
        "{:?}",
        c.notes
    );
    assert!(
        c.notes
            .iter()
            .any(|n| n.contains("Bottom Left Extra Chest")),
        "{:?}",
        c.notes
    );
    assert!(c.toml.contains("# À vérifier :"));
    // Les options de l'apworld sont recopiées en commentaire, prêtes à décommenter.
    assert!(c.toml.contains("# name = \"player_sprite\""));
    assert!(c.toml.contains("# aliases = { kitty = \"cat\" }"));
    // Ordre de création de l'apworld, donc celui du portage.
    let names: Vec<&str> = world.locations.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Bottom Left Chest",
            "Bottom Right Room Left Chest",
            "Bottom Right Room Right Chest",
            "Top Left Room Chest",
            "Right Room Enemy Drop",
            "Top Middle Chest",
            "Bottom Left Extra Chest",
        ]
    );
}

#[test]
fn conversion_deterministe() {
    assert_eq!(converted(), converted());
}

#[test]
fn meme_monde_que_le_portage_aux_options_par_defaut() {
    let toml = converted().toml;
    for seed in [1u64, 2, 3] {
        let a = build(&request(seed, port(), port())).unwrap();
        let b = build(&request(seed, native(&toml), native(&toml))).unwrap();
        assert_eq!(a.world.slots(), b.world.slots());
        assert_eq!(a.world.regions(), b.world.regions());
        assert_eq!(a.world.entrances(), b.world.entrances());
        // Lieux : identiques, à l'ordre près des listes de `has_all` (Archipelago trie
        // les noms dans `rule_builder`).
        let locations = |w: &logic::MultiWorld| -> Vec<logic::Location> {
            w.locations()
                .iter()
                .cloned()
                .map(|mut l| {
                    l.rule = normalized(l.rule);
                    l
                })
                .collect()
        };
        assert_eq!(locations(&a.world), locations(&b.world));
        assert_eq!(a.world.item_groups(), b.world.item_groups());
        assert_eq!(a.pool, b.pool);
        assert_eq!(a.start_items, b.start_items);
        assert_eq!(a.slot_data, b.slot_data);
        // Sortes d'objets : mêmes noms et numéros ; seules diffèrent les classifications
        // des objets absents du pool (inconnues de l'export, écrites « filler »).
        let names = |w: &logic::MultiWorld| -> Vec<(String, Option<i64>)> {
            w.item_kinds()
                .iter()
                .map(|k| (k.name.clone(), k.code))
                .collect()
        };
        assert_eq!(names(&a.world), names(&b.world));
    }
}

#[test]
fn meme_seed_archipelago_que_le_portage() {
    let toml = converted().toml;
    for seed in [7u64, 77, 2026, 20_261_120] {
        let a = generate(&request(seed, port(), port())).unwrap();
        let b = generate(&request(seed, native(&toml), native(&toml))).unwrap();
        assert!(a.archipelago == b.archipelago, "seed {seed}");
    }
}

#[test]
fn meme_datapackage_que_le_portage() {
    let toml = converted().toml;
    assert_eq!(port().package(), native(&toml).package());
}

/// Ce que le format 1 ne sait pas écrire est refusé, jamais converti de travers.
#[test]
fn refus_clairs() {
    let base: Value = serde_json::from_slice(EXPORT).unwrap();
    let refuse = |change: &dyn Fn(&mut Value)| -> String {
        let mut v = base.clone();
        change(&mut v);
        let export = read_export(&serde_json::to_vec(&v).unwrap()).unwrap();
        match convert(&export, 1, None) {
            Err(ConvertError::Unsupported(why)) => why,
            other => panic!("attendu un refus, obtenu {other:?}"),
        }
    };
    let why = refuse(&|v| {
        v["players"][0]["locations"][0]["rule"] = serde_json::json!({"t": "opaque", "why": "f"})
    });
    assert!(why.contains("opaque"), "{why}");
    let why = refuse(&|v| v["players"][0]["custom_collect"] = Value::Bool(true));
    assert!(why.contains("collect"), "{why}");
    let why = refuse(&|v| {
        v["players"][0]["locations"][0]["rule"] =
            serde_json::json!({"t": "has", "player": 2, "item": "Key", "count": 1})
    });
    assert!(why.contains("joueur 2"), "{why}");
    // Un lieu à numéro qui porte l'objet d'un autre joueur, ou un objet sans numéro.
    let why = refuse(&|v| {
        v["players"][0]["locations"][0]["item"] =
            serde_json::json!({"player": 2, "name": "Key", "code": 1, "classification": 1})
    });
    assert!(why.contains("autre joueur"), "{why}");
    let why = refuse(&|v| {
        v["players"][0]["locations"][0]["item"] =
            serde_json::json!({"player": 1, "name": "Key", "code": null, "classification": 1})
    });
    assert!(why.contains("sans numéro"), "{why}");
    assert!(matches!(
        convert(&read_export(EXPORT).unwrap(), 9, None),
        Err(ConvertError::UnknownPlayer(9))
    ));
}

/// Règles de `rule_builder` dépliées : « au moins N », comptes.
#[test]
fn regles_depliees() {
    let mut v: Value = serde_json::from_slice(EXPORT).unwrap();
    v["players"][0]["locations"][0]["rule"] = serde_json::json!({"t": "at_least", "count": 2, "of": [
        {"t": "has", "player": 1, "item": "Key", "count": 1},
        {"t": "has", "player": 1, "item": "Sword", "count": 1},
        {"t": "has_any_count", "player": 1, "items": [["Shield", 1], ["Health Upgrade", 2]]}
    ]});
    let export = read_export(&serde_json::to_vec(&v).unwrap()).unwrap();
    let c = convert(&export, 1, None).unwrap();
    assert!(
        c.toml.contains(
            "rule = 'has(Key) and has(Sword) or has(Key) and (has(Shield) or has(\"Health Upgrade\", 2)) or has(Sword) and (has(Shield) or has(\"Health Upgrade\", 2))'"
        ),
        "{}",
        c.toml
    );
}

/// `has_all` et `has_any` d'une liste vide (Mega Man 2 et 3 dans l'inventaire) : le
/// format refuse `has_all()`, la conversion écrit `true` et `false`, comme Python.
#[test]
fn listes_vides() {
    let mut v: Value = serde_json::from_slice(EXPORT).unwrap();
    v["players"][0]["locations"][0]["rule"] = serde_json::json!({"t": "or", "of": [
        {"t": "and", "of": [
            {"t": "has_all", "player": 1, "items": []},
            {"t": "has", "player": 1, "item": "Key", "count": 1}
        ]},
        {"t": "has_any", "player": 1, "items": []}
    ]});
    let export = read_export(&serde_json::to_vec(&v).unwrap()).unwrap();
    let c = convert(&export, 1, None).unwrap();
    assert!(
        c.toml.contains("rule = 'true and has(Key) or false'"),
        "{}",
        c.toml
    );
    worlds_native::parse_world(&c.toml).unwrap();
}

/// Exemplaires d'un même objet aux classifications différentes (Muse Dash, Sonic
/// Adventure 2 Battle, Saving Princess) : l'union des drapeaux est gardée, pour que
/// l'objet reste une progression même si le premier exemplaire n'en est pas une.
#[test]
fn classifications_mixtes_union() {
    let mut v: Value = serde_json::from_slice(EXPORT).unwrap();
    v["pool"][3]["classification"] = Value::from(0);
    v["pool"][4]["classification"] = Value::from(1);
    let export = read_export(&serde_json::to_vec(&v).unwrap()).unwrap();
    let c = convert(&export, 1, None).unwrap();
    assert!(
        c.toml
            .contains("name = \"Health Upgrade\"\nid = 5\nclassification = \"progression\""),
        "{}",
        c.toml
    );
    assert!(
        c.notes
            .iter()
            .any(|n| n.contains("Health Upgrade") && n.contains("union")),
        "{:?}",
        c.notes
    );
}

/// `start_inventory_from_pool`, déclarée par chaque monde mais appliquée par Archipelago
/// avant l'export, n'est ni recopiée ni signalée comme une option du jeu.
#[test]
fn option_commune_ignoree() {
    let mut v: Value = serde_json::from_slice(EXPORT).unwrap();
    v["players"][0]["options"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"name": "start_inventory_from_pool",
            "display": "Start Inventory from Pool", "kind": "other"}));
    let export = read_export(&serde_json::to_vec(&v).unwrap()).unwrap();
    let c = convert(&export, 1, None).unwrap();
    assert!(!c.toml.contains("start_inventory_from_pool"), "{}", c.toml);
    assert!(
        !c.notes
            .iter()
            .any(|n| n.contains("start_inventory_from_pool")),
        "{:?}",
        c.notes
    );
}

/// Objet posé d'avance dans un lieu à numéro (ADR 0054, DOOM, Meritous…) : écrit en
/// format 3, `item = …` sur le lieu, hors du pool, verrouillé dans la seed Rust.
#[test]
fn objet_pose_d_avance_format_3() {
    let mut v: Value = serde_json::from_slice(EXPORT).unwrap();
    // « Key » posée par le monde dans « Bottom Left Chest » : elle sort du pool exporté,
    // comme le fait `place_locked_item` d'Archipelago.
    let key = v["pool"].as_array_mut().unwrap().remove(0);
    assert_eq!(key["name"], "Key");
    let location = &mut v["players"][0]["locations"][0];
    location["item"] = key;
    location["locked"] = Value::Bool(true);
    let export = read_export(&serde_json::to_vec(&v).unwrap()).unwrap();
    let c = convert(&export, 1, None).unwrap();
    assert!(c.toml.contains("\nformat_version = 3\n"), "{}", c.toml);
    assert!(
        c.toml.contains(
            "name = \"Bottom Left Chest\"\nid = 3\nregion = \"Overworld\"\nitem = \"Key\"\n"
        ),
        "{}",
        c.toml
    );
    assert!(
        c.toml
            .contains("name = \"Key\"\nid = 1\nclassification = \"progression\""),
        "{}",
        c.toml
    );
    assert!(
        c.notes.iter().any(|n| n.contains("posé(s) d'avance")),
        "{:?}",
        c.notes
    );

    // Moteur Rust : la clé reste dans son lieu, verrouillée, et la seed se génère.
    for seed in [1u64, 2, 3] {
        let built = build(&request(seed, native(&c.toml), native(&c.toml))).unwrap();
        let chests: Vec<&logic::Location> = built
            .world
            .locations()
            .iter()
            .filter(|l| l.name == "Bottom Left Chest")
            .collect();
        assert_eq!(chests.len(), 2);
        for chest in chests {
            assert!(chest.locked);
            assert_eq!(chest.item, built.world.item_named(chest.slot, "Key"));
        }
        generate(&request(seed, native(&c.toml), native(&c.toml))).unwrap();
    }
}

/// Sans objet posé ni événement partagé, la conversion reste en format 1.
#[test]
fn version_minimale() {
    assert!(converted().toml.contains("\nformat_version = 1\n"));
}

/// Événement qui donne un objet portant le nom d'un objet à numéro (Super Mario Land 2) :
/// format 3, l'événement nomme l'objet déclaré, qui reste une progression.
#[test]
fn evenement_qui_donne_un_objet_a_numero() {
    let mut v: Value = serde_json::from_slice(EXPORT).unwrap();
    // Un nouvel événement donne « Shield » (sans numéro, comme chez Archipelago).
    v["players"][0]["locations"].as_array_mut().unwrap().push(
        serde_json::json!({"name": "Shield Shrine", "address": null,
            "region": "Overworld", "rule": {"t": "true"}, "progress_type": 1,
            "locked": true, "item": {"player": 1, "name": "Shield", "code": null,
            "classification": 1}}),
    );
    let export = read_export(&serde_json::to_vec(&v).unwrap()).unwrap();
    let c = convert(&export, 1, None).unwrap();
    assert!(c.toml.contains("\nformat_version = 3\n"), "{}", c.toml);
    assert!(
        c.toml.contains(
            "[[events]]\nname = \"Shield Shrine\"\nregion = \"Overworld\"\nitem = \"Shield\"\n"
        ),
        "{}",
        c.toml
    );
    let world = worlds_native::parse_world(&c.toml).unwrap();
    assert_eq!(world.items.iter().filter(|i| i.name == "Shield").count(), 1);
}

/// Objet cité par une règle mais jamais créé (« CANNOT ACCESS » de Celeste 64) : compté à
/// zéro par Archipelago, écrit `false` (avant : monde produit illisible).
#[test]
fn objet_jamais_cree_vaut_false() {
    let mut v: Value = serde_json::from_slice(EXPORT).unwrap();
    v["players"][0]["locations"][0]["rule"] = serde_json::json!({"t": "or", "of": [
        {"t": "has", "player": 1, "item": "CANNOT ACCESS", "count": 1},
        {"t": "has_any", "player": 1, "items": ["CANNOT ACCESS", "Key"]},
        {"t": "has_all", "player": 1, "items": ["CANNOT ACCESS", "Key"]}
    ]});
    let export = read_export(&serde_json::to_vec(&v).unwrap()).unwrap();
    let c = convert(&export, 1, None).unwrap();
    assert!(
        c.toml.contains("rule = 'false or has_any(Key) or false'"),
        "{}",
        c.toml
    );
    assert!(
        c.notes.iter().any(|n| n.contains("CANNOT ACCESS")),
        "{:?}",
        c.notes
    );
}

/// Sommes de compteurs (ADR 0055) : une liste qui n'est pas un groupe, « au moins k parmi
/// n » et une somme pondérée s'écrivent en version 4 ; un cas simple reste en version 1.
#[test]
fn sommes_de_compteurs_en_version_4() {
    let base: Value = serde_json::from_slice(EXPORT).unwrap();
    let with_rule = |rule: Value| -> String {
        let mut v = base.clone();
        v["players"][0]["locations"][0]["rule"] = rule;
        let export = read_export(&serde_json::to_vec(&v).unwrap()).unwrap();
        convert(&export, 1, None).unwrap().toml
    };
    let toml = with_rule(serde_json::json!(
        {"t": "has_from_list", "player": 1, "items": ["Key", "Sword"], "count": 2}));
    assert!(
        toml.contains(
            "
format_version = 4
"
        ),
        "{toml}"
    );
    assert!(toml.contains("count_from_list(Key, Sword) >= 2"), "{toml}");
    let toml = with_rule(serde_json::json!(
        {"t": "has_from_list_unique", "player": 1, "items": ["Key", "Sword", "Shield"], "count": 2}));
    assert!(
        toml.contains("count_unique(Key, Sword, Shield) >= 2"),
        "{toml}"
    );
    let toml = with_rule(serde_json::json!(
        {"t": "sum", "player": 1, "terms": [["Sword", 2, false], ["Key", 1, false], ["Nope", 5, false]],
         "count": 3}));
    assert!(
        toml.contains("2 * count(Sword) + count(Key) >= 3"),
        "{toml}"
    );
    // Un seul objet de poids 1 : `has`, et la version ne monte pas.
    let toml = with_rule(serde_json::json!(
        {"t": "sum", "player": 1, "terms": [["Key", 1, false]], "count": 2}));
    assert!(toml.contains("has(Key, 2)"), "{toml}");
    assert!(!toml.contains("format_version = 4"), "{toml}");
}
