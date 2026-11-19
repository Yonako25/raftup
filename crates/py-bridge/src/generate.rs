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
/// L'export des mondes (ADR 0028) a échoué après la construction des mondes.
pub const ERR_EXPORT_FAILED: i64 = -32013;

/// Module Python d'export (ADR 0028), envoyé en texte dans la requête `export_worlds` :
/// le worker passe par `python -c`, dont la ligne de commande est limitée en taille.
pub const EXPORTER_SCRIPT: &str = include_str!("../worker/raftup_export.py");

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

    /// Construit les mondes jusqu'à `pre_fill`, sans placement, et les décrit dans le
    /// fichier JSON `export_path` (ADR 0028). Comme `generate`, une seule fois par process.
    pub fn export_worlds_cancellable(
        &mut self,
        p: &ExportParams,
        timeout: Duration,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<ExportResult, BridgeError> {
        let params = serde_json::to_value(p)
            .map_err(|e| BridgeError::InvalidResponse(format!("paramètres illisibles : {e}")))?;
        let value = self.call_cancellable("export_worlds", params, timeout, cancel)?;
        serde_json::from_value(value)
            .map_err(|e| BridgeError::InvalidResponse(format!("résultat de export_worlds : {e}")))
    }
}

/// Paramètres de `export_worlds` : ceux de `generate`, plus le fichier à écrire et le
/// module d'export.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportParams {
    /// `flatten` : les champs de `GenerateParams` sont écrits au même niveau que les autres.
    #[serde(flatten)]
    pub generate: GenerateParams,
    /// Chemin absolu du JSON à écrire.
    pub export_path: PathBuf,
    /// Source du module d'export, en général [`EXPORTER_SCRIPT`].
    pub exporter: String,
}

/// Résultat de `export_worlds`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ExportResult {
    pub export: PathBuf,
    pub seed_name: String,
    pub ap_version: String,
    pub players: Vec<GeneratedPlayer>,
    /// Taille du JSON écrit.
    pub bytes: u64,
    /// Nombre de règles que l'export n'a pas su traduire.
    pub opaque_rules: u64,
    /// Nombre de règles Python (lambdas) traduites par sondage.
    pub probed_rules: u64,
    pub seconds: f64,
}

/// `finish_worlds` sans export en cours dans ce worker.
pub const ERR_NO_EXPORT: i64 = -32014;

/// Paramètres de `finish_worlds` : le placement de Raftup, dans un fichier JSON (une liste
/// de `[joueur du lieu, lieu, joueur de l'objet, objet]`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FinishParams {
    /// Chemin absolu du fichier de placement.
    pub placement_path: PathBuf,
}

impl PyBridge {
    /// Après [`PyBridge::export_worlds_cancellable`] : pose le placement de Raftup, puis
    /// Archipelago reprend sa génération et écrit la seed (`AP_<seed>.zip`), comme
    /// `generate` (ADR 0028).
    pub fn finish_worlds_cancellable(
        &mut self,
        p: &FinishParams,
        timeout: Duration,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<GenerateResult, BridgeError> {
        let params = serde_json::to_value(p)
            .map_err(|e| BridgeError::InvalidResponse(format!("paramètres illisibles : {e}")))?;
        let value = self.call_cancellable("finish_worlds", params, timeout, cancel)?;
        serde_json::from_value(value)
            .map_err(|e| BridgeError::InvalidResponse(format!("résultat de finish_worlds : {e}")))
    }
}
