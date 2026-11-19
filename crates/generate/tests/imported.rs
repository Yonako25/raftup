//! Lecture d'un export de mondes (ADR 0028, étape 1), sur des exports écrits à la main.

#![allow(clippy::unwrap_used)]

use generate::imported::{build_world, read_export, ImportError, MAX_EXPORT_BYTES};
use logic::{sweep, Reachability, Rule};
use serde_json::{json, Value};

/// Un joueur « Alice » : Menu → Cave (porte : Key) ; lieux Chest (Menu), Deep (Cave),
/// Boss (Cave, événement Victory). Pool : Key, Rupee.
fn base() -> Value {
    json!({
        "format_version": 2,
        "seed": 1,
        "seed_name": "1",
        "players": [{
            "slot": 1, "name": "Alice", "game": "Test", "origin": "Menu",
            "regions": ["Menu", "Cave"],
            "entrances": [{"name": "Door", "from": "Menu", "to": "Cave",
                           "rule": {"t": "has", "player": 1, "item": "Key", "count": 1}}],
            "locations": [
                {"name": "Chest", "address": 1, "region": "Menu", "rule": {"t": "true"},
                 "progress_type": 1, "locked": false, "item": null},
                {"name": "Deep", "address": 2, "region": "Cave", "rule": {"t": "true"},
                 "progress_type": 1, "locked": false, "item": null},
                {"name": "Boss", "address": null, "region": "Cave", "rule": {"t": "true"},
                 "progress_type": 1, "locked": true,
                 "item": {"player": 1, "name": "Victory", "code": null, "classification": 1}}
            ],
            "precollected": [],
            "goal": {"t": "has", "player": 1, "item": "Victory", "count": 1},
            "required_client_version": [0, 1, 6]
        }],
        "pool": [
            {"player": 1, "name": "Key", "code": 10, "classification": 1},
            {"player": 1, "name": "Rupee", "code": 11, "classification": 0}
        ],
        "datapackage": {},
        "opaque_rules": 0
    })
}

fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}

fn import(v: &Value) -> Result<generate::imported::Imported, ImportError> {
    build_world(&read_export(&bytes(v))?)
}

fn unsupported(v: &Value) -> String {
    match import(v) {
        Err(ImportError::Unsupported { reason, .. }) => reason,
        other => panic!("attendu Unsupported, obtenu {other:?}"),
    }
}

/// Règle du lieu « Deep » remplacée par `rule`.
fn with_deep_rule(rule: Value) -> Value {
    let mut v = base();
    v["players"][0]["locations"][1]["rule"] = rule;
    v
}

#[test]
fn export_simple_relu_et_jouable() {
    let imported = import(&base()).unwrap();
    let w = &imported.world;
    assert_eq!(w.slots().len(), 1);
    assert_eq!(w.locations().len(), 3);
    assert_eq!(imported.pool.len(), 2);
    // L'événement est posé, la porte demande la clé.
    let alice = w.slot_ids().next().unwrap();
    let boss = w.location_named(alice, "Boss").unwrap();
    assert!(w.location(boss).unwrap().item.is_some());
    let reach = Reachability::compute(w, &imported.start);
    let deep = w.location_named(alice, "Deep").unwrap();
    assert!(!reach.location(deep));
    // Avec tout le pool en main : tout s'ouvre et la victoire est atteinte.
    let mut state = imported.start.clone();
    for item in &imported.pool {
        state.collect(w, *item);
    }
    let result = sweep(w, state);
    assert!(result.unreachable_locations(w).is_empty());
    assert!(result.all_goals_reached(w));
}

#[test]
fn regle_opaque_refusee() {
    let v = with_deep_rule(json!({"t": "opaque", "why": "worlds.test.<lambda>"}));
    let reason = unsupported(&v);
    assert!(
        reason.contains("Deep") && reason.contains("<lambda>"),
        "{reason}"
    );
    // Dans un « et », aussi.
    let v = with_deep_rule(json!({"t": "and", "of": [{"t": "true"}, {"t": "opaque", "why": "x"}]}));
    assert!(unsupported(&v).contains("Deep"));
    // Dans la condition de victoire.
    let mut v = base();
    v["players"][0]["goal"] = json!({"t": "opaque", "why": "goal"});
    assert!(unsupported(&v).contains("victoire"));
}

/// L'export de base où « Chest » refuse la roupie (règle d'objet n° 0).
fn with_item_rule() -> Value {
    let mut v = base();
    v["item_rules"] = json!([{"allow_only": false, "list": [[1, "Rupee"]], "always": []}]);
    v["players"][0]["locations"][0]["item_rule"] = json!(0);
    v
}

