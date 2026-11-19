//! Le moteur « export » (ADR 0028) : Archipelago construit les mondes et écrit la seed,
//! Raftup fait le placement entre les deux.
//!
//! 1. `export_worlds` : le worker construit les mondes jusqu'à `pre_fill` et les décrit
//!    en JSON (`export.json`), puis attend ;
//! 2. Raftup lit l'export (`generate::imported`, contrôle croisé compris), place les
//!    objets et équilibre la progression ;
//! 3. `finish_worlds` : le worker pose ce placement (`placement.json`) et Archipelago
//!    finit sa génération comme d'habitude (indices, `slot_data`, zip de la seed).
//!
//! Si un monde ne peut pas passer par Raftup (règle non traduite, option non prise en
//! charge, placement impossible…), la raison est écrite dans le journal et la génération
//! repart avec le générateur Python seul, dans un worker neuf : le joueur a toujours sa
//! seed.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use generate::imported::{build_world, fill_imported, read_export, ImportError, MAX_EXPORT_BYTES};
use py_bridge::{
    CancelToken, ExportParams, FinishParams, GenerateParams, GenerateResult, PyBridge,
    EXPORTER_SCRIPT,
};

use super::run::{GeneratorBackend, PythonBackend, Runtime};
use super::rust::random_seed;
use super::GenerationError;
use multidata::read_file_bounded;

/// Le moteur « export » : un générateur Python pour construire et écrire, Raftup pour placer.
#[derive(Debug, Clone)]
pub struct ExportBackend {
    /// Le générateur Python, qui sert aussi de repli.
    pub python: PythonBackend,
}

impl ExportBackend {
    /// Moteur avec les réglages par défaut (réseau du worker bloqué).
    pub fn new(runtime: Runtime) -> ExportBackend {
        ExportBackend {
            python: PythonBackend::new(runtime),
        }
    }
}

/// Pourquoi la génération repasse au générateur Python seul.
enum Fallback {
    /// Le monde ne passe pas par Raftup (raison lisible).
    Unsupported(String),
}

impl GeneratorBackend for ExportBackend {
    fn archipelago_dir(&self) -> &Path {
        self.python.archipelago_dir()
    }

    fn run(
        &mut self,
        p: &GenerateParams,
        timeout: Duration,
        cancel: &CancelToken,
        log: Sender<String>,
    ) -> Result<GenerateResult, GenerationError> {
        // La seed doit être connue des deux côtés : on la tire ici si l'hôte n'en donne pas.
        let mut p = p.clone();
        let seed = *p.seed.get_or_insert_with(random_seed);
        let mut bridge = self.python.start_bridge(log.clone())?;
        let outcome =
            export_then_finish(&self.python, &mut bridge, &p, seed, timeout, cancel, &log);
        // Un worker par génération : fermé dans tous les cas (erreur de fermeture ignorée).
        let _ = bridge.shutdown();
        match outcome {
            Ok(result) => Ok(result),
            Err(Step::Fallback(Fallback::Unsupported(reason))) => {
                let _ = log.send(format!(
                    "Placement par Raftup impossible ({reason}) : Archipelago génère seul."
                ));
                self.python.run(&p, timeout, cancel, log)
            }
            Err(Step::Error(e)) => Err(e),
        }
    }
}

/// Issue d'une étape : erreur à remonter, ou repli sur le générateur Python.
enum Step {
    Error(GenerationError),
    Fallback(Fallback),
}

impl From<GenerationError> for Step {
    fn from(e: GenerationError) -> Step {
        Step::Error(e)
    }
}

impl From<py_bridge::BridgeError> for Step {
    fn from(e: py_bridge::BridgeError) -> Step {
        Step::Error(e.into())
    }
}

fn io_err(path: &Path, source: std::io::Error) -> GenerationError {
    GenerationError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[allow(clippy::too_many_arguments)]
fn export_then_finish(
    python: &PythonBackend,
    bridge: &mut PyBridge,
    p: &GenerateParams,
    seed: u64,
    timeout: Duration,
    cancel: &CancelToken,
    log: &Sender<String>,
) -> Result<GenerateResult, Step> {
    let export_path: PathBuf = p.output_dir.join("export.json");
    let params = ExportParams {
        generate: python.complete(p),
        export_path: export_path.clone(),
        exporter: EXPORTER_SCRIPT.to_string(),
    };
    let exported = bridge.export_worlds_cancellable(&params, timeout, cancel)?;
    let _ = log.send(format!(
        "Mondes exportés : {} règles Python traduites, {} non traduites",
        exported.probed_rules, exported.opaque_rules
    ));

    // L'export vient du code des apworlds : lu avec une taille maximale.
    let bytes = read_file_bounded(&export_path, MAX_EXPORT_BYTES as u64)
        .map_err(|source| io_err(&export_path, source))?;
    let unsupported = |e: ImportError| Step::Fallback(Fallback::Unsupported(e.to_string()));
    let export = read_export(&bytes).map_err(unsupported)?;
    if export.checks.is_none() {
        return Err(Step::Fallback(Fallback::Unsupported(
            "export sans contrôle croisé".to_string(),
        )));
    }
    let mut imported = build_world(&export).map_err(unsupported)?;
    let _ = log.send("Placement des objets par Raftup".to_string());
    let placement = fill_imported(&mut imported, &export, seed).map_err(unsupported)?;
    if cancel.is_cancelled() {
        return Err(Step::Error(GenerationError::Cancelled));
    }

    let placement_path = p.output_dir.join("placement.json");
    let text = serde_json::to_string(&placement).map_err(|e| {
        Step::Error(io_err(
            &placement_path,
            std::io::Error::other(e.to_string()),
        ))
    })?;
    fs::write(&placement_path, text).map_err(|e| io_err(&placement_path, e))?;
    let finish = FinishParams {
        placement_path: placement_path.clone(),
    };
    let result = bridge.finish_worlds_cancellable(&finish, timeout, cancel)?;
    // Les deux fichiers de travail ne servent plus : la seed est dans le zip.
    let _ = fs::remove_file(&export_path);
    let _ = fs::remove_file(&placement_path);
    Ok(result)
}
