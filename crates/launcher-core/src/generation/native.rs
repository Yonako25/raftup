//! Mondes natifs installés (ADR 0041) : fichiers TOML posés dans le dossier
//! `mondes-natifs/` des données de Raftup, lus par la crate `worlds-native`.
//!
//! Le dossier contient des fichiers `.toml` ou des sous-dossiers avec un `world.toml`.
//! Un monde illisible est mis de côté avec la raison, sans empêcher les autres de servir ;
//! deux mondes du même jeu : le premier (par ordre alphabétique du chemin) l'emporte.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use worlds_native::NativeWorld;

/// Mondes natifs lus, et ceux qui ont été écartés.
#[derive(Debug, Clone, Default)]
pub struct NativeWorlds {
    pub worlds: Vec<Arc<NativeWorld>>,
    /// `(chemin, raison)` de chaque monde écarté.
    pub rejected: Vec<(PathBuf, String)>,
}

/// Nombre maximal de mondes lus dans le dossier (chaque lecture est déjà bornée).
pub const MAX_NATIVE_WORLDS: usize = 256;

/// Lit les mondes natifs du dossier. Un dossier absent donne une liste vide.
pub fn load_native_worlds(dir: &Path) -> NativeWorlds {
    let (worlds, rejected) = scan_native_worlds(dir);
    NativeWorlds {
        worlds: worlds
            .into_iter()
            .map(|(_, world)| Arc::new(world))
            .collect(),
        rejected,
    }
}

/// Comme [`load_native_worlds`], mais garde le chemin de chaque monde retenu (fichier
/// `.toml` ou dossier) : la Bibliothèque en a besoin pour la fiche et la désinstallation.
/// Rend `(mondes retenus, mondes écartés avec la raison)`, dans l'ordre des chemins.
#[allow(clippy::type_complexity)]
pub fn scan_native_worlds(dir: &Path) -> (Vec<(PathBuf, NativeWorld)>, Vec<(PathBuf, String)>) {
    let mut worlds: Vec<(PathBuf, NativeWorld)> = Vec::new();
    let mut rejected = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (worlds, rejected);
    };
    let mut candidates: Vec<PathBuf> = entries
        // `flatten` saute les entrées illisibles.
        .flatten()
        .filter(|entry| {
            // `file_type` ne suit pas les liens : un lien n'est ni un fichier ni un dossier.
            let Ok(kind) = entry.file_type() else {
                return false;
            };
            let path = entry.path();
            (kind.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("toml")))
                || (kind.is_dir() && path.join("world.toml").is_file())
        })
        .map(|entry| entry.path())
        .collect();
    // Ordre stable d'un système à l'autre.
    candidates.sort();
    for path in candidates.into_iter().take(MAX_NATIVE_WORLDS) {
        match worlds_native::load_path(&path) {
            Ok(world) => {
                if let Some((_, first)) = worlds.iter().find(|(_, w)| w.game == world.game) {
                    let reason = format!(
                        "le jeu « {} » est déjà fourni par un autre monde",
                        first.game
                    );
                    rejected.push((path, reason));
                } else {
                    worlds.push((path, world));
                }
            }
            Err(e) => rejected.push((path, e.to_string())),
        }
    }
    (worlds, rejected)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{load_native_worlds, scan_native_worlds};

    const MINIMAL: &str = include_str!("../../../../spec/monde-natif/exemples/minimal/world.toml");

    #[test]
    fn dossier_absent_ou_vide() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_native_worlds(&dir.path().join("absent"))
            .worlds
            .is_empty());
        assert!(load_native_worlds(dir.path()).worlds.is_empty());
    }

    #[test]
    fn fichiers_dossiers_et_rejets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.toml"), MINIMAL).unwrap();
        // Même jeu une deuxième fois, dans un dossier : écarté.
        std::fs::create_dir(dir.path().join("b")).unwrap();
        std::fs::write(dir.path().join("b").join("world.toml"), MINIMAL).unwrap();
        // Un autre jeu, valide.
        let other = MINIMAL.replace("name = \"Minimal\"", "name = \"Autre\"");
        std::fs::write(dir.path().join("c.TOML"), other).unwrap();
        // Illisible.
        std::fs::write(dir.path().join("d.toml"), "format_version = 9").unwrap();
        // Ignorés : autre extension, dossier sans world.toml.
        std::fs::write(dir.path().join("e.txt"), MINIMAL).unwrap();
        std::fs::create_dir(dir.path().join("f")).unwrap();

        let found = load_native_worlds(dir.path());
        let games: Vec<&str> = found.worlds.iter().map(|w| w.game.as_str()).collect();
        assert_eq!(games, ["Minimal", "Autre"]);
        assert_eq!(found.rejected.len(), 2);
        assert!(found.rejected[0].1.contains("déjà fourni"));
        assert!(found.rejected[1].1.contains("format_version 9"));

        // La version qui garde les chemins voit les mêmes mondes.
        let (worlds, rejected) = scan_native_worlds(dir.path());
        let paths: Vec<_> = worlds.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(
            paths,
            [dir.path().join("a.toml"), dir.path().join("c.TOML")]
        );
        assert_eq!(rejected.len(), 2);
    }
}
