//! Inventaire de conversion (test RÉEL, lancé à la main) : chaque monde intégré
//! d'Archipelago 0.6.8 passe, avec les options par défaut, par l'export du worker
//! (ADR 0028), l'import et son contrôle croisé, la conversion en monde natif (ADR 0042)
//! et une génération par le moteur Rust seul. Le résultat sert à
//! `docs/inventaire-conversion.md`.
//!
//! Il ne télécharge rien et n'écrit rien dans le dépôt. Variables :
//! - `RAFTUP_REAL_RUNTIME` : racine d'un runtime installé (`setup_generator`) ;
//! - `RAFTUP_INVENTAIRE_JEUX` : fichier texte, un nom de jeu par ligne (liste mesurée :
//!   `tests/data/jeux-archipelago-0.6.8.txt`) ;
//! - `RAFTUP_INVENTAIRE_OUT` : fichier JSON lignes écrit au fil des jeux ;
//! - `RAFTUP_INVENTAIRE_ALLER_RETOUR` (facultatif) : jeux, séparés par `|`, dont le
//!   monde converti est réexporté en apworld puis repasse le contrôle croisé ;
//! - `RAFTUP_INVENTAIRE_NATIFS` (facultatif) : dossier où écrire les `world.toml`
//!   produits (hors du dépôt : ce sont des œuvres dérivées des apworlds).
//!
//! `cargo test -p launcher-core --test inventaire_real -- --ignored --nocapture --test-threads=1`

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use launcher_core::generation::{pins, runtime_status, Runtime, RuntimeStatus};
use launcher_core::Paths;
use py_bridge::{
    BridgeOptions, CancelToken, ExportParams, GenerateParams, PyBridge, EXPORTER_SCRIPT,
};
use serde_json::{json, Value};

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

fn bridge(runtime: &Runtime) -> Result<PyBridge, String> {
    let mut options = BridgeOptions::new(&runtime.python_exe);
    options.working_dir = Some(runtime.archipelago_dir.clone());
    options
        .env
        .insert("SKIP_REQUIREMENTS_UPDATE".into(), "1".into());
    options
        .env
        .insert("RAFTUP_BLOCK_NETWORK".into(), "1".into());
    PyBridge::start(options).map_err(|e| e.to_string())
}

/// Le YAML d'un joueur avec les options par défaut. Le nom du jeu est écrit en chaîne
/// JSON (qui est aussi du YAML valide) : « Jak and Daxter: … » contient « : ».
fn default_yaml(game: &str) -> String {
    let quoted = serde_json::to_string(game).unwrap();
    format!("name: P1\ngame: {quoted}\n{quoted}: {{}}\n")
}

/// Lance `export_worlds` dans un worker neuf ; rend les octets du JSON et le nombre de
/// règles sondées, ou le message d'erreur.
fn export(runtime: &Runtime, work: &Path, yaml: &str, seed: u64) -> Result<(Vec<u8>, u64), String> {
    let players = work.join("players");
    let out = work.join("out");
    std::fs::create_dir_all(&players).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    std::fs::write(players.join("P01.yaml"), yaml).map_err(|e| e.to_string())?;
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
    let mut bridge = bridge(runtime)?;
    let result =
        bridge.export_worlds_cancellable(&params, Duration::from_secs(900), &CancelToken::new());
    let _ = bridge.shutdown();
    let result = result.map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    Ok((bytes, result.probed_rules))
}

/// Raisons des règles opaques de l'export (`{"t": "opaque", "why": …}`), où qu'elles soient.
fn opaque_reasons(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if map.get("t").and_then(Value::as_str) == Some("opaque") {
                let why = map.get("why").and_then(Value::as_str).unwrap_or("?");
                out.push(why.chars().take(200).collect());
            }
            for v in map.values() {
                opaque_reasons(v, out);
            }
        }
        Value::Array(items) => {
            for v in items {
                opaque_reasons(v, out);
            }
        }
        _ => {}
    }
}

/// Coupe un message trop long pour le rapport.
fn short(text: impl std::fmt::Display) -> String {
    text.to_string().chars().take(400).collect()
}

/// Le monde converti, réexporté en apworld (renommé « … Natif » pour ne pas heurter le
/// monde intégré), exporté par le worker puis importé : le contrôle croisé de l'import
/// compare ses règles au vrai Archipelago.
fn round_trip(runtime: &Runtime, toml: &str) -> Result<(), String> {
    let mut world = worlds_native::parse_world(toml).map_err(short)?;
    world.game = format!("{} Natif", world.game);
    let config = worlds_native::apworld::ApworldConfig::for_world(&world);
    let files = worlds_native::apworld::to_apworld(&world, &config).map_err(short)?;
    let dir = runtime.archipelago_dir.join("custom_worlds");
    std::fs::create_dir_all(&dir).map_err(short)?;
    let installed = dir.join(format!("{}.apworld", config.module));
    {
        let file = std::fs::File::create(&installed).map_err(short)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for f in &files {
            zip.start_file(format!("{}/{}", config.module, f.name), options)
                .map_err(short)?;
            zip.write_all(f.contents.as_bytes()).map_err(short)?;
        }
        zip.finish().map_err(short)?;
    }
    let work = tempfile::tempdir().map_err(short)?;
    let result = export(runtime, work.path(), &default_yaml(&world.game), 3).and_then(|(b, _)| {
        let export = generate::imported::read_export(&b).map_err(short)?;
        generate::imported::build_world(&export).map_err(short)?;
        Ok(())
    });
    let _ = std::fs::remove_file(&installed);
    result
}

