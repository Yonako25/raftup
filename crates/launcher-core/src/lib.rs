//! Cœur du launcher Raftup, sans interface (ADR 0010).
//!
//! - [`seed`] : lire une seed `.archipelago` pour montrer ses slots ;
//! - [`host`] : héberger une partie dans le launcher, la suivre, l'arrêter ;
//! - [`logs`] : garder le journal du serveur en mémoire pour l'afficher ;
//! - [`catalog`], [`profile`] : les profils d'installation des jeux (ADR 0015) ;
//! - [`detect`] : trouver les jeux déjà installés (Steam, GOG, Epic, dossier choisi) ;
//! - [`install`], [`uninstall`] : poser et retirer les mods, avec reçu et sauvegardes ;
//! - [`lockfile`] : comparer les mods attendus par une seed à ceux installés ;
//! - [`launch`] : préparer et lancer le jeu ;
//! - [`diagnose`] : expliquer en clair un échec de connexion ;
//! - [`config`], [`paths`] : réglages du joueur et dossiers de Raftup.
//!
//! `launcher-ui` (egui) n'appelle que cette crate.

pub mod archive;
pub mod catalog;
pub mod config;
pub mod detect;
pub mod diagnose;
pub mod download;
pub mod error;
pub mod hash;
pub mod host;
pub mod install;
pub mod launch;
pub mod lockfile;
pub mod logs;
pub mod paths;
pub mod profile;
pub mod record;
pub mod seed;
pub mod template;
pub mod uninstall;

pub use catalog::Catalog;
pub use config::{GameSettings, LoadedConfig, UserConfig};
pub use detect::{
    detect_games, validate_manual_dir, DetectRule, DetectedGame, DetectionRoots, GameState, Store,
};
pub use diagnose::{explain, ConnectionProblem};
pub use download::{DirFetcher, Downloader, HttpFetcher};
pub use error::LauncherError;
pub use host::{Encryption, Host, HostOptions, HostStatus, NumberedEvent};
pub use install::{install, status, InstallError, InstallOptions, InstallReport, InstallStatus};
pub use launch::{launch, plan_launch, Credentials, LaunchError, LaunchPlan};
pub use lockfile::{LockReport, Lockfile, Mismatch};
pub use logs::{LogBuffer, LogLine};
pub use paths::Paths;
pub use profile::{Profile, ProfileError};
pub use record::{InstallRecord, RecordState};
pub use seed::{SeedSlot, SeedSummary};
pub use uninstall::{uninstall, UninstallReport};
// Types du serveur que l'interface affiche : elle ne dépend que de cette crate.
pub use server::{GameEvent, ServerStatus, SlotStatus};
