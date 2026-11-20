//! Les dossiers de Raftup (config, données, cache), testables dans un dossier temporaire.

use std::path::{Path, PathBuf};

use directories::BaseDirs;

/// Ce qui empêche de trouver ou de préparer un dossier de Raftup.
#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    /// Lecture ou écriture impossible sur ce chemin.
    #[error("accès à {path} : {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Le système ne donne pas les dossiers de l'utilisateur.
    #[error("impossible de trouver le dossier personnel de l'utilisateur")]
    NoHomeDir,
}

/// Les trois racines de Raftup. Tout le reste s'en déduit par les accesseurs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Configuration utilisateur (`%APPDATA%\Raftup` sous Windows).
    pub config_dir: PathBuf,
    /// Données (`%LOCALAPPDATA%\Raftup`).
    pub data_dir: PathBuf,
    /// Cache (`data_dir/cache`).
    pub cache_dir: PathBuf,
}

impl Paths {
    /// Les vrais dossiers du système.
    pub fn from_os() -> Result<Paths, PathsError> {
        let base = BaseDirs::new().ok_or(PathsError::NoHomeDir)?;
        let data_dir = base.data_local_dir().join("Raftup");
        Ok(Paths {
            config_dir: base.config_dir().join("Raftup"),
            cache_dir: data_dir.join("cache"),
            data_dir,
        })
    }

    /// Tout sous `root` : pratique pour les tests avec un dossier temporaire.
    // `&Path` : on emprunte le chemin, l'appelant le garde ; `join` en fabrique des copies.
    pub fn under(root: &Path) -> Paths {
        let data_dir = root.join("data");
        Paths {
            config_dir: root.join("config"),
            cache_dir: data_dir.join("cache"),
            data_dir,
        }
    }

    /// `config.toml` de l'utilisateur.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// Dossier des reçus d'installation (un fichier par jeu).
    pub fn installs_dir(&self) -> PathBuf {
        self.data_dir.join("installs")
    }

    /// Originaux sauvegardés pour un jeu.
    pub fn backups_dir(&self, game_id: &str) -> PathBuf {
        self.data_dir.join("backups").join(game_id)
    }

    /// Cache des téléchargements (rangés par sha256).
    pub fn downloads_dir(&self) -> PathBuf {
        self.cache_dir.join("downloads")
    }

    /// Verrou et boîte aux lettres de l'instance unique du launcher.
    pub fn instance_dir(&self) -> PathBuf {
        self.data_dir.join("instance")
    }

    /// Dossier des générations (un sous-dossier horodaté par génération).
    pub fn generations_dir(&self) -> PathBuf {
        self.data_dir.join("generations")
    }

    /// Python autonome du générateur, un dossier par version épinglée (`python.exe` dedans).
    pub fn python_dir(&self, tag: &str) -> PathBuf {
        self.data_dir.join("python").join(tag)
    }

    /// Générateur installé pour une version d'Archipelago : `archipelago/`, `site-packages/`
    /// et le marqueur `runtime.json`.
    pub fn generator_dir(&self, ap_version: &str) -> PathBuf {
        self.data_dir.join("generator").join(ap_version)
    }

    /// Apworlds pour une version d'Archipelago.
    pub fn worlds_dir(&self, ap_version: &str) -> PathBuf {
        self.data_dir.join("worlds").join(ap_version)
    }

    /// Mondes natifs (ADR 0041) : fichiers `.toml` ou dossiers avec un `world.toml`.
    pub fn native_worlds_dir(&self) -> PathBuf {
        self.data_dir.join("mondes-natifs")
    }
}