#[test]
fn regles_d_objet_lues_et_respectees() {
    let v = with_item_rule();
    let mut imported = import(&v).unwrap();
    let w = &imported.world;
    let alice = w.slot_ids().next().unwrap();
    let chest = w.location_named(alice, "Chest").unwrap();
    let deep = w.location_named(alice, "Deep").unwrap();
    let key = w.item_named(alice, "Key").unwrap();
    let rupee = w.item_named(alice, "Rupee").unwrap();
    assert!(w.can_hold(chest, key));
    assert!(!w.can_hold(chest, rupee));
    assert!(w.can_hold(deep, rupee));
    // Le placement respecte la règle : la roupie ne va jamais dans « Chest ».
    let export = read_export(&bytes(&v)).unwrap();
    for seed in 0..20 {
        let mut again = imported.clone();
        let lines = generate::imported::fill_imported(&mut again, &export, seed).unwrap();
        assert!(
            lines.contains(&(1, "Chest".to_string(), 1, "Key".to_string())),
            "{lines:?}"
        );
    }
    // « Seuls acceptés » et « toujours acceptés » : Chest exclu accepte quand même la clé.
    let mut v = base();
    v["item_rules"] = json!([{"allow_only": true, "list": [[1, "Rupee"]], "always": [[1, "Key"]]}]);
    v["players"][0]["locations"][0]["item_rule"] = json!(0);
    v["players"][0]["locations"][0]["progress_type"] = json!(3);
    imported = import(&v).unwrap();
    let w = &imported.world;
    assert!(w.can_hold(chest, key), "always_allow passe l'exclusion");
    assert!(w.can_hold(chest, rupee));
}

#[test]
fn regles_d_objet_incoherentes_ou_non_traduites() {
    let mut v = with_item_rule();
    v["players"][0]["locations"][0]["item_rule"] = json!(5);
    assert!(matches!(import(&v), Err(ImportError::Invalid(_))));
    let mut v = with_item_rule();
    v["item_rules"][0]["list"] = json!([[1, "Inconnu"]]);
    assert!(matches!(import(&v), Err(ImportError::Invalid(_))));
    let mut v = base();
    v["players"][0]["item_rule_error"] = json!("lieu Chest : ValueError: x");
    let reason = unsupported(&v);
    assert!(
        reason.contains("règle d'objet") && reason.contains("Chest"),
        "{reason}"
    );
}

#[test]
fn controle_croise_des_regles_d_objet() {
    let mut v = with_item_rule();
    v["checks"] = json!({"samples": [], "rules": [], "item_checks": [
        [1, "Chest", 1, "Key", "1"], [1, "Chest", 1, "Rupee", "0"], [1, "Deep", 1, "Rupee", "1"]
    ]});
    import(&v).unwrap();
    v["checks"]["item_checks"][1][4] = json!("1");
    let reason = unsupported(&v);
    assert!(
        reason.contains("règle d'objet") && reason.contains("Rupee"),
        "{reason}"
    );
    v["checks"]["item_checks"][1][4] = json!("e");
    assert!(unsupported(&v).contains("plante"));
}

#[test]
fn objets_a_placer_tot_et_accessibilite_lus() {
    let mut v = base();
    v["players"][0]["early_items"] = json!({"Key": 1, "Absent": 2});
    v["players"][0]["local_early_items"] = json!({"Rupee": 1});
    v["players"][0]["accessibility"] = json!(2);
    let imported = import(&v).unwrap();
    let w = &imported.world;
    let alice = w.slot_ids().next().unwrap();
    let key = w.item_named(alice, "Key").unwrap();
    let rupee = w.item_named(alice, "Rupee").unwrap();
    assert_eq!(
        imported.early,
        vec![
            fill::EarlyItem {
                item: key,
                count: 1,
                local: false
            },
            fill::EarlyItem {
                item: rupee,
                count: 1,
                local: true
            },
        ],
        "un objet absent du pool est ignoré"
    );
    assert_eq!(
        w.slot(alice).unwrap().accessibility,
        logic::Accessibility::Minimal
    );
    let mut v = base();
    v["players"][0]["accessibility"] = json!(7);
    assert!(matches!(import(&v), Err(ImportError::Invalid(_))));
}

#[test]
fn lieux_prioritaires_et_exclus_lus() {
    let mut v = base();
    v["players"][0]["locations"][0]["progress_type"] = json!(3);
    v["players"][0]["locations"][1]["progress_type"] = json!(2);
    let imported = import(&v).unwrap();
    let w = &imported.world;
    let alice = w.slot_ids().next().unwrap();
    let progress = |n: &str| {
        w.location(w.location_named(alice, n).unwrap())
            .unwrap()
            .progress
    };
    assert_eq!(progress("Chest"), logic::Progress::Excluded);
    assert_eq!(progress("Deep"), logic::Progress::Priority);
    assert_eq!(progress("Boss"), logic::Progress::Default);
    // Valeur inconnue : export incohérent.
    let mut v = base();
    v["players"][0]["locations"][0]["progress_type"] = json!(7);
    assert!(matches!(import(&v), Err(ImportError::Invalid(_))));
}

