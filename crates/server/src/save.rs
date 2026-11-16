//! Ouverture de la sauvegarde `.raftup` d'une seed (ADR 0014), partagée par
//! le binaire `raftup-server` et le launcher.
//!
//! Si le fichier existe, la partie reprend ; sinon, un fichier neuf est créé.
//! Une sauvegarde d'une autre seed, d'un format inconnu ou corrompue au
//! milieu est refusée : on ne repart jamais d'une partie vide par erreur.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use multidata::raftup::{seed_sha256, SaveError, SaveFile, SaveHeader, Snapshot};
use tracing::{info, warn};

/// Sauvegarde ouverte, prête pour [`crate::Server::with_save`].
pub struct OpenedSave {
    pub file: SaveFile,
    /// État contenu dans le fichier (`Snapshot::default()` s'il est neuf).
    pub snapshot: Snapshot,
    /// Vrai si une partie existante reprend.
    pub resumed: bool,
}

/// Échec d'ouverture ou de création de la sauvegarde.
#[derive(Debug, thiserror::Error)]
pub enum SaveSetupError {
    #[error("reprise impossible de {path} : {source}")]
    Open { path: String, source: SaveError },
    #[error("création impossible de {path} : {source}")]
    Create { path: String, source: SaveError },
}

/// Chemin par défaut : à côté de la seed, avec l'extension `.raftup`
/// (comme le `.apsave` du serveur officiel).
pub fn default_save_path(seed: &Path) -> PathBuf {
    seed.with_extension("raftup")
}

/// Ouvre la sauvegarde `path` de cette seed, ou la crée.
///
/// `seed_bytes` : le fichier `.archipelago` tel quel, pour son empreinte.
pub fn open_or_create(
    path: &Path,
    seed_bytes: &[u8],
    seed_name: &str,
) -> Result<OpenedSave, SaveSetupError> {
    let shown = path.display().to_string();
    let sha = seed_sha256(seed_bytes);
    if path.exists() {
        let (file, snapshot) =
            SaveFile::open(path, &sha).map_err(|source| SaveSetupError::Open {
                path: shown.clone(),
                source,
            })?;
        if file.ignored_tail() {
            warn!("dernière ligne de la sauvegarde tronquée (coupure ?) : ignorée");
        }
        info!(save = %shown, "partie reprise");
        Ok(OpenedSave {
            file,
            snapshot,
            resumed: true,
        })
    } else {
        // Secondes depuis 1970, comme `time.time()` en Python. Une horloge
        // avant 1970 donne 0 : sans gravité, c'est seulement informatif.
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let header = SaveHeader::new(env!("CARGO_PKG_VERSION"), seed_name, &sha, created);
        let file = SaveFile::create(path, header).map_err(|source| SaveSetupError::Create {
            path: shown.clone(),
            source,
        })?;
        info!(save = %shown, "nouvelle sauvegarde");
        Ok(OpenedSave {
            file,
            snapshot: Snapshot::default(),
            resumed: false,
        })
    }
}
