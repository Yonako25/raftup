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

use generate::imported::{build_world, read_export, ImportError, Imported};
use generate::worlds::apquest::ApQuestOptions;
use generate::{build, Built, GameOptions, GenerateRequest, PlayerRequest};
use launcher_core::generation::{
    pins, runtime_status, ExportBackend, GenerateRequest as HostRequest, Runtime, RuntimeStatus,
};
use launcher_core::Paths;
use logic::{MultiWorld, Reachability, SlotId, State};
use py_bridge::{
    BridgeOptions, CancelToken, ExportParams, ExportResult, GenerateParams, PyBridge,
    EXPORTER_SCRIPT,
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

/// Lance `export_worlds` sur ces YAML et relit le JSON ; exige zéro règle opaque.
fn export(runtime: &Runtime, work: &Path, yamls: &[String], seed: u64) -> Imported {
    let (result, imported) = export_raw(runtime, work, yamls, seed);
    assert_eq!(result.opaque_rules, 0, "règles opaques");
    imported.unwrap()
}

/// Lance `export_worlds` et rend son résultat et l'import (réussi ou non).
fn export_raw(
    runtime: &Runtime,
    work: &Path,
    yamls: &[String],
    seed: u64,
) -> (ExportResult, Result<Imported, ImportError>) {
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
    let bytes = std::fs::read(&path).unwrap();
    let export = read_export(&bytes).unwrap();
    assert!(export.checks.is_some(), "contrôle croisé absent");
    (result, build_world(&export))
}

/// Installe le monde de test `raftup_sonde` (tests/data) dans `custom_worlds` du runtime de
/// test, sous forme d'`.apworld`. Pas de suppression ensuite : chaque génération du
/// launcher vide ce dossier (ADR 0017, décision 8).
fn install_sonde(runtime: &Runtime) {
    let dir = runtime.archipelago_dir.join("custom_worlds");
    std::fs::create_dir_all(&dir).unwrap();
    write_sonde_apworld(&dir.join("raftup_sonde.apworld"));
}

/// Écrit l'`.apworld` du monde de test à cet endroit.
fn write_sonde_apworld(path: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/raftup_sonde");
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for name in ["__init__.py", "archipelago.json"] {
        zip.start_file(format!("raftup_sonde/{name}"), options)
            .unwrap();
        std::io::Write::write_all(&mut zip, &std::fs::read(source.join(name)).unwrap()).unwrap();
    }
    zip.finish().unwrap();
}

fn sonde_yaml(bad_rule: bool) -> String {
    format!("name: Sam\ngame: Raftup Sonde\nRaftup Sonde:\n  bad_rule: {bad_rule}\n")
}

#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn lambdas_sondees_et_controlees() {
    let Some(runtime) = runtime() else { return };
    install_sonde(&runtime);
    let work = tempfile::tempdir().unwrap();
    let (result, imported) = export_raw(&runtime, work.path(), &[sonde_yaml(false)], 3);
    assert_eq!(result.opaque_rules, 0);
    // 3 entrées, 5 lieux à règle (dont « Victory ») et la condition de victoire.
    assert_eq!(result.probed_rules, 9, "{result:?}");
    // L'import a passé le contrôle croisé sur les 64 inventaires du vrai Archipelago.
    let imported = imported.unwrap();
    let w = &imported.world;
    let sam = w.slot_ids().next().unwrap();
    let id = |n: &str| w.item_named(sam, n).unwrap();
    let reach = |items: &[(&str, u32)], location: &str| {
        let mut state = State::new(w);
        for (name, n) in items {
            state.add(id(name), *n);
        }
        Reachability::compute(w, &state).location(w.location_named(sam, location).unwrap())
    };
    // Cave : Sword puis Lamp, ou Sword puis 2 Gem.
    assert!(!reach(&[("Sword", 1)], "Cave Pool"));
    assert!(reach(&[("Sword", 1), ("Lamp", 1)], "Cave Pool"));
    assert!(reach(&[("Sword", 1), ("Gem", 2)], "Cave Pool"));
    // Castle : Sword et 2 Key (count >= 2).
    assert!(!reach(&[("Sword", 1), ("Key", 1)], "Castle Gate"));
    assert!(reach(&[("Sword", 1), ("Key", 2)], "Castle Gate"));
    // Groupe « Shiny » (Gem, Lamp) : 3 au total.
    assert!(reach(
        &[("Sword", 1), ("Gem", 2), ("Lamp", 1)],
        "Cave Crystal"
    ));
    assert!(!reach(&[("Sword", 1), ("Gem", 2)], "Cave Crystal"));
    // Lieux prioritaire et exclu, posés par le monde dans `set_rules`.
    let progress = |n: &str| {
        w.location(w.location_named(sam, n).unwrap())
            .unwrap()
            .progress
    };
    assert_eq!(progress("Start Chest"), logic::Progress::Priority);
    assert_eq!(progress("Forest Stump"), logic::Progress::Excluded);
    assert_eq!(progress("Cave Pool"), logic::Progress::Default);
    // Tout le pool en main : tout s'ouvre, victoire comprise.
    let mut state = imported.start.clone();
    for item in &imported.pool {
        state.collect(w, *item);
    }
    let result = logic::sweep(w, state);
    assert!(result.unreachable_locations(w).is_empty());
    assert!(result.all_goals_reached(w));
}

