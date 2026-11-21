//! Écran « Catalogue » (plan de l'interface, étape 4) : les jeux pris en charge par
//! Raftup, ceux du catalogue embarqué et ceux du registre signé, avec un filtre.
//!
//! Chaque ligne dit d'où vient le profil (et son niveau de confiance pour un profil du
//! registre, ADR 0045), son état sur ce PC, et ouvre sa fiche dans la Bibliothèque,
//! où se font l'installation et le lancement.

use egui::{RichText, Ui};

use super::library::{GameSummary, LibraryState, Tone};
use super::registry_panel;
use crate::app::Page;
use crate::theme;
use crate::widgets;

/// Ce que le joueur a tapé dans le filtre, gardé d'une image à l'autre.
#[derive(Default)]
pub struct CatalogState {
    pub filter: String,
    /// Case « Seulement les jeux détectés sur ce PC ».
    pub detected_only: bool,
}

/// Vrai si le jeu passe le filtre : texte cherché dans le nom du jeu, son nom chez
/// Archipelago et ses auteurs, sans tenir compte des majuscules.
pub fn matches_filter(game: &GameSummary, filter: &str, detected_only: bool) -> bool {
    if detected_only && !game.detected {
        return false;
    }
    let wanted = filter.trim().to_lowercase();
    if wanted.is_empty() {
        return true;
    }
    let mut haystack = vec![game.name.to_lowercase(), game.ap_game.to_lowercase()];
    haystack.extend(game.authors.iter().map(|a| a.to_lowercase()));
    haystack.iter().any(|text| text.contains(&wanted))
}

fn tone_color(tone: Tone) -> egui::Color32 {
    match tone {
        Tone::Ready | Tone::Dim => theme::TEXT_DIM,
        Tone::Accent => theme::ACCENT,
        Tone::Warning => theme::AMBER,
    }
}

pub fn show(ui: &mut Ui, state: &mut CatalogState, library: &mut LibraryState) -> Option<Page> {
    let mut goto = None;
    let games = library.summaries();
    super::page(ui, |ui| {
        super::header(ui, "Catalogue", "Parcourir", |_| {});
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.filter)
                    .hint_text("Chercher un jeu ou un auteur")
                    .desired_width(320.0),
            );
            ui.checkbox(
                &mut state.detected_only,
                "Seulement les jeux détectés sur ce PC",
            );
        });
        ui.add_space(10.0);

        let shown: Vec<&GameSummary> = games
            .iter()
            .filter(|g| matches_filter(g, &state.filter, state.detected_only))
            .collect();
        let count = shown.len();
        let plural = if count > 1 { "x" } else { "" };
        ui.label(widgets::dim(format!("{count} jeu{plural}")).size(13.0));
        ui.add_space(6.0);
        if shown.is_empty() {
            widgets::panel(ui, "", |ui| {
                ui.label(widgets::dim("Aucun jeu ne correspond à ce filtre."));
            });
        }
        for game in shown {
            widgets::panel(ui, "", |ui| {
                ui.horizontal(|ui| {
                    super::library::small_cover(ui, 48.0, &game.id, &game.name);
                    ui.vertical(|ui| {
                        ui.label(widgets::title(&game.name, 18.0));
                        let origin = if game.from_registry {
                            "Registre signé"
                        } else {
                            "Catalogue de Raftup"
                        };
                        ui.horizontal(|ui| {
                            ui.label(widgets::dim(origin).size(12.0));
                            if let Some(trust) = &game.trust {
                                registry_panel::trust_badge(ui, trust);
                            }
                        });
                        if !game.authors.is_empty() {
                            ui.label(widgets::dim(game.authors.join(", ")).size(12.0));
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button = if game.ready { "Ouvrir" } else { "Ajouter" };
                        if widgets::ghost_button(ui, &format!("{button} {}", game.name)) {
                            library.select(&game.id);
                            goto = Some(Page::Library);
                        }
                        ui.label(RichText::new(&game.label).color(tone_color(game.tone)));
                    });
                });
            });
            ui.add_space(8.0);
        }
    });
    goto
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slime() -> GameSummary {
        GameSummary {
            id: "slime-rancher".to_string(),
            name: "Slime Rancher".to_string(),
            ap_game: "Slime Rancher".to_string(),
            authors: vec!["SWCreeperKing (Slimipelago)".to_string()],
            label: "Non détecté".to_string(),
            tone: Tone::Dim,
            ready: false,
            detected: false,
            from_registry: false,
            trust: None,
        }
    }

    #[test]
    fn filtre_par_nom_ou_auteur() {
        let game = slime();
        assert!(matches_filter(&game, "", false));
        assert!(matches_filter(&game, "  slime ", false));
        assert!(matches_filter(&game, "creeper", false));
        assert!(!matches_filter(&game, "hollow", false));
        // Pas détecté : caché par la case « seulement les jeux détectés ».
        assert!(!matches_filter(&game, "", true));
        let detected = GameSummary {
            detected: true,
            ..slime()
        };
        assert!(matches_filter(&detected, "slime", true));
    }
}
