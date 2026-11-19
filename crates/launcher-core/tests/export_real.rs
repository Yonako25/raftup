//! Test RÉEL de l'export des mondes (ADR 0028, étape 1) : vrai Python, vrai Archipelago
//! 0.6.8. APQuest exporté depuis Python doit donner le même monde que le portage Rust :
//! mêmes régions, entrées, lieux, objets, inventaire de départ, datapackage, et les mêmes
//! choses atteignables pour n'importe quel inventaire.
//!
//! Ignoré par défaut ; il ne télécharge rien. Il lui faut un runtime installé
//! (`setup_generator`), dont la racine est dans `RAFTUP_REAL_RUNTIME` :
//!
//! `cargo test -p launcher-core --test export_real -- --ignored --nocapture --test-threads=1`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::type_complexity)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use generate::imported::{build_world, read_export, Imported};
use generate::worlds::apquest::ApQuestOptions;
use generate::{build, Built, GameOptions, GenerateRequest, PlayerRequest};
use launcher_core::generation::{pins, runtime_status, Runtime, RuntimeStatus};
use launcher_core::Paths;
use logic::{MultiWorld, Reachability, SlotId, State};
use py_bridge::{
    BridgeOptions, CancelToken, ExportParams, GenerateParams, PyBridge, EXPORTER_SCRIPT,
};

fn runtime() -> Option<Runtime> {
    let root = PathBuf::from(std::env::var_os("RAFTUP_REAL_RUNTIME").or_else(|| {
        eprintln!("RAFTUP_REAL_RUNTIME absente : test réel sauté");
        None
    })?);
    match runtime_status(&Paths::under(&root), &pins().unwrap()) {
        RuntimeStatus::Ready(runtime) => Some(runtime),
        other => panic!("runtime non prêt dans {} : {other:?}", root.display()),
    }
}

/// Lance `export_worlds` sur ces YAML et relit le JSON.
fn export(runtime: &Runtime, work: &Path, yamls: &[String], seed: u64) -> Imported {
    let players = work.join("players");
    let out = work.join("out");
    std::fs::create_dir_all(&players).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    for entry in std::fs::read_dir(&players).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    for (i, yaml) in yamls.iter().enumerate() {
        std::fs::write(players.join(format!("P{:02}.yaml", i + 1)), yaml).unwrap();
    }
    let mut options = BridgeOptions::new(&runtime.python_exe);
    options.working_dir = Some(runtime.archipelago_dir.clone());
    options
        .env
        .insert("SKIP_REQUIREMENTS_UPDATE".into(), "1".into());
    options
        .env
        .insert("RAFTUP_BLOCK_NETWORK".into(), "1".into());
    let mut bridge = PyBridge::start(options).unwrap();
    let path = out.join("export.json");
    let params = ExportParams {
        generate: GenerateParams {
            archipelago_dir: runtime.archipelago_dir.clone(),
            site_packages: runtime.site_packages.clone(),
            player_files_dir: players,
            output_dir: out,
            seed: Some(seed),
            spoiler: 0,
            race: false,
        },
        export_path: path.clone(),
        exporter: EXPORTER_SCRIPT.to_string(),
    };
    let result = bridge
        .export_worlds_cancellable(&params, Duration::from_secs(300), &CancelToken::new())
        .unwrap();
    let _ = bridge.shutdown();
    assert_eq!(result.ap_version, "0.6.8");
    assert_eq!(
        result.opaque_rules, 0,
        "APQuest n'a que des règles rule_builder"
    );
    let bytes = std::fs::read(&path).unwrap();
    build_world(&read_export(&bytes).unwrap()).unwrap()
}

fn yaml(name: &str, o: &ApQuestOptions) -> String {
    format!(
        "name: {name}\ngame: APQuest\nAPQuest:\n  hard_mode: {}\n  hammer: {}\n  extra_starting_chest: {}\n  start_with_one_confetti_cannon: {}\n  trap_chance: 0\n",
        o.hard_mode, o.hammer, o.extra_starting_chest, o.start_with_one_confetti_cannon
    )
}

/// Ce qui ne dépend pas de l'ordre de création : noms, liens, numéros.
#[derive(Debug, PartialEq, Eq)]
struct Shape {
    regions: BTreeSet<(String, String)>,
    entrances: BTreeSet<(String, String, String, String)>,
    /// (joueur, lieu, numéro, région, objet déjà posé)
    locations: BTreeSet<(String, String, Option<i64>, String, Option<String>)>,
    origins: BTreeSet<(String, String)>,
}

fn slot_name(w: &MultiWorld, slot: SlotId) -> String {
    w.slot(slot).unwrap().name.clone()
}

fn region_name(w: &MultiWorld, id: logic::RegionId) -> String {
    w.region(id).unwrap().name.clone()
}

fn shape(w: &MultiWorld) -> Shape {
    Shape {
        regions: w
            .regions()
            .iter()
            .map(|r| (slot_name(w, r.slot), r.name.clone()))
            .collect(),
        entrances: w
            .entrances()
            .iter()
            .map(|e| {
                (
                    slot_name(w, e.slot),
                    e.name.clone(),
                    region_name(w, e.from),
                    region_name(w, e.to),
                )
            })
            .collect(),
        locations: w
            .locations()
            .iter()
            .map(|l| {
                (
                    slot_name(w, l.slot),
                    l.name.clone(),
                    l.address,
                    region_name(w, l.region),
                    l.item.map(|i| w.item_kind(i).unwrap().name.clone()),
                )
            })
            .collect(),
        origins: w
            .slots()
            .iter()
            .map(|s| (s.name.clone(), region_name(w, s.origin.unwrap())))
            .collect(),
    }
}

