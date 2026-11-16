//! Cœur du launcher Raftup, sans interface (ADR 0010).
//!
//! - [`seed`] : lire une seed `.archipelago` pour montrer ses slots ;
//! - [`host`] : héberger une partie dans le launcher, la suivre, l'arrêter ;
//! - [`logs`] : garder le journal du serveur en mémoire pour l'afficher.
//!
//! `launcher-ui` (egui) n'appelle que cette crate.

pub mod error;
pub mod host;
pub mod logs;
pub mod seed;

pub use error::LauncherError;
pub use host::{Encryption, Host, HostOptions, HostStatus, NumberedEvent};
pub use logs::{LogBuffer, LogLine};
pub use seed::{SeedSlot, SeedSummary};
// Types du serveur que l'interface affiche : elle ne dépend que de cette crate.
pub use server::{GameEvent, ServerStatus, SlotStatus};
