//! Le reçu d'installation (un fichier TOML par jeu), écrit de façon atomique.
//!
//! Le reçu est écrit **avant** de toucher au dossier du jeu (état `InProgress`), puis
//! passé à `Complete` quand tout est extrait. La désinstallation s'appuie sur lui
//! pour savoir exactement quels fichiers retirer (ADR 0015, « Réversibilité »).
//!
//! Tous les chemins de `StepRecord` (fichiers, dossiers, sauvegardes) sont **relatifs
//! à la racine de l'étape** (`game_dir` ou `worlds_dir`), jamais absolus : le reçu
//! ne dit donc pas où est le jeu, sauf dans `game_dir` en tête de fichier.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hash::Sha256;
use crate::paths::Paths;

/// Version du format de reçu que ce launcher sait lire et écrire.
pub const FORMAT_VERSION: u32 = 1;

/// Ce qui fait échouer la lecture ou l'écriture d'un reçu.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    /// Lecture ou écriture impossible sur ce chemin.
    #[error("accès à {path} : {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Le fichier n'est pas un reçu TOML valide (ou n'a pas pu être écrit).
    #[error("reçu illisible : {0}")]
    Toml(String),
    /// Le reçu a été écrit par un Raftup plus récent.
    #[error("reçu de format {found}, ce Raftup comprend jusqu'au format {supported} : mettez Raftup à jour")]
    UnsupportedVersion { found: u32, supported: u32 },
    /// L'identifiant de jeu ne peut pas servir de nom de fichier.
    #[error("identifiant de jeu invalide : {0:?}")]
    InvalidGameId(String),
}

/// Où en est l'installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordState {
    /// Écriture commencée : le dossier du jeu peut être à moitié modifié.
    InProgress,
    /// Tout est installé et les hash sont notés.
    Complete,
}

/// Un original sauvegardé avant d'être remplacé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    /// Emplacement de la sauvegarde (relatif au dossier de sauvegarde du jeu).
    pub path: PathBuf,
    /// Hash de l'original, pour vérifier la sauvegarde avant de la restaurer.
    pub original_sha256: Sha256,
}

/// Un fichier écrit par une étape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// Chemin relatif à la racine de l'étape.
    pub path: PathBuf,
    /// Hash écrit une fois le fichier extrait ; `None` tant que l'état est `InProgress`.
    // `skip_serializing_if` : TOML n'a pas de « null », on n'écrit simplement pas la clé ;
    // `default` : à la lecture, une clé absente donne `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<Sha256>,
    /// L'original remplacé, s'il y en avait un.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<Backup>,
}

/// Ce qu'une étape du profil a posé dans le jeu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRecord {
    pub id: String,
    pub version: String,
    /// Dossier absolu de l'étape (`{game_dir}` ou `{worlds_dir}`) : les chemins de
    /// `files` et `dirs` sont relatifs à lui.
    pub root: PathBuf,
    /// Hash de l'archive téléchargée.
    pub sha256: Sha256,
    #[serde(default)]
    pub files: Vec<FileRecord>,
    /// Dossiers créés par l'étape (supprimés s'ils sont vides à la désinstallation).
    #[serde(default)]
    pub dirs: Vec<PathBuf>,
}

/// Le reçu d'un jeu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallRecord {
    pub format_version: u32,
    pub game_id: String,
    pub game_dir: PathBuf,
    pub state: RecordState,
    /// Dans l'ordre d'installation ; on désinstalle à l'envers.
    #[serde(default)]
    pub steps: Vec<StepRecord>,
}

/// Un identifiant sert de nom de fichier : on refuse tout ce qui pourrait sortir du dossier.
fn check_game_id(game_id: &str) -> Result<(), RecordError> {
    let ok = !game_id.is_empty()
        && game_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(RecordError::InvalidGameId(game_id.to_string()))
    }
}

fn record_file(paths: &Paths, game_id: &str) -> Result<PathBuf, RecordError> {
    check_game_id(game_id)?;
    Ok(paths.installs_dir().join(format!("{game_id}.toml")))
}

/// Raccourci pour fabriquer une erreur d'entrée/sortie avec son chemin.
fn io_err(path: &Path, source: std::io::Error) -> RecordError {
    RecordError::Io {
        path: path.to_path_buf(),
        source,
    }
}

impl InstallRecord {
    /// Un reçu neuf, `InProgress`, sans étape.
    pub fn new(game_id: &str, game_dir: PathBuf) -> InstallRecord {
        InstallRecord {
            format_version: FORMAT_VERSION,
            game_id: game_id.to_string(),
            game_dir,
            state: RecordState::InProgress,
            steps: Vec::new(),
        }
    }

    /// Lit le reçu d'un jeu ; `Ok(None)` s'il n'y en a pas (jeu non installé).
    pub fn load(paths: &Paths, game_id: &str) -> Result<Option<InstallRecord>, RecordError> {
        let file = record_file(paths, game_id)?;
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_err(&file, e)),
        };
        // On lit d'abord la seule version : un reçu futur peut avoir une forme que nous
        // ne savons pas décoder, et l'erreur utile est « mettez Raftup à jour ».
        let value: toml::Table =
            toml::from_str(&text).map_err(|e| RecordError::Toml(e.to_string()))?;
        let found = value
            .get("format_version")
            .and_then(toml::Value::as_integer)
            .ok_or_else(|| RecordError::Toml("format_version manquant".to_string()))?;
        let found = u32::try_from(found)
            .map_err(|_| RecordError::Toml("format_version hors limites".to_string()))?;
        if found > FORMAT_VERSION {
            return Err(RecordError::UnsupportedVersion {
                found,
                supported: FORMAT_VERSION,
            });
        }
        let record: InstallRecord =
            toml::from_str(&text).map_err(|e| RecordError::Toml(e.to_string()))?;
        Ok(Some(record))
    }

    /// Écrit le reçu de façon atomique : fichier `.tmp` voisin, puis renommage.
    /// Un plantage laisse donc l'ancien reçu intact, jamais un fichier à moitié écrit.
    pub fn save(&self, paths: &Paths) -> Result<(), RecordError> {
        let file = record_file(paths, &self.game_id)?;
        let text = toml::to_string_pretty(self).map_err(|e| RecordError::Toml(e.to_string()))?;
        let dir = paths.installs_dir();
        std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
        let tmp = dir.join(format!("{}.toml.tmp", self.game_id));
        // Même dossier que la cible : le renommage reste sur le même disque, donc atomique.
        let result = std::fs::write(&tmp, text)
            .map_err(|e| io_err(&tmp, e))
            .and_then(|()| std::fs::rename(&tmp, &file).map_err(|e| io_err(&file, e)));
        if result.is_err() {
            // Meilleur effort : on ne laisse pas de `.tmp` traîner (l'erreur d'origine prime).
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// Supprime le reçu (dernière étape d'une désinstallation). Un reçu absent n'est pas une erreur.
    pub fn remove(paths: &Paths, game_id: &str) -> Result<(), RecordError> {
        let file = record_file(paths, game_id)?;
        match std::fs::remove_file(&file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_err(&file, e)),
        }
    }
}