#[test]
fn deux_classifications_pour_un_meme_objet() {
    let mut v = base();
    v["pool"][1] = json!({"player": 1, "name": "Key", "code": 10, "classification": 2});
    assert!(unsupported(&v).contains("Key"));
}

#[test]
fn liste_somme_et_objets_differents() {
    // has_from_list : 2 exemplaires au total de Key ou Rupee… mais Rupee n'est pas de
    // progression, seule Key compte. has_from_list_unique : 2 objets différents parmi
    // Key, Gem, Gem (Gem cité par la règle seulement).
    let v = with_deep_rule(json!({"t": "and", "of": [
        {"t": "has_from_list", "player": 1, "items": ["Key", "Gem"], "count": 2},
        {"t": "has_from_list_unique", "player": 1, "items": ["Key", "Gem", "Gem"], "count": 2}
    ]}));
    let imported = import(&v).unwrap();
    let w = &imported.world;
    let alice = w.slot_ids().next().unwrap();
    let key = w.item_named(alice, "Key").unwrap();
    let gem = w.item_named(alice, "Gem").unwrap();
    assert!(w.item_kind(gem).unwrap().classification.progression);
    let deep = w.location_named(alice, "Deep").unwrap();
    let reaches = |k: u32, g: u32| {
        let mut s = imported.start.clone();
        s.add(key, k);
        s.add(gem, g);
        Reachability::compute(w, &s).location(deep)
    };
    assert!(
        !reaches(2, 0),
        "deux clés : somme 2, mais un seul objet différent"
    );
    assert!(!reaches(1, 0));
    assert!(reaches(1, 1));
    assert!(reaches(2, 3));
}

#[test]
fn au_moins_n_parmi() {
    let v = with_deep_rule(json!({"t": "at_least", "count": 2, "of": [
        {"t": "has", "player": 1, "item": "Key", "count": 1},
        {"t": "has", "player": 1, "item": "A", "count": 1},
        {"t": "has", "player": 1, "item": "B", "count": 1}
    ]}));
    let imported = import(&v).unwrap();
    let w = &imported.world;
    let alice = w.slot_ids().next().unwrap();
    let deep = w.location_named(alice, "Deep").unwrap();
    let id = |n: &str| w.item_named(alice, n).unwrap();
    let reaches = |names: &[&str]| {
        let mut s = imported.start.clone();
        for n in names {
            s.add(id(n), 1);
        }
        Reachability::compute(w, &s).location(deep)
    };
    assert!(!reaches(&["Key"]));
    assert!(reaches(&["Key", "B"]));
    assert!(
        !reaches(&["A", "B"]),
        "A et B sans la clé : la porte reste fermée"
    );
    // Bornes : 0 ⇒ vraie, plus que de règles ⇒ fausse.
    let v = with_deep_rule(json!({"t": "at_least", "count": 0, "of": []}));
    let imported = import(&v).unwrap();
    let w = &imported.world;
    let deep = w
        .location_named(w.slot_ids().next().unwrap(), "Deep")
        .unwrap();
    assert_eq!(w.location(deep).unwrap().rule, Rule::True);
    let v = with_deep_rule(json!({"t": "at_least", "count": 3, "of": [{"t": "true"}]}));
    let imported = import(&v).unwrap();
    let w = &imported.world;
    let deep = w
        .location_named(w.slot_ids().next().unwrap(), "Deep")
        .unwrap();
    assert_eq!(w.location(deep).unwrap().rule, Rule::False);
}

#[test]
fn regle_qui_cite_un_lieu_cree_plus_tard() {
    // La porte (créée avant les lieux) demande d'atteindre « Chest ».
    let mut v = base();
    v["players"][0]["entrances"][0]["rule"] =
        json!({"t": "can_reach_location", "player": 1, "location": "Chest"});
    let imported = import(&v).unwrap();
    let w = &imported.world;
    let alice = w.slot_ids().next().unwrap();
    let deep = w.location_named(alice, "Deep").unwrap();
    assert!(Reachability::compute(w, &imported.start).location(deep));
}

