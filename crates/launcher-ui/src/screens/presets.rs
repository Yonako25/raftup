//! Presets de génération « Rapide / Standard / Long » (F06, ADR 0023, proposé).
//!
//! Module pur (aucun dessin, aucun accès disque) : un preset est une petite table de
//! réglages GÉNÉRIQUES de la demande de génération, communs à tous les jeux. Il
//! n'invente aucune option propre à un jeu et ne réécrit aucun YAML de joueur.
//!
//! Un preset règle le délai maximal de génération et, sauf Standard, un `meta.yaml`
//! (`progression_balancing`, `accessibility`) que le générateur applique à tous les
//! joueurs (voir l'ADR 0023).

use std::time::Duration;

use launcher_core::generation::{GenerateRequest, DEFAULT_TIMEOUT};

/// Le preset choisi dans « Créer une partie ».
// `Copy` : un petit enum sans données se copie comme un entier, pas besoin de `clone()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    Rapide,
    /// Équivalent des valeurs par défaut de `GenerateRequest` : ne change rien.
    #[default]
    Standard,
    Long,
}

impl Preset {
    /// Tous les presets, dans l'ordre d'affichage.
    pub const ALL: [Preset; 3] = [Preset::Rapide, Preset::Standard, Preset::Long];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Rapide => "Rapide",
            Preset::Standard => "Standard",
            Preset::Long => "Long",
        }
    }

    /// Phrase d'aide sous le sélecteur.
    pub fn description(self) -> &'static str {
        match self {
            Preset::Rapide => "Délai de 5 minutes, progression équilibrée et accès minimal : pour de petites parties.",
            Preset::Standard => "Délai de 15 minutes, valeur par défaut.",
            Preset::Long => "Délai de 45 minutes, progression moins forcée et accès complet : grosses parties.",
        }
    }

    /// Délai maximal de génération du preset.
    pub fn timeout(self) -> Duration {
        match self {
            Preset::Rapide => Duration::from_secs(5 * 60),
            Preset::Standard => DEFAULT_TIMEOUT,
            Preset::Long => Duration::from_secs(45 * 60),
        }
    }

    /// Applique le preset à la demande ; les fichiers et la seed restent intacts.
    pub fn apply(self, request: &mut GenerateRequest) {
        request.timeout = self.timeout();
        request.meta_yaml = meta_yaml(self);
    }
}

/// `meta.yaml` du preset, ou `None` pour Standard (aucun réglage global imposé).
///
/// Format réel d'Archipelago 0.6.8 (`Generate.py`) : `meta_description` est obligatoire ;
/// la catégorie `null:` (clé YAML nulle) vaut pour TOUS les jeux et n'accepte que les
/// options communes (`progression_balancing` : 0 à 99 ; `accessibility` : `full` ou
/// `minimal`). Ces valeurs écrasent celles des YAML des joueurs.
pub fn meta_yaml(preset: Preset) -> Option<String> {
    let (balancing, accessibility, description) = match preset {
        // Pas de « : » dans la description : en YAML, « a: b : c » est refusé (par
        // PyYAML aussi), et tout le meta.yaml serait illisible.
        Preset::Rapide => (80, "minimal", "Raftup, partie rapide"),
        Preset::Standard => return None,
        Preset::Long => (20, "full", "Raftup, partie longue"),
    };
    Some(format!(
        "meta_description: {description}\nnull:\n  progression_balancing: {balancing}\n  accessibility: {accessibility}\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_ne_change_rien() {
        let mut request = GenerateRequest::default();
        Preset::Standard.apply(&mut request);
        assert_eq!(request.timeout, GenerateRequest::default().timeout);
    }

    #[test]
    fn meta_yaml_par_preset() {
        assert_eq!(meta_yaml(Preset::Standard), None);
        let rapide = meta_yaml(Preset::Rapide).expect("meta rapide");
        assert!(rapide.starts_with("meta_description: "));
        assert!(rapide.contains("\nnull:\n"));
        assert!(rapide.contains("progression_balancing: 80\n"));
        assert!(rapide.contains("accessibility: minimal\n"));
        let long = meta_yaml(Preset::Long).expect("meta long");
        assert!(long.contains("progression_balancing: 20\n"));
        assert!(long.contains("accessibility: full\n"));
    }

    #[test]
    fn apply_pose_le_meta() {
        let mut request = GenerateRequest::default();
        Preset::Rapide.apply(&mut request);
        assert_eq!(request.meta_yaml, meta_yaml(Preset::Rapide));
        Preset::Standard.apply(&mut request);
        assert_eq!(request.meta_yaml, None);
    }

    #[test]
    fn delais_croissants() {
        assert!(Preset::Rapide.timeout() < Preset::Standard.timeout());
        assert!(Preset::Standard.timeout() < Preset::Long.timeout());
    }

    #[test]
    fn apply_garde_fichiers_et_seed() {
        let mut request = GenerateRequest {
            player_files: vec!["a.yaml".into()],
            seed: Some(7),
            ..GenerateRequest::default()
        };
        Preset::Long.apply(&mut request);
        assert_eq!(request.player_files.len(), 1);
        assert_eq!(request.seed, Some(7));
        assert_eq!(request.timeout, Preset::Long.timeout());
    }

    #[test]
    fn defaut_standard_et_libelles_distincts() {
        assert_eq!(Preset::default(), Preset::Standard);
        let mut labels: Vec<_> = Preset::ALL.iter().map(|p| p.label()).collect();
        labels.dedup();
        assert_eq!(labels.len(), 3);
    }
}
