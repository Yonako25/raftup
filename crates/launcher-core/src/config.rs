//! Configuration de l'utilisateur, qui survit aux mises à jour (F07, ADR 0015).
//!
//! - Le fichier porte un `format_version` ; à la lecture, il passe par des migrations
//!   `vN -> vN+1` jusqu'à la version courante.
//! - Un champ ajouté plus tard se lit avec une valeur par défaut (`#[serde(default)]`),
//!   donc un ancien fichier reste lisible.
//! - Un fichier illisible est renommé en `config.toml.illisible` (puis `.illisible.1`…),
//!   jamais écrasé en silence.
//! - L'écriture est atomique : fichier `.tmp`, puis renommage.
//! - **Aucun mot de passe** n'est stocké ici (NF09) : le type n'a pas de tel champ.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Version du format écrite par ce launcher.
pub const CURRENT_FORMAT_VERSION: u32 = 1;

/// Ce qui fait échouer la lecture ou l'écriture de la configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Lecture ou écriture impossible sur ce chemin.
    #[error("accès à {path} : {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Le fichier vient d'un launcher plus récent : on n'y touche pas.
    #[error("configuration en version {found}, ce launcher ne connaît que jusqu'à la version {supported} : mettez Raftup à jour")]
    UnsupportedVersion { found: u32, supported: u32 },
    /// Impossible de produire le TOML.
    #[error("écriture de la configuration : {0}")]
    Serialize(String),
}

/// Réglages propres à un jeu.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameSettings {
    /// Dossier du jeu choisi à la main.
    #[serde(default)]
    pub game_dir: Option<PathBuf>,
}

/// Configuration de l'utilisateur. Volontairement sans mot de passe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserConfig {
    #[serde(default = "current_version")]
    pub format_version: u32,
    #[serde(default)]
    pub player_name: Option<String>,
    /// `BTreeMap` : ordre stable, donc fichier identique d'une écriture à l'autre.
    #[serde(default)]
    pub games: BTreeMap<String, GameSettings>,
    #[serde(default)]
    pub last_port: Option<u16>,
}

fn current_version() -> u32 {
    CURRENT_FORMAT_VERSION
}

impl Default for UserConfig {
    fn default() -> Self {
        UserConfig {
            format_version: CURRENT_FORMAT_VERSION,
            player_name: None,
            games: BTreeMap::new(),
            last_port: None,
        }
    }
}

/// Résultat de `load`.
#[derive(Debug)]
pub struct LoadedConfig {
    pub config: UserConfig,
    /// Où le fichier illisible a été mis de côté, s'il y en avait un.
    pub set_aside: Option<PathBuf>,
}

/// Migre la table TOML de la version `from` vers `from + 1`.
///
/// Aucune migration n'existe encore (une seule version) : le jour où le format change,
/// on ajoute ici une branche `1 => { ... }` qui transforme la table.
fn migrate_one(_from: u32, table: toml::Table) -> toml::Table {
    table
}

/// Lit la configuration. Absente : valeurs par défaut. Illisible : mise de côté puis défauts.
pub fn load(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LoadedConfig {
                config: UserConfig::default(),
                set_aside: None,
            })
        }
        // Contenu non UTF-8 : le fichier existe mais est illisible.
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            return set_aside_and_default(path);
        }
        Err(source) => {
            return Err(ConfigError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    };

    let Ok(mut table) = text.parse::<toml::Table>() else {
        return set_aside_and_default(path);
    };

    // Version absente : on suppose la première.
    let found = match table.get("format_version") {
        None => 1,
        Some(value) => match value.as_integer().and_then(|v| u32::try_from(v).ok()) {
            Some(v) => v,
            None => return set_aside_and_default(path),
        },
    };
    if found > CURRENT_FORMAT_VERSION {
        return Err(ConfigError::UnsupportedVersion {
            found,
            supported: CURRENT_FORMAT_VERSION,
        });
    }
    for version in found..CURRENT_FORMAT_VERSION {
        table = migrate_one(version, table);
    }
    table.insert(
        "format_version".to_string(),
        toml::Value::Integer(i64::from(CURRENT_FORMAT_VERSION)),
    );

    match table.try_into::<UserConfig>() {
        Ok(config) => Ok(LoadedConfig {
            config,
            set_aside: None,
        }),
        Err(_) => set_aside_and_default(path),
    }
}

/// Renomme le fichier illisible et rend la configuration par défaut.
fn set_aside_and_default(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let target = free_set_aside_name(path);
    fs::rename(path, &target).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    tracing::warn!(
        "configuration illisible, mise de côté dans {}",
        target.display()
    );
    Ok(LoadedConfig {
        config: UserConfig::default(),
        set_aside: Some(target),
    })
}

/// `config.toml.illisible`, sinon `.illisible.1`, `.illisible.2`… : le premier nom libre.
fn free_set_aside_name(path: &Path) -> PathBuf {
    let base = format!("{}.illisible", path.display());
    let mut candidate = PathBuf::from(&base);
    let mut n = 1u32;
    while candidate.exists() {
        candidate = PathBuf::from(format!("{base}.{n}"));
        n += 1;
    }
    candidate
}

impl UserConfig {
    /// Écrit la configuration de façon atomique : `.tmp` puis renommage.
    /// Une coupure en plein milieu laisse l'ancien fichier intact.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let io = |p: &Path, source| ConfigError::Io {
            path: p.to_path_buf(),
            source,
        };
        let text =
            toml::to_string_pretty(self).map_err(|e| ConfigError::Serialize(e.to_string()))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        let tmp = PathBuf::from(format!("{}.tmp", path.display()));
        if let Err(e) = fs::write(&tmp, text) {
            let _ = fs::remove_file(&tmp);
            return Err(io(&tmp, e));
        }
        fs::rename(&tmp, path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            io(path, e)
        })
    }
}
