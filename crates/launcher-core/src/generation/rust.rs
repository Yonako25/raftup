//! Le moteur Rust (ADR 0027, étape 4) : même contrat que le générateur Python, sans Python.
//!
//! Il ne prend une génération que si tous les joueurs jouent à un monde porté en Rust,
//! avec des options qu'il comprend ([`rust_support`]) ; sinon l'interface garde le
//! générateur Python. Il est sur option tant que l'équilibrage de progression n'est pas
//! fait (étape 5).

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use py_bridge::{CancelToken, GenerateParams, GenerateResult, GeneratedPlayer};

use super::run::GeneratorBackend;
use super::GenerationError;

/// Nom du moteur, écrit à la place de la version d'Archipelago.
pub fn engine_name() -> String {
    format!("Raftup {} (moteur Rust)", env!("CARGO_PKG_VERSION"))
}

/// Le générateur écrit en Rust.
#[derive(Debug, Clone, Copy, Default)]
pub struct RustBackend;

/// Le moteur Rust sait-il générer ces fichiers joueurs ? `Err` donne la raison lisible
/// (jeu non porté, option avancée…) : la génération passe alors en Python.
///
/// Les tirages ne servent ici qu'à vérifier : la seed 0 suffit.
pub fn rust_support(player_files: &[PathBuf]) -> Result<(), String> {
    let files = read_files(player_files).map_err(|e| e.to_string())?;
    generate::read_players(&files, 0)
        .map(|_| ())
        .map_err(|e| e.0)
}

fn read_files(paths: &[PathBuf]) -> Result<Vec<(String, String)>, GenerationError> {
    paths
        .iter()
        .map(|path| {
            let text = fs::read_to_string(path).map_err(|source| GenerationError::Io {
                path: path.clone(),
                source,
            })?;
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok((name, text))
        })
        .collect()
}

/// Fichiers joueurs posés par `generate` (`P01_….yaml`), dans l'ordre des slots.
fn player_files_in(dir: &Path) -> Result<Vec<PathBuf>, GenerationError> {
    let entries = fs::read_dir(dir).map_err(|source| GenerationError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            path.is_file()
                && name.ends_with(".yaml")
                && name != "meta.yaml"
                && name != "weights.yaml"
        })
        .collect();
    files.sort();
    Ok(files)
}

/// Seed au hasard quand l'hôte n'en donne pas.
///
/// `RandomState` est la graine aléatoire des `HashMap` de la bibliothèque standard,
/// tirée par le système : on s'en sert comme source de hasard sans dépendance de plus.
fn random_seed() -> u64 {
    RandomState::new().build_hasher().finish()
}

fn failed(kind: &str, message: String) -> GenerationError {
    GenerationError::GeneratorFailed {
        kind: kind.to_string(),
        message,
        traceback: String::new(),
    }
}

impl GeneratorBackend for RustBackend {
    /// Pas de dossier d'Archipelago : jamais lu, puisque [`uses_apworlds`] est faux.
    ///
    /// [`uses_apworlds`]: GeneratorBackend::uses_apworlds
    fn archipelago_dir(&self) -> &Path {
        Path::new("")
    }

    fn uses_apworlds(&self) -> bool {
        false
    }

    fn run(
        &mut self,
        p: &GenerateParams,
        _timeout: Duration,
        cancel: &CancelToken,
        log: Sender<String>,
    ) -> Result<GenerateResult, GenerationError> {
        let started = Instant::now();
        let files = read_files(&player_files_in(&p.player_files_dir)?)?;
        let seed = p.seed.unwrap_or_else(random_seed);
        let _ = log.send(format!("{} : seed {seed}", engine_name()));

        let players =
            generate::read_players(&files, seed).map_err(|e| failed("Unsupported", e.0))?;
        if cancel.is_cancelled() {
            return Err(GenerationError::Cancelled);
        }
        let _ = log.send("Placement des objets".to_string());
        let generated = generate::generate(&generate::GenerateRequest {
            seed,
            players: players.clone(),
        })
        .map_err(|e| failed("RustGenerator", e.to_string()))?;
        if cancel.is_cancelled() {
            return Err(GenerationError::Cancelled);
        }

        let zip = p.output_dir.join(format!("AP_{}.zip", generated.seed_name));
        write_zip(
            &zip,
            &format!("AP_{}.archipelago", generated.seed_name),
            &generated.archipelago,
        )?;
        let _ = log.send(format!("Seed écrite : {}", zip.display()));

        let players = players
            .iter()
            .enumerate()
            .map(|(index, player)| GeneratedPlayer {
                slot: u32::try_from(index + 1).unwrap_or(u32::MAX),
                name: player.name.clone(),
                game: player.options.game().to_string(),
            })
            .collect();
        Ok(GenerateResult {
            zip,
            seed_name: generated.seed_name,
            ap_version: engine_name(),
            players,
            seconds: started.elapsed().as_secs_f64(),
        })
    }
}

/// Écrit `AP_<seed>.zip` avec la seule seed dedans, comme le générateur officiel.
fn write_zip(path: &Path, entry: &str, bytes: &[u8]) -> Result<(), GenerationError> {
    let io = |source: std::io::Error| GenerationError::Io {
        path: path.to_path_buf(),
        source,
    };
    let file = fs::File::create(path).map_err(io)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let zip_err = |e: zip::result::ZipError| io(std::io::Error::other(e.to_string()));
    zip.start_file(entry, options).map_err(zip_err)?;
    zip.write_all(bytes).map_err(io)?;
    zip.finish().map_err(zip_err)?;
    Ok(())
}