/// Passe un jeu par toute la chaîne ; rend une ligne du rapport.
fn inventory_one(runtime: &Runtime, game: &str, round_trip_games: &[String]) -> Value {
    let started = Instant::now();
    let mut row = json!({ "game": game });
    let work = tempfile::tempdir().unwrap();
    let (bytes, probed) = match export(runtime, work.path(), &default_yaml(game), 1) {
        Ok(ok) => ok,
        Err(e) => {
            row["stage"] = json!("export");
            row["error"] = json!(short(e));
            row["seconds"] = json!(started.elapsed().as_secs_f64());
            return row;
        }
    };
    row["probed_rules"] = json!(probed);
    if let Ok(raw) = serde_json::from_slice::<Value>(&bytes) {
        let mut reasons = Vec::new();
        opaque_reasons(&raw, &mut reasons);
        row["opaque_rules"] = json!(reasons.len());
        reasons.sort();
        reasons.dedup();
        reasons.truncate(8);
        row["opaque_samples"] = json!(reasons);
        // Drapeaux par joueur que le convertisseur refuse ou signale.
        if let Some(p) = raw.get("players").and_then(|p| p.get(0)) {
            row["custom_collect"] = p.get("custom_collect").cloned().unwrap_or(Value::Null);
            row["early_items"] = p.get("early_items").cloned().unwrap_or(Value::Null);
            row["locations"] = json!(p
                .get("locations")
                .and_then(Value::as_array)
                .map(|a| a.len()));
        }
    }
    let export = match generate::imported::read_export(&bytes) {
        Ok(export) => export,
        Err(e) => {
            row["stage"] = json!("read_export");
            row["error"] = json!(short(e));
            return row;
        }
    };
    // Import et contrôle croisé (moteur « export ») : indépendant de la conversion.
    row["import"] = match generate::imported::build_world(&export) {
        Ok(_) => json!("ok"),
        Err(e) => json!(short(e)),
    };
    let conversion = match generate::convert::convert(&export, 1, None) {
        Ok(c) => c,
        Err(e) => {
            row["stage"] = json!("convert");
            row["error"] = json!(short(e));
            row["seconds"] = json!(started.elapsed().as_secs_f64());
            return row;
        }
    };
    row["notes"] = json!(conversion
        .notes
        .iter()
        .map(|n| n.chars().take(300).collect::<String>())
        .collect::<Vec<_>>());
    if let Some(dir) = std::env::var_os("RAFTUP_INVENTAIRE_NATIFS") {
        let file = PathBuf::from(dir).join(format!(
            "{}.toml",
            worlds_native::apworld::ApworldConfig::for_world(
                &worlds_native::parse_world(&conversion.toml).unwrap()
            )
            .module
        ));
        let _ = std::fs::write(file, &conversion.toml);
    }
    // Génération par le moteur Rust seul, sans Python.
    let world = match worlds_native::parse_world(&conversion.toml) {
        Ok(w) => std::sync::Arc::new(w),
        Err(e) => {
            row["stage"] = json!("parse");
            row["error"] = json!(short(e));
            return row;
        }
    };
    let request = generate::GenerateRequest::new(
        1,
        vec![generate::PlayerRequest::new(
            "P1",
            generate::GameOptions::Native(generate::NativeOptions::new(world)),
        )],
    );
    row["rust_seed"] = match generate::generate(&request) {
        Ok(generated) => {
            match multidata::load_archipelago(
                &generated.archipelago,
                &multidata::ArchiveLimits::default(),
            ) {
                Ok(md) => json!({ "ok": true, "filled": md.locations.get(&1).map(|l| l.len()) }),
                Err(e) => json!({ "ok": false, "error": short(e) }),
            }
        }
        Err(e) => json!({ "ok": false, "error": short(e) }),
    };
    if round_trip_games.iter().any(|g| g == game) {
        row["round_trip"] = match round_trip(runtime, &conversion.toml) {
            Ok(()) => json!("ok"),
            Err(e) => json!(e),
        };
    }
    row["stage"] = json!("done");
    row["seconds"] = json!(started.elapsed().as_secs_f64());
    row
}

#[test]
#[ignore = "demande RAFTUP_REAL_RUNTIME, RAFTUP_INVENTAIRE_JEUX et RAFTUP_INVENTAIRE_OUT"]
fn inventaire_conversion() {
    let Some(runtime) = runtime() else { return };
    let (Some(list), Some(out)) = (
        std::env::var_os("RAFTUP_INVENTAIRE_JEUX"),
        std::env::var_os("RAFTUP_INVENTAIRE_OUT"),
    ) else {
        eprintln!("RAFTUP_INVENTAIRE_JEUX ou RAFTUP_INVENTAIRE_OUT absente : test sauté");
        return;
    };
    let games: Vec<String> = std::fs::read_to_string(list)
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let round_trip_games: Vec<String> = std::env::var("RAFTUP_INVENTAIRE_ALLER_RETOUR")
        .unwrap_or_default()
        .split('|')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out)
        .unwrap();
    for game in &games {
        eprintln!("== {game}");
        // Un jeu qui fait paniquer la chaîne ne doit pas arrêter l'inventaire.
        let row = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            inventory_one(&runtime, game, &round_trip_games)
        }))
        .unwrap_or_else(|_| json!({ "game": game, "stage": "panic" }));
        eprintln!("   {}", row["stage"]);
        writeln!(file, "{row}").unwrap();
        file.flush().unwrap();
    }
}