#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn regle_non_monotone_refusee() {
    let Some(runtime) = runtime() else { return };
    install_sonde(&runtime);
    let work = tempfile::tempdir().unwrap();
    let (result, imported) = export_raw(&runtime, work.path(), &[sonde_yaml(true)], 3);
    assert_eq!(result.opaque_rules, 1);
    match imported {
        Err(ImportError::Unsupported { reason, .. }) => {
            assert!(
                reason.contains("Bad Spot") && reason.contains("non monotone"),
                "{reason}"
            );
        }
        other => panic!("attendu Unsupported : {other:?}"),
    }
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

/// Génération complète par le moteur « export » (ADR 0028, étape 4) : seed écrite par
/// Archipelago, placement fait par Raftup.
fn generate_with_export(
    runtime: &Runtime,
    work: &Path,
    yamls: &[(&str, String)],
    seed: u64,
) -> (launcher_core::generation::GenerateOutput, Vec<String>) {
    let (output, log) = try_generate_with_export(runtime, work, yamls, seed);
    (output.unwrap(), log)
}

/// Comme [`generate_with_export`], sans exiger le succès.
fn try_generate_with_export(
    runtime: &Runtime,
    work: &Path,
    yamls: &[(&str, String)],
    seed: u64,
) -> (
    Result<launcher_core::generation::GenerateOutput, launcher_core::GenerationError>,
    Vec<String>,
) {
    let apworld = work.join("raftup_sonde.apworld");
    write_sonde_apworld(&apworld);
    let mut backend = ExportBackend::new(runtime.clone());
    run_generation(&mut backend, work, yamls, vec![apworld], seed)
}

/// Une génération complète par `generate` du launcher, avec le moteur donné.
fn run_generation(
    backend: &mut dyn launcher_core::generation::GeneratorBackend,
    work: &Path,
    yamls: &[(&str, String)],
    apworlds: Vec<PathBuf>,
    seed: u64,
) -> (
    Result<launcher_core::generation::GenerateOutput, launcher_core::GenerationError>,
    Vec<String>,
) {
    let data = Paths::under(&PathBuf::from(
        std::env::var_os("RAFTUP_REAL_RUNTIME").unwrap(),
    ));
    let player_files = yamls
        .iter()
        .map(|(name, text)| {
            let path = work.join(name);
            std::fs::write(&path, text).unwrap();
            path
        })
        .collect();
    let request = HostRequest {
        player_files,
        apworlds,
        seed: Some(seed),
        timeout: Duration::from_secs(300),
        ..HostRequest::default()
    };
    let (log, lines) = std::sync::mpsc::channel();
    let output =
        launcher_core::generation::generate(&data, backend, &request, &CancelToken::new(), log);
    (output, lines.try_iter().collect())
}

