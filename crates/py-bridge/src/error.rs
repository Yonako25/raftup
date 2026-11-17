//! Erreurs du pont : celles du worker (`RpcError`) et celles du transport (`BridgeError`).

use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

/// Erreur renvoyée par le worker lui-même dans le champ `error` d'une réponse.
#[derive(Debug, Clone, PartialEq, Deserialize, Error)]
#[error("erreur {code} du worker : {message}")]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    /// Détails optionnels ; absent du JSON = `None` (comportement par défaut de serde pour `Option`).
    #[serde(default)]
    pub data: Option<Value>,
}

/// Toutes les erreurs possibles du pont.
#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("Python introuvable : {}", path.display())]
    PythonNotFound { path: PathBuf },

    #[error("impossible de lancer {} : {source}", path.display())]
    Spawn {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("le process Python s'est arrêté (statut : {}) ; fin de stderr : {stderr_tail}", status_text(.status))]
    ProcessExited {
        status: Option<ExitStatus>,
        stderr_tail: String,
    },

    #[error("JSON invalide ({reason}) dans la ligne : {line}")]
    InvalidJson { line: String, reason: String },

    #[error("réponse invalide du worker : {0}")]
    InvalidResponse(String),

    #[error("ligne trop longue : plus de {max} octets")]
    LineTooLong { max: usize },

    #[error("id inattendu dans la réponse : attendu {expected}, reçu {got:?}")]
    UnexpectedId { expected: u64, got: Option<u64> },

    #[error("délai dépassé pour « {method} » après {after:?}")]
    Timeout { method: String, after: Duration },

    // `#[from]` fait que `?` convertit automatiquement un `RpcError` en `BridgeError`.
    #[error(transparent)]
    Worker(#[from] RpcError),

    #[error("worker incompatible : protocole {got} reçu, {expected} attendu")]
    IncompatibleWorker { expected: u32, got: u32 },

    #[error("requête trop grande : {len} octets, maximum {max}")]
    RequestTooLarge { max: usize, len: usize },

    /// Le worker a répondu `id: null` avec une erreur (requête illisible, ligne trop longue...) :
    /// le flux est considéré désynchronisé, le pont se ferme, mais on garde son message.
    #[error("erreur de protocole signalée par le worker : {0}")]
    Protocol(RpcError),

    #[error("erreur d'entrée-sortie ({context}) : {source}")]
    Io {
        context: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("impossible d'encoder la requête : {0}")]
    Encode(String),

    /// Sous Windows : `python` doit être un `.exe` (les `.bat`/`.cmd` sont refusés).
    #[error("exécutable Python non supporté (un .exe est requis) : {}", path.display())]
    UnsupportedPython { path: PathBuf },

    #[error("le pont est fermé")]
    Closed,
}

impl BridgeError {
    /// Construit une `InvalidJson` en tronquant la ligne à 200 caractères.
    pub fn invalid_json(line: &str, reason: impl Into<String>) -> Self {
        // On compte des caractères (pas des octets) pour ne jamais couper un caractère UTF-8 en deux.
        let line: String = line.chars().take(200).collect();
        BridgeError::InvalidJson {
            line,
            reason: reason.into(),
        }
    }
}

fn status_text(status: &Option<ExitStatus>) -> String {
    match status {
        Some(s) => s.to_string(),
        None => "inconnu".to_string(),
    }
}
