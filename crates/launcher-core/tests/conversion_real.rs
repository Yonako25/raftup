//! Tests RÉELS de la conversion apworld ↔ monde natif (ADR 0042) : vrai Python, vrai
//! Archipelago 0.6.8.
//!
//! Ignorés par défaut ; ils ne téléchargent rien. Il leur faut un runtime installé
//! (`setup_generator`), dont la racine est dans `RAFTUP_REAL_RUNTIME` :
//!
//! `cargo test -p launcher-core --test conversion_real -- --ignored --nocapture --test-threads=1`

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use launcher_core::generation::{pins, runtime_status, Runtime, RuntimeStatus};
use launcher_core::Paths;
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

fn bridge(runtime: &Runtime) -> PyBridge {
    let mut options = BridgeOptions::new(&runtime.python_exe);
    options.working_dir = Some(runtime.archipelago_dir.clone());
    options
        .env
        .insert("SKIP_REQUIREMENTS_UPDATE".into(), "1".into());
    options
        .env
        .insert("RAFTUP_BLOCK_NETWORK".into(), "1".into());
    PyBridge::start(options).unwrap()
}

/// Écrit les YAML dans `work/players` et rend les paramètres de génération.
fn params(runtime: &Runtime, work: &Path, yamls: &[String], seed: u64) -> GenerateParams {
    let players = work.join("players");
    let out = work.join("out");
    std::fs::create_dir_all(&players).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    for (i, yaml) in yamls.iter().enumerate() {
        std::fs::write(players.join(format!("P{:02}.yaml", i + 1)), yaml).unwrap();
    }
    GenerateParams {
        archipelago_dir: runtime.archipelago_dir.clone(),
        site_packages: runtime.site_packages.clone(),
        player_files_dir: players,
        output_dir: out,
        seed: Some(seed),
        spoiler: 0,
        race: false,
    }
}

/// Lance `export_worlds` et rend les octets du JSON.
fn export_bytes(runtime: &Runtime, work: &Path, yamls: &[String], seed: u64) -> Vec<u8> {
    let generate = params(runtime, work, yamls, seed);
    let path = generate.output_dir.join("export.json");
    let params = ExportParams {
        generate,
        export_path: path.clone(),
        exporter: EXPORTER_SCRIPT.to_string(),
    };
    let mut bridge = bridge(runtime);
    let result = bridge
        .export_worlds_cancellable(&params, Duration::from_secs(300), &CancelToken::new())
        .unwrap();
    let _ = bridge.shutdown();
    assert_eq!(result.ap_version, "0.6.8");
    assert_eq!(result.opaque_rules, 0, "règles opaques");
    std::fs::read(&path).unwrap()
}

/// Sauvegarde l'export d'APQuest (options par défaut) là où `RAFTUP_SAVE_EXPORT` le dit :
/// c'est ainsi qu'a été écrit `generate/tests/data/apquest-export.json`.
#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn sauver_export_apquest() {
    let Some(runtime) = runtime() else { return };
    let Some(target) = std::env::var_os("RAFTUP_SAVE_EXPORT") else {
        eprintln!("RAFTUP_SAVE_EXPORT absente : rien à écrire");
        return;
    };
    let work = tempfile::tempdir().unwrap();
    let yaml = "name: Alice\ngame: APQuest\nAPQuest: {}\n".to_string();
    let bytes = export_bytes(&runtime, work.path(), &[yaml], 1);
    std::fs::write(target, bytes).unwrap();
}

// ---- Export inverse : monde natif → apworld (ADR 0042, § 4) ----

const APQUEST_NATIF: &str = include_str!("../../../spec/monde-natif/exemples/apquest/world.toml");

/// APQuest natif, renommé : le vrai APQuest est déjà dans Archipelago, et deux mondes ne
/// peuvent pas porter le même nom de jeu.
fn apquest_natif() -> worlds_native::NativeWorld {
    let mut world = worlds_native::parse_world(APQUEST_NATIF).unwrap();
    world.game = "APQuest Natif".into();
    world
}

/// Écrit l'apworld du monde dans `custom_worlds` du runtime ; rend son chemin.
fn install_apworld(runtime: &Runtime, world: &worlds_native::NativeWorld) -> PathBuf {
    let config = worlds_native::apworld::ApworldConfig::for_world(world);
    let files = worlds_native::apworld::to_apworld(world, &config).unwrap();
    let dir = runtime.archipelago_dir.join("custom_worlds");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.apworld", config.module));
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for f in &files {
        zip.start_file(format!("{}/{}", config.module, f.name), options)
            .unwrap();
        std::io::Write::write_all(&mut zip, f.contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    path
}

/// Le `.archipelago` contenu dans le zip produit par Archipelago.
fn archipelago_in(zip_path: &Path) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(zip_path).unwrap()).unwrap();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        if entry.name().unwrap().ends_with(".archipelago") {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
            return bytes;
        }
    }
    panic!("pas de .archipelago dans {}", zip_path.display());
}

