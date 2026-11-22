//! Écran « Catalogue » (plan de l'interface, étape 4) : les jeux pris en charge par
//! Raftup, ceux du catalogue embarqué et ceux du registre signé, avec un filtre.
//!
//! Chaque ligne dit d'où vient le profil (et son niveau de confiance pour un profil du
//! registre, ADR 0045), son état sur ce PC, et ouvre sa fiche dans la Bibliothèque,
//! où se font l'installation et le lancement.

use egui::Ui;
use launcher_core::art::ArtKind;

use super::library::{cover_card, CardLook, GameSummary, LibraryState, Tone};
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
    let ctx = ui.ctx().clone();
    super::page(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(widgets::title("Catalogue", 28.0));
            ui.label(widgets::dim("Les jeux compatibles avec Raftup").size(14.0));
        });
        ui.add_space(12.0);
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
        ui.add_space(10.0);
        if shown.is_empty() {
            widgets::panel(ui, "", |ui| {
                ui.label(widgets::dim("Aucun jeu ne correspond à ce filtre."));
            });
        }

        // Grille de jaquettes, comme la maquette « Catalogue » : l'illustration du hub
        // (IGDB, ADR 0047) si elle est arrivée, sinon l'aplat coloré avec le titre.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(22.0, 22.0);
            for game in shown {
                let cover = library.art_for(&ctx, &game.id, ArtKind::Cover);
                ui.vertical(|ui| {
                    let marker = game.from_registry.then_some(("Registre", theme::ITEM));
                    let look = CardLook {
                        cover_id: &game.id,
                        name: &game.name,
                        status: game.label.clone(),
                        status_color: tone_color(game.tone),
                        fraction: if game.ready { 1.0 } else { 0.0 },
                        marker,
                        cover,
                    };
                    let mut open = cover_card(ui, &look, false);
                    if let Some(trust) = &game.trust {
                        registry_panel::trust_badge(ui, trust);
                    }
                    // Même largeur que la jaquette.
                    // « Ajouter » en bleu (action principale), « Dans la bibliothèque » discret.
                    let button = if game.ready {
                        egui::Button::new("Dans la bibliothèque")
                    } else {
                        egui::Button::new(
                            egui::RichText::new("Ajouter")
                                .color(egui::Color32::from_rgb(0x08, 0x12, 0x22))
                                .strong()
                                .family(theme::bold_family()),
                        )
                        .fill(theme::ACCENT)
                    }
                    .min_size(egui::vec2(150.0, 30.0));
                    open |= ui.add(button).clicked();
                    if open {
                        library.select(&game.id);
                        goto = Some(Page::Library);
                    }
                });
            }
        });

        // Les apworlds du registre signé et la conversion en monde natif (ADR 0045,
        // 0046), déplacés ici depuis la Bibliothèque.
        super::library::registry_and_conversion(ui, library);
        ui.add_space(14.0);
        if let Some((text, url)) = library.art_attribution() {
            super::library::attribution_line(ui, &text, &url);
        }
        ui.label(
            widgets::dim(
                "Raftup ne fournit aucun jeu : il installe le mod sur ta copie, après vérification de son empreinte.",
            )
            .size(12.0),
        );
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
