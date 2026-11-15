//! Le modèle de partie doit correspondre à ce que lit le serveur officiel.
//!
//! `tests/data/seed-mit.modele.json` a été calculé par Python avec le code
//! officiel (`restricted_loads`, puis `NetUtils.encode` pour les valeurs envoyées
//! en JSON). Voir `tests/data/README.md`.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;

use ap_protocol::{HintStatus, SlotType};
use multidata::{load_archipelago, ArchiveLimits, LoadError, MultiData};
use serde_json::{json, Map, Value};

/// Forme JSON du modèle, la même que celle du script Python.
fn model_json(m: &MultiData) -> Value {
    let version = |v: &ap_protocol::NetworkVersion| json!([v.major, v.minor, v.build]);
    let by_slot = |entries: Vec<(String, Value)>| Value::Object(entries.into_iter().collect());
    json!({
        "seed_name": m.seed_name,
        "generator_version": version(&m.generator_version),
        "minimum_server_version": version(&m.minimum_server_version),
        "minimum_client_versions": by_slot(m.minimum_client_versions.iter()
            .map(|(s, v)| (s.to_string(), version(v))).collect()),
        "tags": m.tags,
        "race_mode": m.race_mode,
        "slots": by_slot(m.slots.iter()
            .map(|(s, slot)| (s.to_string(), serde_json::to_value(slot).unwrap())).collect()),
        "connect_names": by_slot(m.connect_names.iter()
            .map(|(name, (team, slot))| (name.clone(), json!([team, slot]))).collect()),
        "slot_data": by_slot(m.slot_data.iter()
            .map(|(s, data)| (s.to_string(), data.clone())).collect()),
        "locations": by_slot(m.locations.iter().map(|(s, locs)| {
            let locs: Map<String, Value> = locs.iter()
                .map(|(l, p)| (l.to_string(), json!([p.item, p.player, p.flags.0]))).collect();
            (s.to_string(), Value::Object(locs))
        }).collect()),
        "precollected_items": by_slot(m.precollected_items.iter()
            .map(|(s, items)| (s.to_string(), json!(items))).collect()),
        "precollected_hints": by_slot(m.precollected_hints.iter().map(|(s, hints)| {
            let hints: Vec<Value> = hints.iter().map(|h| serde_json::to_value(h).unwrap()).collect();
            (s.to_string(), Value::Array(hints))
        }).collect()),
        "er_hint_data": by_slot(m.er_hint_data.iter().map(|(s, entries)| {
            let entries: Map<String, Value> = entries.iter()
                .map(|(l, e)| (l.to_string(), json!(e))).collect();
            (s.to_string(), Value::Object(entries))
        }).collect()),
        "spheres": m.spheres.iter().map(|sphere| by_slot(sphere.iter()
            .map(|(s, locs)| (s.to_string(), json!(locs))).collect())).collect::<Vec<_>>(),
        "server_options": {
            "password": m.server_options.password,
            "server_password": m.server_options.server_password,
            "hint_cost": m.server_options.hint_cost,
            "location_check_points": m.server_options.location_check_points,
            "release_mode": m.server_options.release_mode,
            "collect_mode": m.server_options.collect_mode,
            "remaining_mode": m.server_options.remaining_mode,
            "countdown_mode": m.server_options.countdown_mode,
            "disable_item_cheat": m.server_options.disable_item_cheat,
            "compatibility": m.server_options.compatibility,
        },
        "datapackage": by_slot(m.datapackage.iter().map(|(game, p)| (game.clone(), json!({
            "item_name_to_id": p.item_name_to_id,
            "location_name_to_id": p.location_name_to_id,
            "item_name_groups": p.item_name_groups,
            "location_name_groups": p.location_name_groups,
            "checksum": p.checksum,
        }))).collect()),
    })
}

/// Les ensembles n'ont pas d'ordre : on trie les listes concernées.
fn sort_unordered(value: &mut Value) {
    for key in ["precollected_hints"] {
        if let Some(Value::Object(slots)) = value.get_mut(key) {
            for list in slots.values_mut() {
                if let Value::Array(items) = list {
                    items.sort_by_key(|v| v.to_string());
                }
            }
        }
    }
    if let Some(Value::Array(spheres)) = value.get_mut("spheres") {
        for sphere in spheres {
            if let Value::Object(slots) = sphere {
                for list in slots.values_mut() {
                    if let Value::Array(items) = list {
                        items.sort_by_key(|v| v.as_i64());
                    }
                }
            }
        }
    }
}

/// Compare le modèle d'une seed avec le JSON calculé par Python, champ par champ.
fn assert_like_python(seed: &Path, expected: &Path) {
    let model = load_archipelago(&fs::read(seed).unwrap(), &ArchiveLimits::default()).unwrap();
    let mut ours = model_json(&model);
    let mut theirs: Value = serde_json::from_str(&fs::read_to_string(expected).unwrap()).unwrap();
    sort_unordered(&mut ours);
    sort_unordered(&mut theirs);
    let (Value::Object(ours), Value::Object(theirs)) = (ours, theirs) else {
        panic!("objets attendus");
    };
    for (key, value) in &theirs {
        assert!(
            ours.get(key) == Some(value),
            "champ « {key} » différent de Python"
        );
    }
    assert_eq!(ours.len(), theirs.len());
}

fn data(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

#[test]
fn seed_mit_comme_python() {
    assert_like_python(&data("seed-mit.archipelago"), &data("seed-mit.modele.json"));
}

#[test]
fn seed_mit_quelques_valeurs() {
    let model = load_archipelago(
        &fs::read(data("seed-mit.archipelago")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    // 3 joueurs + le groupe « Lien » (item link d'Alice et Bob).
    assert_eq!(model.slots.len(), 4);
    let group = &model.slots[&4];
    assert_eq!(group.name, "Lien");
    assert_eq!(group.slot_type, SlotType::GROUP);
    assert_eq!(group.group_members, [1, 2]);
    assert_eq!(model.connect_names["Chloé"], (0, 3));
    // L'indice de départ d'Alice est prioritaire.
    assert!(model.precollected_hints[&1]
        .iter()
        .any(|h| h.status == HintStatus::PRIORITY));
    assert_eq!(model.datapackage.len(), 3);
    assert_eq!(model.server_options.hint_cost, 10);
}

/// Un champ obligatoire absent donne un message qui nomme le champ.
#[test]
fn champ_absent_nomme() {
    // Pickle protocole 4 : {"seed_name": "x"}, compressé avec zlib.
    let pickle = b"\x80\x04}\x8c\x09seed_name\x8c\x01xs.";
    let mut encoder = flate2::write::ZlibEncoder::new(vec![3u8], flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, pickle).unwrap();
    let file = encoder.finish().unwrap();
    match load_archipelago(&file, &ArchiveLimits::default()) {
        Err(LoadError::Model(error)) => {
            assert_eq!(error.path, "racine");
            assert!(error.problem.contains("minimum_versions"), "{error}");
        }
        other => panic!("erreur attendue : {other:?}"),
    }
}

/// Seed hors dépôt, à la demande :
/// `RAFTUP_SEED=… RAFTUP_SEED_MODELE=… cargo test -p multidata -- --ignored`
#[test]
#[ignore = "demande une seed et son modèle attendu hors dépôt"]
fn seed_externe_comme_python() {
    let seed = std::env::var("RAFTUP_SEED").unwrap();
    let expected = std::env::var("RAFTUP_SEED_MODELE").unwrap();
    assert_like_python(Path::new(&seed), Path::new(&expected));
}