fn natif_yamls() -> Vec<String> {
    vec![
        "name: Alice\ngame: APQuest Natif\nAPQuest Natif: {}\n".to_string(),
        "name: Bob\ngame: APQuest Natif\nAPQuest Natif:\n  hard_mode: true\n  hammer: true\n  extra_starting_chest: true\n  start_with_one_confetti_cannon: true\n  trap_chance: 50\n  confetti_explosiveness: 7\n  player_sprite: kitty\n".to_string(),
    ]
}

/// L'apworld écrit depuis APQuest natif tourne dans le vrai Archipelago : la seed se
/// génère, elle a le même datapackage que le monde natif (checksum compris) et le même
/// `slot_data` que le moteur Rust pour les mêmes options.
#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn apworld_natif_genere_par_archipelago() {
    let Some(runtime) = runtime() else { return };
    let world = std::sync::Arc::new(apquest_natif());
    let installed = install_apworld(&runtime, &world);
    let work = tempfile::tempdir().unwrap();
    let params = params(&runtime, work.path(), &natif_yamls(), 5);
    let mut bridge = bridge(&runtime);
    let result = bridge.generate(&params, Duration::from_secs(300));
    let _ = bridge.shutdown();
    let result = result.unwrap();
    let bytes = archipelago_in(&result.zip);
    let md = multidata::load_archipelago(&bytes, &multidata::ArchiveLimits::default()).unwrap();

    // Même datapackage que le monde natif.
    let native = generate::NativeOptions::new(world.clone()).package();
    assert_eq!(md.datapackage["APQuest Natif"], native);
    // Mêmes noms et numéros que le vrai APQuest (seul le nom du jeu change).
    let apquest = generate::GameOptions::ApQuest(Default::default()).package();
    assert_eq!(native.item_name_to_id, apquest.item_name_to_id);
    assert_eq!(native.location_name_to_id, apquest.location_name_to_id);

    // Seed jouable : tous les lieux de Bob remplis (coffre en plus : 7 lieux),
    // et son slot_data est celui que donne le moteur Rust pour ces options.
    let bob = md.connect_names["Bob"].1;
    assert_eq!(md.locations[&bob].len(), 7);
    let mut options = generate::NativeOptions::new(world.clone());
    for (name, value) in [
        ("hard_mode", 1),
        ("hammer", 1),
        ("extra_starting_chest", 1),
        ("start_with_one_confetti_cannon", 1),
        ("trap_chance", 50),
        ("confetti_explosiveness", 7),
        ("player_sprite", 3),
    ] {
        options.set(name, value).unwrap();
    }
    let request = generate::GenerateRequest::new(
        5,
        vec![generate::PlayerRequest::new(
            "Bob",
            generate::GameOptions::Native(options),
        )],
    );
    let built = generate::build(&request).unwrap();
    assert_eq!(md.slot_data[&bob], built.slot_data[0]);
    std::fs::remove_file(installed).unwrap();
}

/// Aller-retour : l'apworld écrit depuis le monde natif est exporté par le worker
/// (règles en lambdas, donc sondées), passe le contrôle croisé contre le vrai
/// Archipelago, puis se reconvertit en monde natif lisible.
#[test]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
fn apworld_natif_aller_retour() {
    let Some(runtime) = runtime() else { return };
    let world = apquest_natif();
    let installed = install_apworld(&runtime, &world);
    let work = tempfile::tempdir().unwrap();
    let bytes = export_bytes(&runtime, work.path(), &natif_yamls(), 5);
    let export = generate::imported::read_export(&bytes).unwrap();
    // Le contrôle croisé (64 inventaires, vrai Archipelago) est fait par l'import.
    let imported = generate::imported::build_world(&export).unwrap();
    assert_eq!(imported.world.slots().len(), 2);
    for player in [1, 2] {
        let back = generate::convert::convert(&export, player, None).unwrap();
        let again = worlds_native::parse_world(&back.toml).unwrap();
        assert_eq!(again.game, "APQuest Natif");
    }
    std::fs::remove_file(installed).unwrap();
}

// ---- Slime Rancher (ADR 0042, § 6) : converti sur le PC, jamais dans le dépôt ----

/// L'apworld Slime Rancher installée sur le PC (`RAFTUP_SLIME_APWORLD`). Elle n'a pas de
/// licence (ADR 0021) : sa version native est une œuvre dérivée, qui ne sort pas du PC.
fn slime_apworld() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("RAFTUP_SLIME_APWORLD").or_else(|| {
        eprintln!("RAFTUP_SLIME_APWORLD absente : test Slime Rancher sauté");
        None
    })?);
    assert!(path.is_file(), "{} introuvable", path.display());
    Some(path)
}