#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn seed_complete_placee_par_raftup() {
    let Some(runtime) = runtime() else { return };
    let work = tempfile::tempdir().unwrap();
    let apquest = yaml("Alice", &ApQuestOptions::default());
    let yamls = [("sam.yaml", sonde_yaml(false)), ("alice.yaml", apquest)];
    let (output, log) = generate_with_export(&runtime, work.path(), &yamls, 99);
    assert!(
        log.iter()
            .any(|l| l.contains("Placement des objets par Raftup")),
        "{log:#?}"
    );
    assert!(
        !log.iter().any(|l| l.contains("Archipelago génère seul")),
        "{log:#?}"
    );
    assert_eq!(output.summary.players, 2);
    // La seed d'Archipelago est lisible par Raftup : chaque lieu a son objet.
    let bytes = std::fs::read(&output.archipelago_file).unwrap();
    let md = multidata::load_archipelago(&bytes, &multidata::ArchiveLimits::default()).unwrap();
    let placed: usize = md.locations.values().map(|l| l.len()).sum();
    assert_eq!(placed, 8 + 6, "lieux de Sam (8) et d'Alice (6)");
    // Même seed, mêmes fichiers : même placement (PYTHONHASHSEED=0, ChaCha).
    let work2 = tempfile::tempdir().unwrap();
    let (again, _) = generate_with_export(&runtime, work2.path(), &yamls, 99);
    let bytes2 = std::fs::read(&again.archipelago_file).unwrap();
    let md2 = multidata::load_archipelago(&bytes2, &multidata::ArchiveLimits::default()).unwrap();
    assert_eq!(md.locations, md2.locations);
    assert_eq!(md.slot_data, md2.slot_data);
}

#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn repli_sur_archipelago_seul() {
    let Some(runtime) = runtime() else { return };
    // Le placement d'Archipelago lui-même échoue sur certaines seeds de ce petit monde
    // (2 lieux ouverts au départ) : on essaie quelques seeds. Le repli doit avoir lieu à
    // chaque fois ; on vérifie la seed sur la première qu'Archipelago réussit.
    for seed in 1..=20 {
        let work = tempfile::tempdir().unwrap();
        let apworld = work.path().join("raftup_sonde.apworld");
        write_sonde_apworld(&apworld);
        let mut backend = ExportBackend::new(runtime.clone());
        let (output, log) = run_generation(
            &mut backend,
            work.path(),
            &[("sam.yaml", sonde_yaml(true))],
            vec![apworld],
            seed,
        );
        assert!(
            log.iter()
                .any(|l| l.contains("Archipelago génère seul") && l.contains("Bad Spot")),
            "seed {seed} : {log:#?}"
        );
        // Le rapport de fin dit le repli et sa raison (montrés par l'interface).
        use launcher_core::generation::{EngineUsed, GeneratorBackend};
        let report = backend.report();
        assert_eq!(report.engine, EngineUsed::Python);
        assert!(
            report
                .fallback
                .as_deref()
                .is_some_and(|r| r.contains("Bad Spot")),
            "{report:?}"
        );
        if let Ok(output) = output {
            assert_eq!(output.summary.players, 1);
            return;
        }
    }
    panic!("Archipelago n'a réussi aucune des 20 seeds");
}

/// L'apworld Slime Rancher installée sur le PC (`RAFTUP_SLIME_APWORLD`), jamais dans le
/// dépôt (elle n'a pas de licence, ADR 0021) ; `None` si la variable manque.
fn slime_apworld() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("RAFTUP_SLIME_APWORLD").or_else(|| {
        eprintln!("RAFTUP_SLIME_APWORLD absente : test Slime Rancher sauté");
        None
    })?);
    assert!(path.is_file(), "{} introuvable", path.display());
    Some(path)
}

const SLIME_YAML: &str = "name: Beatrix\ngame: Slime Rancher\nSlime Rancher: {}\n";

