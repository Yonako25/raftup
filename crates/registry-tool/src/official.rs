//! Le vrai générateur des vérifications : le générateur officiel d'Archipelago, épinglé
//! (ADR 0017), lancé par `launcher-core` comme dans « Créer une partie » : réseau bloqué,
//! bac à sable (ADR 0034), apworld posée dans `custom_worlds`.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use launcher_core::generation::{
    discard_generation, generate, inspect_worlds, GenerateRequest, PythonBackend, Runtime,
};
use launcher_core::{GenerationError, Paths};
use py_bridge::{BridgeError, CancelToken};

use crate::check::{Generator, RunOutcome};
use crate::options::{parse_description, OptionSpec, DESCRIBE_SCRIPT};

/// Le générateur officiel, prêt à servir.
pub struct OfficialGenerator {
    paths: Paths,
    backend: PythonBackend,
    /// Dossier où sont écrits les YAML de test.
    work_dir: PathBuf,
}

impl OfficialGenerator {
    /// `runtime` : runtime installé (`launcher_core::generation::ensure_runtime`).
    /// `work_dir` : dossier à soi pour les YAML de test.
    pub fn new(paths: Paths, runtime: Runtime, work_dir: PathBuf) -> OfficialGenerator {
        OfficialGenerator {
            paths,
            backend: PythonBackend::new(runtime),
            work_dir,
        }
    }

    /// Écrit le YAML de test et rend la requête de génération.
    fn request(
        &self,
        apworld: &Path,
        yaml: &str,
        seed: u64,
        timeout: Duration,
    ) -> Result<GenerateRequest, RunOutcome> {
        std::fs::create_dir_all(&self.work_dir)
            .map_err(|e| RunOutcome::Environment(format!("{} : {e}", self.work_dir.display())))?;
        let file = self.work_dir.join("Testeur.yaml");
        std::fs::write(&file, yaml)
            .map_err(|e| RunOutcome::Environment(format!("{} : {e}", file.display())))?;
        Ok(GenerateRequest {
            player_files: vec![file],
            apworlds: vec![apworld.to_path_buf()],
            seed: Some(seed),
            // Pas de spoiler : on ne garde rien de la seed.
            spoiler: 0,
            race: false,
            timeout,
            meta_yaml: None,
        })
    }
}

impl Generator for OfficialGenerator {
    fn name(&self) -> String {
        format!("Archipelago {}", self.backend.runtime.ap_version)
    }

    fn generate(&mut self, apworld: &Path, yaml: &str, seed: u64, timeout: Duration) -> RunOutcome {
        let req = match self.request(apworld, yaml, seed, timeout) {
            Ok(req) => req,
            Err(outcome) => return outcome,
        };
        // Les lignes du worker ne nous servent pas : le récepteur est jeté tout de suite
        // et les envois échouent sans bruit.
        let (log, _) = mpsc::channel();
        match generate(
            &self.paths,
            &mut self.backend,
            &req,
            &CancelToken::new(),
            log,
        ) {
            Ok(output) => {
                // On ne garde que le verdict : la seed est effacée.
                if let Err(e) = discard_generation(&self.paths, &output) {
                    tracing::warn!("seed de test non effacée : {e}");
                }
                RunOutcome::Generated
            }
            Err(error) => classify(error),
        }
    }

    fn describe_options(
        &mut self,
        apworld: &Path,
        game: &str,
        timeout: Duration,
    ) -> Result<Vec<OptionSpec>, RunOutcome> {
        let req = self.request(apworld, &crate::options::default_yaml(game), 1, timeout)?;
        let (log, _) = mpsc::channel();
        let value = inspect_worlds(
            &self.paths,
            &self.backend,
            &req,
            DESCRIBE_SCRIPT,
            &CancelToken::new(),
            log,
        )
        .map_err(classify)?;
        parse_description(&value).map_err(RunOutcome::Refused)
    }
}

/// Range une erreur de génération : refus propre, plantage, boucle ou environnement.
pub fn classify(error: GenerationError) -> RunOutcome {
    let text = error.to_string();
    match error {
        // Archipelago a levé une exception et le worker l'a rapportée : refus propre.
        GenerationError::GeneratorFailed { .. }
        | GenerationError::InvalidPlayerFile { .. }
        | GenerationError::IncompatibleApworld { .. } => RunOutcome::Refused(text),
        GenerationError::Bridge(bridge) => match bridge {
            BridgeError::Timeout { .. } => RunOutcome::TimedOut(text),
            BridgeError::ProcessExited { .. }
            | BridgeError::InvalidJson { .. }
            | BridgeError::InvalidResponse(_)
            | BridgeError::LineTooLong { .. }
            | BridgeError::UnexpectedId { .. }
            | BridgeError::Protocol(_)
            | BridgeError::Closed => RunOutcome::Crashed(text),
            // Erreur du worker hors génération (erreur interne -32603, export -32013…).
            BridgeError::Worker(_) => RunOutcome::Crashed(text),
            // Python absent, bac à sable impossible, flux cassé… : la machine, pas le paquet.
            _ => RunOutcome::Environment(text),
        },
        // Le générateur dit « fini » mais sa sortie est inutilisable.
        GenerationError::NoOutput | GenerationError::UnsafeOutput(_) | GenerationError::Seed(_) => {
            RunOutcome::Crashed(text)
        }
        _ => RunOutcome::Environment(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use py_bridge::RpcError;

    #[test]
    fn erreurs_rangees() {
        let refus = GenerationError::GeneratorFailed {
            kind: "OptionError".into(),
            message: "valeur inconnue".into(),
            traceback: String::new(),
        };
        assert!(matches!(classify(refus), RunOutcome::Refused(_)));
        let boucle = GenerationError::Bridge(BridgeError::Timeout {
            method: "generate".into(),
            after: Duration::from_secs(1),
        });
        assert!(matches!(classify(boucle), RunOutcome::TimedOut(_)));
        let mort = GenerationError::Bridge(BridgeError::ProcessExited {
            status: None,
            stderr_tail: "Segmentation fault".into(),
        });
        assert!(matches!(classify(mort), RunOutcome::Crashed(_)));
        let interne = GenerationError::Bridge(BridgeError::Worker(RpcError {
            code: -32603,
            message: "erreur interne".into(),
            data: None,
        }));
        assert!(matches!(classify(interne), RunOutcome::Crashed(_)));
        assert!(matches!(
            classify(GenerationError::Busy),
            RunOutcome::Environment(_)
        ));
        let sans_python = GenerationError::Bridge(BridgeError::PythonNotFound {
            path: PathBuf::from("python.exe"),
        });
        assert!(matches!(classify(sans_python), RunOutcome::Environment(_)));
    }
}
