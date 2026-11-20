//! Panneau « Durée estimée » de l'écran « Créer une partie » (ADR 0030).
//!
//! Dès qu'une seed est générée, on la relit (lecture sûre de `multidata`) et on estime la
//! durée de chaque joueur avec `launcher_core::estimate`. Le résultat est gardé dans la
//! mémoire d'egui (`ui.data`) : le panneau reste affiché, pour la dernière partie
//! générée, même après le démarrage du serveur, quand l'hôte revient sur l'écran.
//!
//! L'écran « Partie en cours » relit la même mémoire ([`show_running`]) : si la partie
//! hébergée est la dernière seed générée, il montre aussi l'estimation ; sinon (seed
//! ouverte depuis un fichier, launcher relancé), il ne montre rien.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Id, RichText, Ui};
use launcher_core::generation::GenerateOutput;
use launcher_core::{estimate_file, Catalog, PlayerEstimate, SeedEstimate};

use crate::format;
use crate::widgets;

/// La dernière estimation calculée, et la seed d'où elle vient.
struct Remembered {
    file: PathBuf,
    seed_name: String,
    estimate: Result<SeedEstimate, String>,
}

/// Clé de l'estimation dans la mémoire d'egui.
fn key() -> Id {
    Id::unique("raftup-duree-estimee")
}

/// Calcule l'estimation de la seed tout juste générée (une seule fois par seed), puis
/// dessine la dernière estimation connue. Ne dessine rien avant la première génération.
pub fn show(ui: &mut Ui, output: Option<&GenerateOutput>) {
    // `Arc` : egui garde une copie de la valeur ; avec un `Arc`, la copie ne coûte qu'un
    // compteur, pas toute l'estimation.
    let remembered: Option<Arc<Remembered>> = ui.data(|d| d.get_temp(key()));
    if let Some(output) = output {
        let known = remembered
            .as_ref()
            .is_some_and(|r| r.file == output.archipelago_file);
        if !known {
            let estimate = Catalog::embedded()
                .map_err(|e| e.to_string())
                .and_then(|catalog| {
                    estimate_file(&output.archipelago_file, &catalog).map_err(|e| e.to_string())
                });
            let fresh = Arc::new(Remembered {
                file: output.archipelago_file.clone(),
                seed_name: output.seed_name.clone(),
                estimate,
            });
            ui.data_mut(|d| d.insert_temp(key(), fresh));
        }
    }
    let Some(remembered) = ui.data(|d| d.get_temp::<Arc<Remembered>>(key())) else {
        return;
    };
    widgets::panel(ui, "Durée estimée", |ui| match &remembered.estimate {
        Ok(estimate) => {
            ui.label(widgets::dim(format!(
                "Seed {} · {}",
                remembered.seed_name,
                summary(estimate)
            )));
            ui.add_space(4.0);
            for player in &estimate.players {
                ui.label(player_line(player));
            }
            ui.add_space(4.0);
            ui.label(widgets::dim(NOTE).size(12.0));
        }
        Err(message) => {
            ui.label(widgets::dim(format!("Estimation impossible : {message}")));
        }
    });
    ui.add_space(10.0);
}

/// Pour « Partie en cours » : l'estimation calculée à la création, si la partie
/// hébergée est cette seed-là et que le calcul a réussi ; `None` sinon.
pub fn for_seed(ui: &Ui, seed_name: &str) -> Option<SeedEstimate> {
    let remembered: Option<Arc<Remembered>> = ui.data(|d| d.get_temp(key()));
    known_estimate(remembered.as_deref(), seed_name).cloned()
}

/// Contenu du panneau « Durée estimée » de « Partie en cours » (colonne de droite).
pub fn running_panel(ui: &mut Ui, estimate: &SeedEstimate) {
    ui.label(widgets::caps("Durée estimée"));
    ui.add_space(4.0);
    ui.label(widgets::dim(summary(estimate)).size(13.0));
    ui.add_space(4.0);
    for player in &estimate.players {
        ui.label(RichText::new(short_player_line(player)).size(13.0));
    }
    ui.add_space(4.0);
    ui.label(widgets::dim(NOTE).size(12.0));
}

/// L'estimation gardée, seulement si elle vient de la seed `seed_name` et a réussi.
fn known_estimate<'a>(
    remembered: Option<&'a Remembered>,
    seed_name: &str,
) -> Option<&'a SeedEstimate> {
    let remembered = remembered?;
    if remembered.seed_name != seed_name {
        return None;
    }
    remembered.estimate.as_ref().ok()
}