#[test]
#[ignore = "demande RAFTUP_REAL_RUNTIME et RAFTUP_SLIME_APWORLD"]
fn slime_rancher_exporte() {
    let Some(runtime) = runtime() else { return };
    let Some(apworld) = slime_apworld() else {
        return;
    };
    let dir = runtime.archipelago_dir.join("custom_worlds");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(&apworld, dir.join("slime_rancher.apworld")).unwrap();
    let work = tempfile::tempdir().unwrap();
    let (result, imported) = export_raw(&runtime, work.path(), &[SLIME_YAML.to_string()], 11);
    eprintln!("{result:?}");
    // Raisons des règles non traduites, regroupées (diagnostic).
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&result.export).unwrap()).unwrap();
    let mut whys = BTreeMap::new();
    collect_whys(&json, &mut whys);
    for (why, n) in &whys {
        eprintln!("opaque ×{n} : {why}");
    }
    let imported = match imported {
        Ok(imported) => imported,
        Err(e) => panic!("import refusé : {e}"),
    };
    assert_eq!(result.opaque_rules, 0);
    let w = &imported.world;
    eprintln!(
        "{} régions, {} entrées, {} lieux, {} objets dans le pool",
        w.regions().len(),
        w.entrances().len(),
        w.locations().len(),
        imported.pool.len()
    );
}

/// Raisons (`why`) des règles opaques d'un export, comptées.
fn collect_whys(value: &serde_json::Value, out: &mut BTreeMap<String, usize>) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("t").and_then(|t| t.as_str()) == Some("opaque") {
                let why = map.get("why").and_then(|w| w.as_str()).unwrap_or("?");
                // Le nom de la lambda varie peu ; la raison est entre parenthèses.
                let reason = why.rsplit_once(" (").map(|(_, r)| r).unwrap_or(why);
                *out.entry(reason.to_string()).or_insert(0) += 1;
            }
            for v in map.values() {
                collect_whys(v, out);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                collect_whys(v, out);
            }
        }
        _ => {}
    }
}

/// Slime Rancher généré de bout en bout par le moteur « export », comparé à la même seed
/// générée par Archipelago seul : même datapackage, mêmes lieux, même inventaire de
/// départ, même `slot_data` ; seul le placement diffère.
#[test]
#[ignore = "demande RAFTUP_REAL_RUNTIME et RAFTUP_SLIME_APWORLD"]
fn slime_rancher_genere_par_export() {
    let Some(runtime) = runtime() else { return };
    let Some(apworld) = slime_apworld() else {
        return;
    };
    let yamls = [("beatrix.yaml", SLIME_YAML.to_string())];
    let load = |output: &launcher_core::generation::GenerateOutput| {
        let bytes = std::fs::read(&output.archipelago_file).unwrap();
        multidata::load_archipelago(&bytes, &multidata::ArchiveLimits::default()).unwrap()
    };

    let work = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    let mut export = ExportBackend::new(runtime.clone());
    let (output, log) = run_generation(&mut export, work.path(), &yamls, vec![apworld.clone()], 11);
    let output = output.unwrap();
    let export_time = started.elapsed();
    assert!(
        log.iter()
            .any(|l| l.contains("Placement des objets par Raftup")),
        "{log:#?}"
    );
    assert!(
        !log.iter().any(|l| l.contains("Archipelago génère seul")),
        "{log:#?}"
    );
    let ours = load(&output);

    let work2 = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    let mut python = launcher_core::generation::PythonBackend::new(runtime.clone());
    let (official, _) = run_generation(&mut python, work2.path(), &yamls, vec![apworld], 11);
    let python_time = started.elapsed();
    let official = load(&official.unwrap());
    eprintln!("export : {export_time:?}, Archipelago seul : {python_time:?}");

    assert_eq!(ours.seed_name, official.seed_name);
    assert_eq!(ours.datapackage, official.datapackage);
    assert_eq!(ours.slot_data, official.slot_data, "slot_data");
    assert_eq!(ours.precollected_items, official.precollected_items);
    let keys = |md: &multidata::MultiData| {
        md.locations
            .iter()
            .map(|(slot, l)| (*slot, l.keys().copied().collect::<Vec<_>>()))
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(&ours), keys(&official), "mêmes lieux");
    // Même pool : les mêmes objets, placés autrement.
    let pool = |md: &multidata::MultiData| {
        let mut items: Vec<_> = md
            .locations
            .values()
            .flat_map(|l| l.values().map(|p| (p.player, p.item)))
            .collect();
        items.sort();
        items
    };
    assert_eq!(pool(&ours), pool(&official), "même pool");
    assert_ne!(
        ours.locations, official.locations,
        "placement identique : suspect"
    );
}
