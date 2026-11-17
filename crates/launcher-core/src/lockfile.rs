//! Versions figées par l'hôte pour une session, sans aucune URL ni chemin (ADR 0015, décision 5).
//!
//! Le lockfile est un fichier `<seed>.raftup-lock.toml` posé à côté de la seed. Il dit quels
//! mods (identifiant, version, sha256 de l'archive) font partie de la partie, pour que chaque
//! joueur puisse vérifier qu'il a les mêmes. Il ne contient **jamais** d'URL ni de chemin local :
//! les types ci-dessous n'ont tout simplement pas de tel champ, et `deny_unknown_fields`
//! refuse un fichier qui en ajouterait.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hash::Sha256;
use crate::profile::Profile;
use crate::record::{InstallRecord, RecordState};

/// Version du format de lockfile que ce launcher sait lire et écrire.
pub const FORMAT_VERSION: u32 = 1;

/// Ce qui fait échouer la lecture ou l'écriture d'un lockfile.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// Lecture ou écriture impossible sur ce chemin.
    #[error("accès à {path} : {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Le fichier n'est pas un lockfile TOML valide (ou n'a pas pu être écrit).
    #[error("lockfile illisible : {0}")]
    Toml(String),
    /// Le lockfile vient d'un Raftup plus récent.
    #[error("lockfile de format {found}, ce Raftup comprend jusqu'au format {supported} : mettez Raftup à jour")]
    UnsupportedVersion { found: u32, supported: u32 },
    /// Contenu incohérent (identifiant vide, mod en double…).
    #[error("lockfile invalide : {0}")]
    Invalid(String),
}

/// Un mod figé : ni URL, ni chemin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedMod {
    /// Identifiant de l'étape dans le profil.
    pub id: String,
    pub version: String,
    /// Hash de l'archive téléchargée.
    pub sha256: Sha256,
}

/// Les versions figées d'une partie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lockfile {
    pub format_version: u32,
    pub game_id: String,
    /// Dans l'ordre d'installation du profil.
    #[serde(default)]
    pub mods: Vec<LockedMod>,
}

/// Un écart entre le lockfile de l'hôte et ce qui est installé chez le joueur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    /// Le lockfile est celui d'un autre jeu que le reçu.
    WrongGame { locked: String, installed: String },
    /// Aucune installation (pas de reçu) : tout manque.
    NotInstalled,
    /// L'installation est restée à moitié faite.
    InstallIncomplete,
    /// Le mod est demandé par l'hôte mais n'est pas installé.
    Missing { id: String },
    /// Le mod est installé dans une autre version.
    VersionDiffers {
        id: String,
        locked: String,
        installed: String,
    },
    /// Même version annoncée, mais l'archive n'a pas le même hash.
    HashDiffers {
        id: String,
        locked: Sha256,
        installed: Sha256,
    },
    /// Le mod est installé mais l'hôte ne l'a pas figé.
    Extra { id: String },
}

/// Le résultat de la comparaison, dans un ordre stable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LockReport {
    pub mismatches: Vec<Mismatch>,
}

impl LockReport {
    /// Vrai si l'installation correspond exactement au lockfile.
    pub fn is_match(&self) -> bool {
        self.mismatches.is_empty()
    }
}

/// Où poser le lockfile d'une seed : `<seed>.raftup-lock.toml` à côté d'elle.
/// On ajoute le suffixe au nom complet (`partie.archipelago` devient
/// `partie.archipelago.raftup-lock.toml`) au lieu de remplacer l'extension.
pub fn path_for_seed(seed: &Path) -> PathBuf {
    // `OsString` : un nom de fichier n'est pas forcément de l'UTF-8 valide, donc on évite `String`.
    let mut name: OsString = seed.as_os_str().to_owned();
    name.push(".raftup-lock.toml");
    PathBuf::from(name)
}

fn io_err(path: &Path, source: std::io::Error) -> LockError {
    LockError::Io {
        path: path.to_path_buf(),
        source,
    }
}

impl Lockfile {
    /// Un lockfile vide pour un jeu.
    pub fn new(game_id: &str) -> Lockfile {
        Lockfile {
            format_version: FORMAT_VERSION,
            game_id: game_id.to_string(),
            mods: Vec::new(),
        }
    }

