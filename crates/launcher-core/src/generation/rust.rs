//! Le moteur Rust (ADR 0027, étape 4) : même contrat que le générateur Python, sans Python.
//!
//! Il ne prend une génération que si tous les joueurs jouent à un monde porté en Rust,
//! avec des options qu'il comprend ([`rust_support`]) ; sinon l'interface garde le
//! générateur Python. L'équilibrage de progression (étape 5) est fait ; un `meta.yaml`
//! (préréglages) est compris s'il ne règle que l'équilibrage et `accessibility: full`.

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use generate::NativeWorld;
use py_bridge::{CancelToken, GenerateParams, GenerateResult, GeneratedPlayer};

use super::export::{EngineReport, EngineUsed};
use super::run::GeneratorBackend;
use super::GenerationError;

/// Nom du moteur, écrit à la place de la version d'Archipelago.
pub fn engine_name() -> String {
    format!("Raftup {} (moteur Rust)", env!("CARGO_PKG_VERSION"))
}

/// Le générateur écrit en Rust.
///
/// `RustBackend::default()` : sans équilibrage par temps, comme avant l'ADR 0030, et
/// sans monde natif.
#[derive(Debug, Clone, Default)]
pub struct RustBackend {
    /// Équilibrage par temps de jeu (ADR 0030), avec les temps par lieu des profils.
    pub time_balance: bool,
    /// Mondes natifs connus (ADR 0041), en plus des mondes portés en Rust.
    pub native_worlds: Vec<Arc<NativeWorld>>,
}

/// Le moteur Rust sait-il générer ces fichiers joueurs ? `Err` donne la raison lisible
/// (jeu non porté, option avancée…) : la génération passe alors en Python.
///
/// `meta` : contenu du `meta.yaml` de la demande (préréglage), s'il y en a un.
///
/// Les tirages ne servent ici qu'à vérifier : la seed 0 suffit.
pub fn rust_support(player_files: &[PathBuf], meta: Option<&str>) -> Result<(), String> {
    rust_support_with_worlds(player_files, meta, &[])
}

/// Comme [`rust_support`], en connaissant aussi des mondes natifs (ADR 0041).
pub fn rust_support_with_worlds(
    player_files: &[PathBuf],
    meta: Option<&str>,
    natives: &[Arc<NativeWorld>],
) -> Result<(), String> {
    let files = read_files(player_files).map_err(|e| e.to_string())?;
    generate::read_players_with_worlds(&files, meta, 0, natives)
        .map(|_| ())
        .map_err(|e| e.0)
}

/// Contenu de `meta.yaml` dans le dossier des joueurs, s'il existe.
fn meta_in(dir: &Path) -> Result<Option<String>, GenerationError> {
    let path = dir.join("meta.yaml");
    if !path.is_file() {
        return Ok(None);
    }
    fs::read_to_string(&path)
        .map(Some)
        .map_err(|source| GenerationError::Io { path, source })
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
pub(super) fn random_seed() -> u64 {
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

    fn report(&self) -> EngineReport {
        EngineReport::plain(EngineUsed::Rust)
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

        let meta = meta_in(&p.player_files_dir)?;
        let players =
            generate::read_players_with_worlds(&files, meta.as_deref(), seed, &self.native_worlds)
                .map_err(|e| failed("Unsupported", e.0))?;
        if cancel.is_cancelled() {
            return Err(GenerationError::Cancelled);
        }
        let _ = log.send("Placement des objets".to_string());
        let time_balance = if self.time_balance {
            crate::estimate::time_balance_embedded(players.iter().map(|p| p.options.game()))
        } else {
            generate::TimeBalance::default()
        };
        let generated = generate::generate(&generate::GenerateRequest {
            seed,
            players: players.clone(),
            time_balance,
        })
        .map_err(|e| failed("RustGenerator", e.to_string()))?;
        if let Some(report) = &generated.time_balance {
            let _ = log.send(time_balance_line(report));
        }
        if cancel.is_cancelled() {
            return Err(GenerationError::Cancelled);
        }

        let zip = p.output_dir.join(format!("AP_{}.zip", generated.seed_name));
        let mut entries = vec![(
            format!("AP_{}.archipelago", generated.seed_name),
            generated.archipelago.clone(),
        )];
        // Spoiler comme Archipelago : aucun au niveau 0, sans parcours au niveau 1.
        if p.spoiler > 0 {
            let text = generated.spoiler.to_text(p.spoiler >= 2);
            entries.push((
                format!("AP_{}_Spoiler.txt", generated.seed_name),
                text.into_bytes(),
            ));
        }
        write_zip(&zip, &entries)?;
        let _ = log.send(format!("Partie écrite : {}", zip.display()));

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

/// Ligne de journal après l'équilibrage par temps : objets avancés, attente et fin
/// simulées avant → après, en minutes (arrondies vers le bas).
pub fn time_balance_line(report: &generate::TimeBalanceReport) -> String {
    format!(
        "Équilibrage par temps : {} objet(s) avancé(s), attente totale {} → {} min, fin {} → {} min (simulation)",
        report.moved,
        report.idle_before / 60,
        report.idle_after / 60,
        report.makespan_before / 60,
        report.makespan_after / 60,
    )
}

/// Écrit `AP_<seed>.zip` avec la seed et, s'il est demandé, le spoiler, comme le
/// générateur officiel. `entries` : (nom dans le zip, contenu), dans l'ordre d'écriture.
fn write_zip(path: &Path, entries: &[(String, Vec<u8>)]) -> Result<(), GenerationError> {
    let io = |source: std::io::Error| GenerationError::Io {
        path: path.to_path_buf(),
        source,
    };
    let file = fs::File::create(path).map_err(io)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let zip_err = |e: zip::result::ZipError| io(std::io::Error::other(e.to_string()));
    for (entry, bytes) in entries {
        zip.start_file(entry.as_str(), options).map_err(zip_err)?;
        zip.write_all(bytes).map_err(io)?;
    }
    zip.finish().map_err(zip_err)?;
    Ok(())
}
