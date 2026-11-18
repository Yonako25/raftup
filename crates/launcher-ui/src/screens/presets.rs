//! Presets de génération « Rapide / Standard / Long » (F06, ADR 0023, proposé).
//!
//! Module pur (aucun dessin, aucun accès disque) : un preset est une petite table de
//! réglages GÉNÉRIQUES de la demande de génération, communs à tous les jeux. Il
//! n'invente aucune option propre à un jeu et ne réécrit aucun YAML de joueur.
//!
//! Aujourd'hui `GenerateRequest` n'expose que `timeout`, `spoiler` et `race` : seul
//! le délai maximal de génération varie donc entre les presets. Les réglages de
//! durée et de progression (`progression_balancing`, `accessibility`) passeraient par
//! un `meta.yaml` côté générateur, qui n'existe pas encore (voir l'ADR 0023).

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
            Preset::Rapide => "Abandonne la génération après 5 minutes : pour de petites parties.",
            Preset::Standard => "Délai de 15 minutes, valeur par défaut.",
            Preset::Long => "Laisse 45 minutes à la génération : grosses parties, nombreux jeux.",
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
    }
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