    /// Fige les versions décrites par un profil (l'hôte, avant la génération).
    pub fn from_profile(profile: &Profile) -> Lockfile {
        let mut lock = Lockfile::new(&profile.game.id);
        lock.mods = profile
            .steps
            .iter()
            .map(|step| LockedMod {
                id: step.id.clone(),
                version: step.version.clone(),
                sha256: step.sha256,
            })
            .collect();
        lock
    }

    /// Lit un lockfile et le valide.
    pub fn load(path: &Path) -> Result<Lockfile, LockError> {
        let text = fs::read_to_string(path).map_err(|e| io_err(path, e))?;
        // On lit d'abord la seule version : un lockfile futur peut avoir d'autres champs.
        let raw: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| LockError::Toml(e.to_string()))?;
        if let Some(found) = raw.get("format_version").and_then(|v| v.as_integer()) {
            let found = u32::try_from(found).unwrap_or(u32::MAX);
            if found > FORMAT_VERSION {
                return Err(LockError::UnsupportedVersion {
                    found,
                    supported: FORMAT_VERSION,
                });
            }
        }
        let lock: Lockfile = raw
            .try_into()
            .map_err(|e: toml::de::Error| LockError::Toml(e.to_string()))?;
        lock.validate()?;
        Ok(lock)
    }

    /// Écrit le lockfile de façon atomique : `.tmp` puis renommage.
    pub fn save(&self, path: &Path) -> Result<(), LockError> {
        self.validate()?;
        let text = toml::to_string_pretty(self).map_err(|e| LockError::Toml(e.to_string()))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        }
        let mut tmp_name: OsString = path.as_os_str().to_owned();
        tmp_name.push(".tmp");
        let tmp = PathBuf::from(tmp_name);
        if let Err(e) = fs::write(&tmp, text) {
            let _ = fs::remove_file(&tmp);
            return Err(io_err(&tmp, e));
        }
        fs::rename(&tmp, path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            io_err(path, e)
        })
    }

    fn validate(&self) -> Result<(), LockError> {
        if self.game_id.is_empty() {
            return Err(LockError::Invalid("game_id vide".into()));
        }
        let mut seen = BTreeMap::new();
        for locked in &self.mods {
            if locked.id.is_empty() || locked.version.is_empty() {
                return Err(LockError::Invalid("mod sans id ou sans version".into()));
            }
            if seen.insert(locked.id.as_str(), ()).is_some() {
                return Err(LockError::Invalid(format!("mod en double : {}", locked.id)));
            }
        }
        Ok(())
    }

    /// Compare le lockfile avec le reçu d'installation du joueur (`None` : jeu non installé).
    ///
    /// Les écarts sont rendus dans un ordre stable : d'abord ceux du lockfile (dans son ordre),
    /// puis les mods installés en trop, triés par identifiant.
    pub fn compare(&self, record: Option<&InstallRecord>) -> LockReport {
        let mut mismatches = Vec::new();
        let Some(record) = record else {
            mismatches.push(Mismatch::NotInstalled);
            return LockReport { mismatches };
        };
        if record.game_id != self.game_id {
            mismatches.push(Mismatch::WrongGame {
                locked: self.game_id.clone(),
                installed: record.game_id.clone(),
            });
            return LockReport { mismatches };
        }
        if record.state != RecordState::Complete {
            mismatches.push(Mismatch::InstallIncomplete);
        }

        // Étapes installées, rangées par id pour une recherche directe.
        let installed: BTreeMap<&str, &crate::record::StepRecord> =
            record.steps.iter().map(|s| (s.id.as_str(), s)).collect();

        for locked in &self.mods {
            match installed.get(locked.id.as_str()) {
                None => mismatches.push(Mismatch::Missing {
                    id: locked.id.clone(),
                }),
                Some(step) if step.version != locked.version => {
                    mismatches.push(Mismatch::VersionDiffers {
                        id: locked.id.clone(),
                        locked: locked.version.clone(),
                        installed: step.version.clone(),
                    })
                }
                Some(step) if step.sha256 != locked.sha256 => {
                    mismatches.push(Mismatch::HashDiffers {
                        id: locked.id.clone(),
                        locked: locked.sha256,
                        installed: step.sha256,
                    })
                }
                Some(_) => {}
            }
        }
        // `BTreeMap` : parcours trié par id.
        for id in installed.keys() {
            if !self.mods.iter().any(|m| m.id == *id) {
                mismatches.push(Mismatch::Extra {
                    id: (*id).to_string(),
                });
            }
        }
        LockReport { mismatches }
    }
}