/// Slime Rancher (147 règles sondées) converti en monde natif : relu, compilé, et généré
/// par le moteur Rust seul (sans Python). Même datapackage et même `slot_data` que la
/// seed d'Archipelago, mêmes lieux, entrées et pool que l'import de l'export, tout
/// atteignable avec le pool. Le `world.toml` (et l'export) ne sont écrits que si
/// `RAFTUP_SLIME_NATIF_OUT` (et `RAFTUP_SLIME_EXPORT_OUT`) donnent un chemin, hors du dépôt.
#[test]
#[ignore = "demande RAFTUP_REAL_RUNTIME et RAFTUP_SLIME_APWORLD"]
fn slime_rancher_converti() {
    use std::collections::BTreeSet;
    let Some(runtime) = runtime() else { return };
    let Some(apworld) = slime_apworld() else {
        return;
    };
    let dir = runtime.archipelago_dir.join("custom_worlds");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(&apworld, dir.join("slime_rancher.apworld")).unwrap();
    let yaml = vec!["name: Beatrix\ngame: Slime Rancher\nSlime Rancher: {}\n".to_string()];

    // La seed d'Archipelago seul (même YAML, même graine) : son slot_data et son
    // datapackage servent de référence.
    let work = tempfile::tempdir().unwrap();
    let params = params(&runtime, &work.path().join("ap"), &yaml, 11);
    let mut b = bridge(&runtime);
    let result = b.generate(&params, Duration::from_secs(300));
    let _ = b.shutdown();
    let md = multidata::load_archipelago(
        &archipelago_in(&result.unwrap().zip),
        &multidata::ArchiveLimits::default(),
    )
    .unwrap();

    let bytes = export_bytes(&runtime, &work.path().join("export"), &yaml, 11);
    if let Some(out) = std::env::var_os("RAFTUP_SLIME_EXPORT_OUT") {
        std::fs::write(out, &bytes).unwrap();
    }
    let export = generate::imported::read_export(&bytes).unwrap();
    let imported = generate::imported::build_world(&export).unwrap();
    let conversion = generate::convert::convert(&export, 1, Some(&md.slot_data[&1])).unwrap();
    eprintln!("{} points à vérifier :", conversion.notes.len());
    for note in &conversion.notes {
        eprintln!("- {}", note.chars().take(300).collect::<String>());
    }
    if let Some(out) = std::env::var_os("RAFTUP_SLIME_NATIF_OUT") {
        std::fs::write(out, &conversion.toml).unwrap();
    }

    let world = std::sync::Arc::new(worlds_native::parse_world(&conversion.toml).unwrap());
    let options = generate::NativeOptions::new(world);
    assert_eq!(options.package(), md.datapackage["Slime Rancher"]);
    let request = generate::GenerateRequest::new(
        11,
        vec![generate::PlayerRequest::new(
            "Beatrix",
            generate::GameOptions::Native(options),
        )],
    );
    let built = generate::build(&request).unwrap();
    assert_eq!(built.slot_data[0], md.slot_data[&1]);

    // Mêmes éléments que l'import de l'export (moteur « export »).
    let names = |w: &logic::MultiWorld| {
        let locations: BTreeSet<(String, Option<i64>)> = w
            .locations()
            .iter()
            .map(|l| (l.name.clone(), l.address))
            .collect();
        let entrances: BTreeSet<String> = w.entrances().iter().map(|e| e.name.clone()).collect();
        let regions: BTreeSet<String> = w.regions().iter().map(|r| r.name.clone()).collect();
        (locations, entrances, regions)
    };
    assert_eq!(names(&built.world), names(&imported.world));
    let pool = |w: &logic::MultiWorld, pool: &[logic::ItemKindId]| {
        let mut names: Vec<String> = pool
            .iter()
            .map(|i| w.item_kind(*i).unwrap().name.clone())
            .collect();
        names.sort();
        names
    };
    assert_eq!(
        pool(&built.world, &built.pool),
        pool(&imported.world, &imported.pool)
    );

    // Seed complète par le moteur Rust seul : tout se place, la victoire est atteinte.
    let generated = generate::generate(&request).unwrap();
    let seed =
        multidata::load_archipelago(&generated.archipelago, &multidata::ArchiveLimits::default())
            .unwrap();
    assert_eq!(seed.locations[&1].len(), md.locations[&1].len());
    eprintln!(
        "Slime Rancher natif : {} lieux, {} entrées, {} régions",
        built.world.locations().len(),
        built.world.entrances().len(),
        built.world.regions().len()
    );
}
