//! Format de monde natif déclaratif de Raftup (V3, ADR 0041).
//!
//! Un monde natif décrit un jeu dans un fichier TOML, sans code : objets, lieux, régions,
//! entrées, événements, options et règles d'accès écrites dans un petit langage. La
//! spécification complète est dans `spec/monde-natif/README.md`.
//!
//! Trois étapes :
//!
//! 1. **lecture** ([`parse_world`], [`load_path`]) : TOML borné, champs inconnus refusés ;
//! 2. **validation** (pendant la lecture) : noms en double, références inconnues, règles mal
//!    typées, régions inaccessibles ; chaque erreur donne la ligne et la colonne ;
//! 3. **compilation** ([`NativeWorld::build`]) : avec les options d'un joueur, le monde est
//!    construit dans un [`logic::MultiWorld`], comme un monde porté en Rust. Aucun nouveau
//!    moteur : le placement reste celui de `fill`.
//!
//! Rien ici ne panique sur une entrée hostile, et tout est parcouru dans l'ordre du
//! fichier : même fichier et mêmes options ⇒ même monde.

pub mod apworld;
mod build;
mod error;
mod expr;
mod load;
pub mod model;

pub use build::{Built, OptionValues};
pub use error::{BuildError, LoadError, Position};
pub use load::{load_path, parse_world};
pub use model::NativeWorld;

/// Version du format comprise par cette crate.
pub const FORMAT_VERSION: i64 = 1;
/// Taille maximale d'un fichier de monde.
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
/// Nombre maximal d'éléments par liste (objets, lieux, régions…).
pub const MAX_ELEMENTS: usize = 100_000;
/// Taille maximale du texte d'une règle.
pub const MAX_RULE_BYTES: usize = 16 * 1024;
/// Imbrication maximale d'une règle (parenthèses, `not`, appels).
pub const MAX_DEPTH: usize = 64;
/// Longueur maximale d'un nom, en caractères.
pub const MAX_NAME_CHARS: usize = 200;
/// Nombre maximal de choix d'une option.
pub const MAX_CHOICES: usize = 1_000;
