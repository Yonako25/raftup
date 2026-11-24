//! Les parties générées sur ce PC, pour les reprendre (écran « En cours », revue
//! d'ergonomie du launcher, point 21).
//!
//! Chaque génération vit dans `generations/<horodatage>/` ; sa seed est extraite dans
//! `output/extracted/` (voir `generation::run`). Quand la partie a été hébergée, sa
//! sauvegarde `.raftup` est posée à côté de la seed (`server::save::default_save_path`) :
//! héberger de nouveau la même seed reprend la partie là où elle en était.
//!
//! Lecture seule : rien n'est créé, déplacé ni supprimé ici. Un dossier illisible ou sans
//! seed est simplement ignoré (le dossier `generations/salon/` des YAML de l'équipage, une
//! génération ratée…).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::paths::Paths;
use crate::seed::SeedSummary;

/// Une partie générée sur ce PC.
#[derive(Debug, Clone)]
pub struct PastGame {
    /// Le fichier `.archipelago` de la partie : c'est lui qu'on héberge pour la reprendre.
    pub seed: PathBuf,
    /// Résumé de la seed (joueurs, jeux).
    pub summary: SeedSummary,
    /// Dernière écriture de la sauvegarde ; `None` : la partie n'a jamais été hébergée.
    pub last_saved: Option<SystemTime>,
    /// Date de la génération (celle du fichier de seed).
    pub generated_at: Option<SystemTime>,
}

impl PastGame {
    /// Date de la dernière activité connue : la sauvegarde, sinon la génération.
    pub fn last_activity(&self) -> Option<SystemTime> {
        self.last_saved.or(self.generated_at)
    }
}

/// Les parties de `generations/`, de la plus récente à la plus ancienne (dernière
/// sauvegarde, sinon date de génération ; à égalité, par chemin, pour un ordre stable).
///
/// Lit chaque seed : à appeler hors du fil de l'interface.
pub fn list(paths: &Paths) -> Vec<PastGame> {
    let Ok(entries) = fs::read_dir(paths.generations_dir()) else {
        return Vec::new();
    };
    let mut games: Vec<PastGame> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|entry| read_generation(&entry.path()))
        .collect();
    // `cmp` sur `Option<SystemTime>` : `None` passe avant `Some`, donc en dernier une fois
    // l'ordre inversé (`b` comparé à `a`).
    games.sort_by(|a, b| {
        b.last_activity()
            .cmp(&a.last_activity())
            .then_with(|| a.seed.cmp(&b.seed))
    });
    games
}

/// Une génération : sa seed (la première `.archipelago` par nom, comme à l'extraction),
/// son résumé et ses dates. `None` si elle n'a pas de seed lisible.
fn read_generation(dir: &Path) -> Option<PastGame> {
    let seed = first_seed(&dir.join("output").join("extracted"))?;
    // `SeedSummary::load` rend aussi le multidata complet : seul le résumé sert ici.
    let (summary, _) = SeedSummary::load(&seed).ok()?;
    let modified = |path: &Path| fs::metadata(path).and_then(|m| m.modified()).ok();
    let last_saved = modified(&server::save::default_save_path(&seed));
    let generated_at = modified(&seed);
    Some(PastGame {
        seed,
        summary,
        last_saved,
        generated_at,
    })
}

/// La première seed `.archipelago` sous `dir` (sous-dossiers compris), par chemin.
fn first_seed(dir: &Path) -> Option<PathBuf> {
    let mut found = Vec::new();
    collect_seeds(dir, &mut found, 0);
    found.sort();
    found.into_iter().next()
}

/// Profondeur maximale parcourue sous `output/extracted/` : le zip d'Archipelago met la
/// seed à sa racine, quelques niveaux suffisent largement.
const MAX_DEPTH: u32 = 4;

fn collect_seeds(dir: &Path, found: &mut Vec<PathBuf>, depth: u32) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        // `file_type` ne suit pas les liens : un lien n'est ni parcouru ni pris.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect_seeds(&path, found, depth + 1);
        } else if kind.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("archipelago"))
        {
            found.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une vraie seed de test (MIT), copiée dans `generations/<nom>/output/extracted/`.
    fn generation(paths: &Paths, name: &str) -> PathBuf {
        let dir = paths
            .generations_dir()
            .join(name)
            .join("output")
            .join("extracted");
        fs::create_dir_all(&dir).unwrap();
        let seed = dir.join("AP_1234.archipelago");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../multidata/tests/data/seed-mit.archipelago");
        fs::copy(source, &seed).unwrap();
        seed
    }

    #[test]
    fn sans_dossier_aucune_partie() {
        let temp = tempfile::tempdir().unwrap();
        assert!(list(&Paths::under(temp.path())).is_empty());
    }

    #[test]
    fn parties_de_la_plus_recente_a_la_plus_ancienne() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::under(temp.path());
        let ancienne = generation(&paths, "100");
        let jamais = generation(&paths, "200");
        // Dossiers sans seed : ignorés.
        fs::create_dir_all(paths.generations_dir().join("salon")).unwrap();
        fs::create_dir_all(paths.generations_dir().join("300").join("output")).unwrap();
        // La plus ancienne a une sauvegarde écrite après la génération de l'autre.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let save = server::save::default_save_path(&ancienne);
        fs::write(&save, "{}\n").unwrap();

        let games = list(&paths);
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].seed, ancienne);
        assert!(games[0].last_saved.is_some());
        assert_eq!(games[1].seed, jamais);
        assert!(games[1].last_saved.is_none());
        assert!(games[1].generated_at.is_some());
        assert!(games[0].summary.players > 0);
    }
}