#[test]
fn exports_incoherents_refuses() {
    let mut v = base();
    v["players"][0]["entrances"][0]["to"] = json!("Nulle part");
    assert!(matches!(import(&v), Err(ImportError::Invalid(_))));
    let mut v = base();
    v["players"][0]["entrances"][0]["to"] = Value::Null;
    assert!(matches!(import(&v), Err(ImportError::Invalid(_))));
    let v = with_deep_rule(json!({"t": "can_reach_region", "player": 1, "region": "Ailleurs"}));
    assert!(matches!(import(&v), Err(ImportError::Invalid(_))));
    let v = with_deep_rule(json!({"t": "has", "player": 9, "item": "Key", "count": 1}));
    assert!(matches!(import(&v), Err(ImportError::Invalid(_))));
    let mut v = base();
    v["players"][0]["slot"] = json!(2);
    assert!(matches!(import(&v), Err(ImportError::Invalid(_))));
    let mut v = base();
    v["players"][0]["regions"] = json!(["Menu", "Menu"]);
    assert!(matches!(import(&v), Err(ImportError::Logic(_))));
    let v = with_deep_rule(json!({"t": "inconnue"}));
    assert!(matches!(import(&v), Err(ImportError::Json(_))));
}

#[test]
fn format_taille_et_imbrication() {
    let mut v = base();
    v["format_version"] = json!(3);
    assert!(matches!(import(&v), Err(ImportError::Format(3))));
    let big = vec![b' '; MAX_EXPORT_BYTES + 1];
    assert!(matches!(read_export(&big), Err(ImportError::TooLarge(_))));
    // Une règle imbriquée sur 10 000 niveaux : refusée par serde_json, sans débordement.
    let mut rule = String::from("{\"t\":\"true\"}");
    for _ in 0..10_000 {
        rule = format!("{{\"t\":\"and\",\"of\":[{rule}]}}");
    }
    let text = serde_json::to_string(&base())
        .unwrap()
        .replacen("{\"t\":\"true\"}", &rule, 1);
    assert!(matches!(
        read_export(text.as_bytes()),
        Err(ImportError::Json(_))
    ));
}

#[test]
fn checksum_du_datapackage_verifie() {
    let package = |checksum: &str| {
        json!({"Test": {
            "item_name_to_id": {"Key": 10, "Rupee": 11},
            "location_name_to_id": {"Chest": 1, "Deep": 2},
            "item_name_groups": {"Everything": ["Key", "Rupee"]},
            "location_name_groups": {"Everywhere": ["Chest", "Deep"]},
            "checksum": checksum
        }})
    };
    let mut v = base();
    v["datapackage"] = package("faux");
    let Err(ImportError::Invalid(message)) = import(&v) else {
        panic!("checksum faux accepté");
    };
    // Le message donne le bon checksum : on le reprend, et l'import passe.
    let good = message
        .split("Raftup ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .unwrap()
        .to_string();
    v["datapackage"] = package(&good);
    let imported = import(&v).unwrap();
    assert_eq!(imported.packages["Test"].checksum, good);
}

/// L'export de base avec un contrôle croisé : deux inventaires (vide, une clé).
fn with_checks(door: &str, goal: &str) -> Value {
    let mut v = base();
    v["checks"] = json!({
        "samples": [[], [[1, "Key", 1]]],
        "rules": [
            {"kind": "entrance", "player": 1, "name": "Door", "results": door},
            {"kind": "location", "player": 1, "name": "Deep", "results": "11"},
            {"kind": "goal", "player": 1, "name": "Alice", "results": goal}
        ]
    });
    v
}

#[test]
fn controle_croise_d_accord() {
    import(&with_checks("01", "00")).unwrap();
}

#[test]
fn controle_croise_en_desaccord() {
    // Archipelago dit la porte ouverte sans clé : Raftup ne dit pas pareil.
    let reason = unsupported(&with_checks("11", "00"));
    assert!(
        reason.contains("contrôle croisé") && reason.contains("Door"),
        "{reason}"
    );
    assert!(reason.contains("n° 0"), "{reason}");
    // La condition de victoire est contrôlée aussi.
    assert!(unsupported(&with_checks("01", "01")).contains("goal"));
    // Une règle Python qui plante sur un inventaire : refusée.
    assert!(unsupported(&with_checks("0e", "00")).contains("plante"));
}

#[test]
fn controle_croise_incoherent() {
    // Pas autant de résultats que d'inventaires.
    assert!(matches!(
        import(&with_checks("0", "00")),
        Err(ImportError::Invalid(_))
    ));
    // Règle contrôlée introuvable.
    let mut v = with_checks("01", "00");
    v["checks"]["rules"][0]["name"] = json!("Fenêtre");
    assert!(matches!(import(&v), Err(ImportError::Invalid(_))));
    let mut v = with_checks("01", "00");
    v["checks"]["rules"][0]["kind"] = json!("region");
    assert!(matches!(import(&v), Err(ImportError::Invalid(_))));
}

#[test]
fn collect_personnalise_refuse() {
    let mut v = base();
    v["players"][0]["custom_collect"] = json!(true);
    assert!(unsupported(&v).contains("collect"));
}
