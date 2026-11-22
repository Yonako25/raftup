//! Illustrations des jeux, choisies par le créateur du world (ADR 0051, qui remplace
//! l'ADR 0047).
//!
//! Le profil d'un jeu peut avoir une section `[art]` : l'adresse et l'empreinte de la
//! jaquette (`cover`) et de l'image large (`hero`), hébergées par le créateur. Comme
//! pour un apworld, le dépôt ne contient que l'adresse : Raftup ne redistribue aucune
//! image.
//!
//! - **À l'installation** du jeu, [`install_art`] télécharge ces images (hash vérifié)
//!   et les range dans `<données>/art/<jeu>/cover.jpg` (ou `.png`). Ensuite, elles
//!   s'affichent sans réseau.
//! - **À l'affichage**, [`ArtSources::find`] prend l'image installée, sinon celle que
//!   Steam garde déjà sur le disque du joueur (`appcache/librarycache/<appid>/`), sinon
//!   rien : l'interface dessine alors sa propre jaquette (aplat coloré et titre).
//!
//! Aucune API ni aucun appel réseau à l'affichage.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::download::{DownloadError, Downloader};
use crate::hash::Sha256;
use crate::paths::Paths;
use crate::profile::Profile;

/// Image : 8 Mio au plus.
pub const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;

/// Domaines permis pour les images : ceux des mods, plus les fichiers bruts de GitHub
/// (`raw.githubusercontent.com`), où les créateurs rangent souvent leurs visuels.
pub const ART_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "raw.githubusercontent.com",
    "thunderstore.io",
    "gcdn.thunderstore.io",
];

/// Sorte d'image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArtKind {
    /// Jaquette en portrait (environ 2:3), pour les cartes.
    Cover,
    /// Image large (16:9), pour le haut d'une fiche et la vitrine de l'Accueil.
    Hero,
}

impl ArtKind {
    /// Les deux sortes, dans un ordre fixe.
    pub const ALL: [ArtKind; 2] = [ArtKind::Cover, ArtKind::Hero];

    pub fn as_str(self) -> &'static str {
        match self {
            ArtKind::Cover => "cover",
            ArtKind::Hero => "hero",
        }
    }

    /// Nom du fichier que Steam garde pour cette sorte d'image.
    fn steam_file(self) -> &'static str {
        match self {
            ArtKind::Cover => "library_600x900.jpg",
            ArtKind::Hero => "library_hero.jpg",
        }
    }
}

/// Section `[art]` d'un profil : les images choisies par le créateur. Tout est
/// facultatif, donc `format_version` ne change pas.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtSection {
    #[serde(default)]
    pub cover: Option<ArtFile>,
    #[serde(default)]
    pub hero: Option<ArtFile>,
}

impl ArtSection {
    pub fn get(&self, kind: ArtKind) -> Option<&ArtFile> {
        match kind {
            ArtKind::Cover => self.cover.as_ref(),
            ArtKind::Hero => self.hero.as_ref(),
        }
    }
}

/// Une image : adresse HTTPS chez le créateur et empreinte obligatoire.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtFile {
    pub url: String,
    pub sha256: Sha256,
}

/// Ce qui empêche d'installer une image.
#[derive(Debug, Error)]
pub enum ArtError {
    #[error("téléchargement de l'image : {0}")]
    Download(#[from] DownloadError),
    #[error("image {path} : {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("le fichier de {0} n'est ni un JPEG ni un PNG")]
    NotAnImage(String),
}

/// Extension d'après les premiers octets : seuls JPEG et PNG sont acceptés.
pub fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("png")
    } else {
        None
    }
}

/// Télécharge et range les images de la section `[art]` du profil. Renvoie le nombre
/// d'images rangées. Les images déjà téléchargées viennent du cache des
/// téléchargements (rangé par sha256), sans réseau.
pub fn install_art(
    paths: &Paths,
    profile: &Profile,
    downloader: &Downloader,
) -> Result<usize, ArtError> {
    let dir = paths.art_dir(&profile.game.id);
    let mut count = 0;
    for kind in ArtKind::ALL {
        let Some(file) = profile.art.get(kind) else {
            continue;
        };
        let downloaded = downloader.get(&file.url, &file.sha256, MAX_IMAGE_BYTES, &mut |_| {})?;
        let bytes = fs::read(&downloaded).map_err(|source| ArtError::Io {
            path: downloaded.clone(),
            source,
        })?;
        let ext = image_extension(&bytes)
            .ok_or_else(|| ArtError::NotAnImage(kind.as_str().to_string()))?;
        fs::create_dir_all(&dir).map_err(|source| ArtError::Io {
            path: dir.clone(),
            source,
        })?;
        // Écriture dans un fichier à côté puis renommage : jamais d'image à moitié écrite.
        let target = dir.join(format!("{}.{ext}", kind.as_str()));
        let part = dir.join(format!("{}.part", kind.as_str()));
        fs::write(&part, &bytes).map_err(|source| ArtError::Io {
            path: part.clone(),
            source,
        })?;
        fs::rename(&part, &target).map_err(|source| ArtError::Io {
            path: target.clone(),
            source,
        })?;
        // Une ancienne image de l'autre format ne doit pas passer devant la nouvelle.
        let other = if ext == "jpg" { "png" } else { "jpg" };
        let _ = fs::remove_file(dir.join(format!("{}.{other}", kind.as_str())));
        count += 1;
    }
    Ok(count)
}

