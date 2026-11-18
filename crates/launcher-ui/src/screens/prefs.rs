//! Réglages mémorisés entre deux lancements : adresse du webhook Discord et préréglage de
//! génération (F10, F06). Ils vivent dans `UserConfig` (ADR 0025 pour le webhook, en clair).
//!
//! Les fonctions `apply` et `from_config` sont pures (testées sans disque) ; `load` et
//! `remember` font le va-et-vient avec `config.toml`. Une erreur de lecture ou d'écriture
//! n'empêche jamais d'héberger : on la note dans le journal, SANS l'adresse du webhook.

use launcher_core::config::{self, UserConfig};
use launcher_core::webhook::WebhookUrl;

use super::library::load_paths;
use super::presets::Preset;

/// Ce qui pré-remplit les formulaires « Héberger » et « Créer ».
// Pas de `Debug` : le champ `webhook` est secret (ADR 0025).
#[derive(Default, Clone, PartialEq, Eq)]
pub struct Prefs {
    /// Texte du champ webhook (vide : rien de mémorisé).
    pub webhook: String,
    pub preset: Preset,
}

/// Lit les réglages mémorisés dans la configuration.
pub fn from_config(config: &UserConfig) -> Prefs {
    Prefs {
        webhook: config.webhook_url.clone().unwrap_or_default(),
        preset: config
            .preset
            .as_deref()
            .and_then(preset_from_label)
            .unwrap_or_default(),
    }
}

/// Retrouve un préréglage par son nom affiché ; un nom inconnu (fichier édité à la main,
/// préréglage retiré dans une version future) donne `None`, donc le préréglage par défaut.
fn preset_from_label(label: &str) -> Option<Preset> {
    Preset::ALL.into_iter().find(|p| p.label() == label)
}

/// Range le webhook (`None` = champ vidé : la valeur mémorisée est effacée) et, si on la
/// connaît, le préréglage. Rend vrai si la configuration a changé.
pub fn apply(
    config: &mut UserConfig,
    webhook: Option<&WebhookUrl>,
    preset: Option<Preset>,
) -> bool {
    let mut changed = config.set_webhook(webhook.map(WebhookUrl::expose));
    if let Some(preset) = preset {
        changed |= config.set_preset(preset.label());
    }
    changed
}

/// Charge les réglages mémorisés ; en cas de problème, des valeurs vides.
pub fn load() -> Prefs {
    let Ok(paths) = load_paths() else {
        return Prefs::default();
    };
    match config::load(&paths.config_file()) {
        Ok(loaded) => from_config(&loaded.config),
        Err(e) => {
            tracing::warn!("réglages mémorisés non lus : {e}");
            Prefs::default()
        }
    }
}

/// Mémorise le webhook et le préréglage sur le disque (seulement si quelque chose change).
pub fn remember(webhook: Option<&WebhookUrl>, preset: Option<Preset>) {
    let Ok(paths) = load_paths() else {
        return;
    };
    let file = paths.config_file();
    let result = config::load(&file).and_then(|loaded| {
        let mut config = loaded.config;
        if apply(&mut config, webhook, preset) {
            config.save(&file)?;
        }
        Ok(())
    });
    if let Err(e) = result {
        // `ConfigError` ne contient que des chemins et des messages système, jamais l'adresse.
        tracing::warn!("réglages non enregistrés : {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> WebhookUrl {
        WebhookUrl::new(s).unwrap()
    }

    #[test]
    fn config_vide_donne_champ_vide_et_preset_par_defaut() {
        let prefs = from_config(&UserConfig::default());
        assert_eq!(prefs.webhook, "");
        assert_eq!(prefs.preset, Preset::Standard);
    }

    #[test]
    fn les_reglages_memorises_preremplissent() {
        let mut config = UserConfig::default();
        let hook = url("https://discord.com/api/webhooks/1/abc");
        assert!(apply(&mut config, Some(&hook), Some(Preset::Long)));
        let prefs = from_config(&config);
        assert_eq!(prefs.webhook, "https://discord.com/api/webhooks/1/abc");
        assert_eq!(prefs.preset, Preset::Long);
    }

    #[test]
    fn un_champ_vide_efface_la_valeur_memorisee() {
        let mut config = UserConfig::default();
        let hook = url("https://discord.com/api/webhooks/1/abc");
        apply(&mut config, Some(&hook), Some(Preset::Rapide));
        assert!(apply(&mut config, None, Some(Preset::Rapide)));
        assert_eq!(config.webhook_url, None);
        // Le préréglage, lui, reste.
        assert_eq!(from_config(&config).preset, Preset::Rapide);
    }

    #[test]
    fn rien_ne_change_rien_a_ecrire() {
        let mut config = UserConfig::default();
        let hook = url("https://discord.com/api/webhooks/1/abc");
        apply(&mut config, Some(&hook), Some(Preset::Long));
        assert!(!apply(&mut config, Some(&hook), Some(Preset::Long)));
        // L'écran « Héberger » n'a pas de préréglage : il ne touche pas à celui mémorisé.
        assert!(!apply(&mut config, Some(&hook), None));
        assert_eq!(from_config(&config).preset, Preset::Long);
    }

    #[test]
    fn preset_inconnu_retombe_sur_le_defaut() {
        let config = UserConfig {
            preset: Some("Délirant".into()),
            ..UserConfig::default()
        };
        assert_eq!(from_config(&config).preset, Preset::Standard);
    }
}