/// `Alice : ≈ 2 h 40 de jeu, au plus tard 3 h 10` (colonne étroite de « Partie en cours »).
fn short_player_line(p: &PlayerEstimate) -> String {
    let mut text = format!("{} : ≈ {} de jeu", p.name, duration(p.play_seconds));
    if p.wait_seconds() > 0 {
        text.push_str(&format!(", au plus tard {}", duration(p.finish_seconds)));
    }
    text
}

/// Rappel affiché sous les chiffres.
const NOTE: &str = "Ordre de grandeur : temps moyen par lieu du profil du jeu (2 min sinon). \
     « Au plus tard » suppose que chacun attend la fin de chaque sphère.";

/// `≈ 3 h 20 de jeu pour le plus long, fin au plus tard vers 4 h 10 (5 sphères)`.
fn summary(estimate: &SeedEstimate) -> String {
    let mut text = format!(
        "≈ {} de jeu pour le plus long, fin au plus tard vers {}",
        duration(estimate.longest_play_seconds()),
        duration(estimate.finish_seconds())
    );
    if estimate.spheres > 0 {
        text.push_str(&format!(" ({} sphères)", estimate.spheres));
    }
    text
}

/// `Alice · Slime Rancher : 40 lieux, ≈ 2 h 40 de jeu, fin au plus tard 3 h 10`.
fn player_line(p: &PlayerEstimate) -> String {
    let mut text = format!(
        "{} · {} : {}, ≈ {} de jeu",
        p.name,
        p.game,
        format::locations_count(p.locations),
        duration(p.play_seconds)
    );
    if p.wait_seconds() > 0 {
        text.push_str(&format!(
            ", fin au plus tard {}",
            duration(p.finish_seconds)
        ));
    }
    if !p.from_profile {
        text.push_str(" (temps par défaut)");
    }
    text
}

fn duration(seconds: u64) -> String {
    // Les secondes tiennent largement dans un `f64` (exact jusqu'à 2^53).
    format::short_duration(seconds as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(play: u64, finish: u64, from_profile: bool) -> PlayerEstimate {
        PlayerEstimate {
            slot: 1,
            name: "Alice".into(),
            game: "Slime Rancher".into(),
            locations: 40,
            seconds_per_location: 240,
            from_profile,
            play_seconds: play,
            finish_seconds: finish,
        }
    }

    #[test]
    fn ligne_d_un_joueur() {
        let line = player_line(&player(9600, 11_400, true));
        assert!(line.starts_with("Alice · Slime Rancher : "), "{line}");
        assert!(line.contains("≈ 2 h 40 de jeu"), "{line}");
        assert!(line.contains("fin au plus tard 3 h 10"), "{line}");
        assert!(!line.contains("défaut"), "{line}");
        // Sans attente, pas de « fin au plus tard » ; temps par défaut signalé.
        let line = player_line(&player(600, 600, false));
        assert!(!line.contains("au plus tard"), "{line}");
        assert!(line.ends_with("(temps par défaut)"), "{line}");
    }

    fn remembered(seed_name: &str, estimate: Result<SeedEstimate, String>) -> Remembered {
        Remembered {
            file: PathBuf::from("seed.archipelago"),
            seed_name: seed_name.into(),
            estimate,
        }
    }

    #[test]
    fn partie_en_cours_montre_l_estimation_de_sa_seed_seulement() {
        let estimate = SeedEstimate {
            players: vec![player(9600, 11_400, true)],
            spheres: 3,
        };
        let kept = remembered("AP_123", Ok(estimate));
        assert!(known_estimate(Some(&kept), "AP_123").is_some());
        // Autre seed hébergée, rien de calculé, ou estimation ratée : rien.
        assert!(known_estimate(Some(&kept), "AP_999").is_none());
        assert!(known_estimate(None, "AP_123").is_none());
        let failed = remembered("AP_123", Err("illisible".into()));
        assert!(known_estimate(Some(&failed), "AP_123").is_none());
    }

    #[test]
    fn ligne_courte_d_un_joueur() {
        assert_eq!(
            short_player_line(&player(9600, 11_400, true)),
            "Alice : ≈ 2 h 40 de jeu, au plus tard 3 h 10"
        );
        assert_eq!(
            short_player_line(&player(600, 600, false)),
            "Alice : ≈ 10 min de jeu"
        );
    }

    #[test]
    fn resume_de_la_seed() {
        let estimate = SeedEstimate {
            players: vec![player(9600, 11_400, true), player(600, 700, false)],
            spheres: 5,
        };
        assert_eq!(
            summary(&estimate),
            "≈ 2 h 40 de jeu pour le plus long, fin au plus tard vers 3 h 10 (5 sphères)"
        );
    }
}