/// Multiset (joueur, objet, classification) d'une liste d'objets.
fn items(w: &MultiWorld, ids: &[logic::ItemKindId]) -> BTreeMap<(String, String, String), usize> {
    let mut out = BTreeMap::new();
    for id in ids {
        let kind = w.item_kind(*id).unwrap();
        let key = (
            slot_name(w, kind.slot),
            kind.name.clone(),
            format!("{:?}", kind.classification),
        );
        *out.entry(key).or_insert(0) += 1;
    }
    out
}

/// Inventaire construit par noms : le même dans les deux mondes.
fn state_from(w: &MultiWorld, counts: &BTreeMap<(String, String), u32>) -> State {
    let mut state = State::new(w);
    for ((player, item), n) in counts {
        let slot = w.slot_ids().find(|s| slot_name(w, *s) == *player).unwrap();
        if let Some(id) = w.item_named(slot, item) {
            if w.item_kind(id).unwrap().classification.progression {
                state.add(id, *n);
            }
        }
    }
    state
}

/// Ce qui est atteignable, par noms.
fn reached(w: &MultiWorld, state: &State) -> (BTreeSet<String>, BTreeSet<String>, Vec<bool>) {
    let reach = Reachability::compute(w, state);
    let locations = w
        .location_ids()
        .filter(|id| reach.location(*id))
        .map(|id| w.location(id).unwrap().name.clone())
        .collect();
    let entrances = w
        .entrances()
        .iter()
        .filter(|e| {
            w.entrance_named(e.slot, &e.name)
                .is_some_and(|id| reach.entrance(id))
        })
        .map(|e| e.name.clone())
        .collect();
    let goals = w
        .slots()
        .iter()
        .map(|s| s.goal.eval(w, state, &reach))
        .collect();
    (locations, entrances, goals)
}

fn compare(imported: &Imported, built: &Built, label: &str) {
    let (a, b) = (&imported.world, &built.world);
    assert_eq!(shape(a), shape(b), "{label} : forme du monde");
    assert_eq!(
        items(a, &imported.pool),
        items(b, &built.pool),
        "{label} : pool"
    );
    let start_a: Vec<_> = imported.start_items.iter().flatten().copied().collect();
    let start_b: Vec<_> = built.start_items.iter().flatten().copied().collect();
    assert_eq!(
        items(a, &start_a),
        items(b, &start_b),
        "{label} : inventaire de départ"
    );

    // Noms de tous les objets de progression, des deux côtés.
    let mut names = BTreeSet::new();
    for w in [a, b] {
        for kind in w.item_kinds() {
            if kind.classification.progression {
                names.insert((slot_name(w, kind.slot), kind.name.clone()));
            }
        }
    }
    // 300 inventaires pseudo-aléatoires (générateur congruentiel : pas de dépendance).
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    for round in 0..300 {
        let mut counts = BTreeMap::new();
        for name in &names {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            counts.insert(name.clone(), ((x >> 33) % 3) as u32);
        }
        let ra = reached(a, &state_from(a, &counts));
        let rb = reached(b, &state_from(b, &counts));
        assert_eq!(ra, rb, "{label} : inventaire n° {round} {counts:?}");
    }
}

#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn apquest_exporte_egal_au_portage_rust() {
    let Some(runtime) = runtime() else { return };
    let work = tempfile::tempdir().unwrap();
    for bits in 0u8..16 {
        let options = ApQuestOptions {
            hard_mode: bits & 1 != 0,
            hammer: bits & 2 != 0,
            extra_starting_chest: bits & 4 != 0,
            start_with_one_confetti_cannon: bits & 8 != 0,
            trap_chance: 0,
            ..ApQuestOptions::default()
        };
        let imported = export(&runtime, work.path(), &[yaml("Alice", &options)], 1);
        let built = build(&GenerateRequest {
            seed: 1,
            players: vec![PlayerRequest::new("Alice", GameOptions::ApQuest(options))],
        })
        .unwrap();
        compare(&imported, &built, &format!("options {bits:04b}"));
        assert_eq!(
            imported.packages["APQuest"],
            GameOptions::ApQuest(options).package(),
            "datapackage"
        );
    }
}

#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn deux_joueurs_exportes() {
    let Some(runtime) = runtime() else { return };
    let work = tempfile::tempdir().unwrap();
    let a = ApQuestOptions::default();
    let b = ApQuestOptions {
        hard_mode: true,
        hammer: true,
        ..ApQuestOptions::default()
    };
    let imported = export(
        &runtime,
        work.path(),
        &[yaml("Alice", &a), yaml("Bob", &b)],
        7,
    );
    let built = build(&GenerateRequest {
        seed: 7,
        players: vec![
            PlayerRequest::new("Alice", GameOptions::ApQuest(a)),
            PlayerRequest::new("Bob", GameOptions::ApQuest(b)),
        ],
    })
    .unwrap();
    compare(&imported, &built, "deux joueurs");
}
