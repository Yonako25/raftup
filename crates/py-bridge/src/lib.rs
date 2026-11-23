//! Pont entre Raftup et le générateur Archipelago officiel (ADR 0016).
//!
//! Le générateur tourne dans un process Python séparé ; on lui parle en JSON-RPC
//! simplifié, une ligne JSON par message, sur stdin/stdout.

pub mod bridge;
pub mod cancel;
pub mod error;
pub mod generate;
mod job;
pub mod line;
pub mod protocol;
pub mod sandbox;

pub use bridge::{BridgeOptions, PyBridge, WorkerVersion};
pub use cancel::CancelToken;
pub use error::{BridgeError, RpcError};
pub use generate::{
    exporter_script, ExportParams, ExportResult, FinishParams, GenerateParams, GenerateResult,
    GeneratedPlayer, ProbeLimits, ERR_ALREADY_USED, ERR_EXPORT_FAILED, ERR_GENERATION_FAILED,
    ERR_IMPORT_FAILED, ERR_NO_EXPORT, EXPORTER_SCRIPT,
};
pub use line::{read_bounded_bytes, read_bounded_line, LineError};
pub use protocol::{decode_response, encode_request};
pub use sandbox::{Sandbox, SANDBOX_SUPPORTED};

/// Script Python du worker, embarqué dans le binaire (livré via `python -c`, rien sur le disque).
pub const WORKER_SCRIPT: &str = include_str!("../worker/raftup_worker.py");

/// Version du protocole parlée par ce pont ; un worker d'une autre version est refusé.
pub const WORKER_PROTOCOL_VERSION: u32 = 1;

/// Taille maximale d'une ligne JSON, en octets (1 Mio).
pub const DEFAULT_MAX_LINE_BYTES: usize = 1024 * 1024;
