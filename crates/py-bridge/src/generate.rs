//! Types JSON de la méthode `generate` du worker (ADR 0017) et son appel côté Rust.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::bridge::PyBridge;
use crate::error::BridgeError;

/// Génération échouée (erreur du générateur ; `data` = `{type, traceback}`).
pub const ERR_GENERATION_FAILED: i64 = -32010;
/// Import d'Archipelago ou d'une de ses dépendances impossible.
pub const ERR_IMPORT_FAILED: i64 = -32011;
/// Un seul `generate` par process : ce worker a déjà servi.
pub const ERR_ALREADY_USED: i64 = -32012;

/// Paramètres de `generate`. Les chemins doivent être absolus (le worker change de dossier courant).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GenerateParams {
    pub archipelago_dir: PathBuf,
    /// `None` est envoyé comme `null` (serde sérialise `Option::None` en `null`).
    pub site_packages: Option<PathBuf>,
    pub player_files_dir: PathBuf,
    pub output_dir: PathBuf,
    pub seed: Option<u64>,
    /// Niveau de spoiler, de 0 à 3.
    pub spoiler: u8,
    pub race: bool,
}

/// Un joueur de la partie générée.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GeneratedPlayer {
    pub slot: u32,
    pub name: String,
    pub game: String,
}

/// Résultat de `generate`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GenerateResult {
    /// Chemin du zip produit (toujours sous `output_dir`).
    pub zip: PathBuf,
    pub seed_name: String,
    pub ap_version: String,
    pub players: Vec<GeneratedPlayer>,
    pub seconds: f64,
}

impl PyBridge {
    /// Lance une génération. Bloquant jusqu'à la fin ou au délai.
    /// Un échec du générateur arrive en `BridgeError::Worker` (codes ci-dessus) et laisse le pont utilisable.
    pub fn generate(
        &mut self,
        p: &GenerateParams,
        timeout: Duration,
    ) -> Result<GenerateResult, BridgeError> {
        let params = serde_json::to_value(p)
            .map_err(|e| BridgeError::InvalidResponse(format!("paramètres illisibles : {e}")))?;
        let value = self.call_with_timeout("generate", params, timeout)?;
        serde_json::from_value(value)
            .map_err(|e| BridgeError::InvalidResponse(format!("résultat de generate : {e}")))
    }

    /// Comme [`PyBridge::generate`], mais annulable : si `cancel` est annulé, le process est
    /// tué et l'appel renvoie `BridgeError::Cancelled`.
    pub fn generate_cancellable(
        &mut self,
        p: &GenerateParams,
        timeout: Duration,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<GenerateResult, BridgeError> {
        let params = serde_json::to_value(p)
            .map_err(|e| BridgeError::InvalidResponse(format!("paramètres illisibles : {e}")))?;
        let value = self.call_cancellable("generate", params, timeout, cancel)?;
        serde_json::from_value(value)
            .map_err(|e| BridgeError::InvalidResponse(format!("résultat de generate : {e}")))
    }
}
