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

/// Le moteur qui a vraiment produit (ou tenté de produire) la seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineUsed {
    /// Le moteur Rust seul, sans Python (ADR 0027).
    Rust,
    /// Archipelago a construit les mondes et écrit la seed, Raftup a placé les objets
    /// (ADR 0028).
    Export,
    /// Le générateur d'Archipelago seul.
    Python,
}

/// Ce que le moteur dit de la dernière génération : qui l'a faite, et pourquoi le
/// placement par Raftup a été abandonné s'il y a eu repli.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineReport {
    pub engine: EngineUsed,
    /// Raison lisible du repli sur Archipelago seul (moteur « export » seulement).
    pub fallback: Option<String>,
}

impl EngineReport {
    /// Rapport d'un moteur qui n'a jamais de repli.
    pub fn plain(engine: EngineUsed) -> EngineReport {
        EngineReport {
            engine,
            fallback: None,
        }
    }
}

/// Le moteur « export » : un générateur Python pour construire et écrire, Raftup pour placer.
#[derive(Debug, Clone)]
pub struct ExportBackend {
    /// Le générateur Python, qui sert aussi de repli.
    pub python: PythonBackend,
    /// Raison du repli de la dernière génération (`None` : placement par Raftup).
    last_fallback: Option<String>,
}

impl ExportBackend {
    /// Moteur avec les réglages par défaut (réseau du worker bloqué).
    pub fn new(runtime: Runtime) -> ExportBackend {
        ExportBackend {
            python: PythonBackend::new(runtime),
            last_fallback: None,
        }
    }
}

/// Phrase du journal qui annonce le repli (gardée identique pour l'interface et les tests).
pub fn fallback_line(reason: &str) -> String {
    format!("Placement par Raftup impossible ({reason}) : Archipelago génère seul.")
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
        self.last_fallback = None;
        let mut bridge = self.python.start_bridge(&p, log.clone())?;
        let outcome =
            export_then_finish(&self.python, &mut bridge, &p, seed, timeout, cancel, &log);
        // Un worker par génération : fermé dans tous les cas (erreur de fermeture ignorée).
        let _ = bridge.shutdown();
        match outcome {
            Ok(result) => Ok(result),
            Err(Step::Fallback(Fallback::Unsupported(reason))) => {
                let _ = log.send(fallback_line(&reason));
                // Gardée pour le rapport de fin (`report`), même si Python échoue ensuite.
                self.last_fallback = Some(reason);
                self.python.run(&p, timeout, cancel, log)
            }
            Err(Step::Error(e)) => Err(e),
        }
    }

    fn report(&self) -> EngineReport {
        match &self.last_fallback {
            Some(reason) => EngineReport {
                engine: EngineUsed::Python,
                fallback: Some(reason.clone()),
            },
            None => EngineReport::plain(EngineUsed::Export),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> ExportBackend {
        // Chemins jamais lus : `report` ne lance aucun worker.
        ExportBackend::new(Runtime::from_local("py", "archipelago", None, "0.6.8"))
    }

    #[test]
    fn rapport_sans_repli() {
        assert_eq!(backend().report(), EngineReport::plain(EngineUsed::Export));
    }

    #[test]
    fn rapport_apres_repli() {
        let mut export = backend();
        export.last_fallback = Some("règle non traduite".to_string());
        let report = export.report();
        assert_eq!(report.engine, EngineUsed::Python);
        assert_eq!(report.fallback.as_deref(), Some("règle non traduite"));
        assert_eq!(
            fallback_line("règle non traduite"),
            "Placement par Raftup impossible (règle non traduite) : Archipelago génère seul."
        );
    }
}