/// Image installée pour ce jeu, s'il y en a une.
pub fn installed_image(paths: &Paths, game_id: &str, kind: ArtKind) -> Option<PathBuf> {
    let dir = paths.art_dir(game_id);
    ["jpg", "png"]
        .iter()
        .map(|ext| dir.join(format!("{}.{ext}", kind.as_str())))
        .find(|path| path.is_file())
}

/// Image que Steam garde pour ce jeu dans `<steam>/appcache/librarycache/<appid>/`.
/// Selon la version de Steam, le fichier est dans ce dossier ou dans un sous-dossier.
pub fn steam_image(steam_dir: &Path, appid: u32, kind: ArtKind) -> Option<PathBuf> {
    let dir = steam_dir
        .join("appcache")
        .join("librarycache")
        .join(appid.to_string());
    let direct = dir.join(kind.steam_file());
    if direct.is_file() {
        return Some(direct);
    }
    // Sous-dossiers triés : le résultat ne dépend pas de l'ordre du disque.
    let mut subdirs: Vec<PathBuf> = fs::read_dir(&dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_dir())
        .collect();
    subdirs.sort();
    subdirs
        .into_iter()
        .map(|sub| sub.join(kind.steam_file()))
        .find(|path| path.is_file())
}

/// Lit une image du disque, au plus [`MAX_IMAGE_BYTES`] ; `None` si elle est absente,
/// trop grosse ou ni JPEG ni PNG.
pub fn read_image(path: &Path) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    // `take` : on ne lit jamais plus que la limite, même si le fichier grossit.
    file.take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return None;
    }
    image_extension(&bytes)?;
    Some(bytes)
}

/// Où chercher les images sur ce PC.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtSources {
    /// Dossiers de Raftup (images installées). `None` : pas d'images installées.
    pub paths: Option<Paths>,
    /// Dossier de Steam. `None` : Steam absent.
    pub steam_dir: Option<PathBuf>,
}

impl ArtSources {
    /// Octets de l'image : celle du créateur si elle est installée, sinon celle de
    /// Steam, sinon `None`.
    pub fn find(&self, game_id: &str, steam_appid: Option<u32>, kind: ArtKind) -> Option<Vec<u8>> {
        let installed = self
            .paths
            .as_ref()
            .and_then(|paths| installed_image(paths, game_id, kind))
            .and_then(|path| read_image(&path));
        installed.or_else(|| {
            let steam = self.steam_dir.as_ref()?;
            read_image(&steam_image(steam, steam_appid?, kind)?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0];
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0];

    #[test]
    fn formats_reconnus() {
        assert_eq!(image_extension(JPEG), Some("jpg"));
        assert_eq!(image_extension(PNG), Some("png"));
        assert_eq!(image_extension(b"GIF89a"), None);
        assert_eq!(image_extension(b""), None);
    }

    #[test]
    fn image_installee_avant_steam() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::under(&root.path().join("raftup"));
        let steam = root.path().join("steam");
        // Steam garde une jaquette (dans un sous-dossier) et une image large (à plat).
        let cache = steam.join("appcache").join("librarycache").join("433340");
        fs::create_dir_all(cache.join("abc")).unwrap();
        fs::write(cache.join("abc").join("library_600x900.jpg"), JPEG).unwrap();
        fs::write(cache.join("library_hero.jpg"), JPEG).unwrap();
        let sources = ArtSources {
            paths: Some(paths.clone()),
            steam_dir: Some(steam),
        };
        assert_eq!(
            sources.find("slime-rancher", Some(433340), ArtKind::Cover),
            Some(JPEG.to_vec())
        );
        assert!(sources
            .find("slime-rancher", Some(433340), ArtKind::Hero)
            .is_some());
        // Sans appid, Steam ne sert pas.
        assert_eq!(sources.find("slime-rancher", None, ArtKind::Cover), None);
        // L'image du créateur, une fois installée, passe devant celle de Steam.
        fs::create_dir_all(paths.art_dir("slime-rancher")).unwrap();
        fs::write(paths.art_dir("slime-rancher").join("cover.png"), PNG).unwrap();
        assert_eq!(
            sources.find("slime-rancher", Some(433340), ArtKind::Cover),
            Some(PNG.to_vec())
        );
    }

    #[test]
    fn fichier_qui_n_est_pas_une_image_ignore() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("cover.jpg");
        fs::write(&path, b"<html>pas une image</html>").unwrap();
        assert_eq!(read_image(&path), None);
        assert_eq!(read_image(&root.path().join("absent.jpg")), None);
    }
}
