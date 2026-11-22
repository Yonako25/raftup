//! Écran « Catalogue » (plan de l'interface, étape 4) : les jeux pris en charge par
//! Raftup, ceux du catalogue embarqué et ceux du registre signé, avec un filtre.
//!
//! Chaque carte dit d'où vient le profil (et son niveau de confiance pour un profil du
//! registre, ADR 0045) et son état sur ce PC.
//!
//! C'est un **référentiel en lecture seule** (choix de l'utilisatrice, 22 novembre
//! 2026) : on y voit les jeux compatibles, on n'y ajoute rien. La Bibliothèque montre
//! d'elle-même les jeux trouvés sur ce PC.

use egui::Ui;
use launcher_core::art::ArtKind;
use launcher_core::profile::{GameStatus, StepKind};

use super::library::{cover_card, CardLook, GameSummary, LibraryState, Tone};
use super::registry_panel;
use crate::theme;
use crate::widgets;

/// Ce que le joueur a tapé dans le filtre, gardé d'une image à l'autre.
#[derive(Default)]
pub struct CatalogState {
    pub filter: String,
    /// Case « Seulement les jeux détectés sur ce PC ».
    pub detected_only: bool,
    /// Jeu dont la fiche d'information est ouverte (clic sur sa carte).
    pub selected: Option<String>,
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

pub fn show(ui: &mut Ui, state: &mut CatalogState, library: &mut LibraryState) {
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

        // Fiche ouverte : grille à gauche, fiche à droite (comme la Bibliothèque) ;
        // fenêtre étroite : la fiche passe sous la grille.
        let open = state
            .selected
            .clone()
            .filter(|id| games.iter().any(|g| &g.id == id));
        match open {
            Some(id) if ui.available_width() >= super::WIDE => {
                ui.horizontal_top(|ui| {
                    let grid_width = ui.available_width() - 400.0;
                    ui.vertical(|ui| {
                        ui.set_width(grid_width);
                        grid(ui, state, library, &shown, &ctx);
                    });
                    ui.vertical(|ui| info_panel(ui, state, library, &id, &ctx));
                });
            }
            Some(id) => {
                grid(ui, state, library, &shown, &ctx);
                ui.add_space(12.0);
                info_panel(ui, state, library, &id, &ctx);
            }
            None => grid(ui, state, library, &shown, &ctx),
        }

        ui.add_space(14.0);
        ui.label(
            widgets::dim(
                "Raftup ne fournit aucun jeu : il installe le mod sur ta copie, après vérification de son empreinte.",
            )
            .size(12.0),
        );
    });
}

/// Grille de jaquettes, comme la maquette « Catalogue » : l'illustration du jeu (ADR
/// 0051) si elle est lue, sinon l'aplat coloré avec le titre. Un clic sur une
/// carte ouvre sa fiche d'information (rien ne s'ajoute d'ici).
fn grid(
    ui: &mut Ui,
    state: &mut CatalogState,
    library: &mut LibraryState,
    shown: &[&GameSummary],
    ctx: &egui::Context,
) {
    // Rangées de `per_row` cartes, comme la grille de la Bibliothèque :
    // `horizontal_wrapped` ne sait pas passer à la ligne des blocs verticaux, et les jeux
    // en trop sortaient de l'écran (bug du 22 novembre 2026).
    let per_row = super::library::cards_per_row(ui.available_width());
    // `scope` : cet espacement ne vaut que pour la grille.
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing =
            egui::vec2(super::library::CARD_GAP, super::library::CARD_GAP);
        // `chunks` découpe la liste en rangées (la dernière peut être plus courte).
        for row in shown.chunks(per_row) {
            ui.horizontal_top(|ui| {
                for game in row {
                    let cover = library.art_for(ctx, &game.id, ArtKind::Cover);
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
                        let selected = state.selected.as_deref() == Some(game.id.as_str());
                        if cover_card(ui, &look, selected) {
                            // Deuxième clic sur la même carte : la fiche se ferme.
                            state.selected = if selected {
                                None
                            } else {
                                Some(game.id.clone())
                            };
                        }
                        if let Some(trust) = &game.trust {
                            registry_panel::trust_badge(ui, trust);
                        }
                    });
                }
            });
        }
    });
}

/// Fiche d'information d'un jeu, en lecture seule : image, état sur ce PC, nom chez
/// Archipelago, auteurs, licence et mods que Raftup installerait.
fn info_panel(
    ui: &mut Ui,
    state: &mut CatalogState,
    library: &mut LibraryState,
    id: &str,
    ctx: &egui::Context,
) {
    let hero = library.art_for(ctx, id, ArtKind::Hero);
    let summary = library.summaries().into_iter().find(|g| g.id == id);
    // `cloned` : une copie du profil, pour ne plus emprunter `library` pendant le dessin.
    let Some(profile) = library.catalog().and_then(|c| c.get(id)).cloned() else {
        return;
    };
    widgets::panel(ui, "", |ui| {
        super::library::hero_banner(ui, hero.as_ref(), id, &profile.game.name);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(widgets::title(&profile.game.name, 24.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::ghost_button(ui, "Fermer") {
                    state.selected = None;
                }
            });
        });
        if let Some(summary) = &summary {
            ui.label(egui::RichText::new(&summary.label).color(tone_color(summary.tone)));
        }
        if profile.game.status == GameStatus::Untested {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Profil non testé : personne ne l'a encore essayé avec le vrai jeu.",
                )
                .color(theme::AMBER)
                .size(13.0),
            );
        }
        ui.add_space(10.0);
        info_line(ui, "Nom chez Archipelago", &profile.game.ap_game);
        info_line(ui, "Auteurs des mods", &profile.credits.authors.join(", "));
        info_line(ui, "Licence", &profile.credits.license);
        ui.add_space(10.0);
        ui.label(widgets::caps("Ce que Raftup installe"));
        ui.add_space(4.0);
        for step in &profile.steps {
            let kind = match step.kind {
                StepKind::ModLoader => "Mod loader",
                StepKind::Mod => "Mod client",
                StepKind::Apworld => "World",
            };
            ui.label(
                egui::RichText::new(format!("{} · {kind} · {}", step.name, step.version))
                    .color(theme::TEXT)
                    .size(13.0),
            );
        }
        ui.add_space(10.0);
        ui.label(
            widgets::dim(
                "Quand le jeu est trouvé sur ce PC, il apparaît dans la Bibliothèque : l'installation se fait là-bas.",
            )
            .size(12.0),
        );
    });
}

/// Une ligne « titre : valeur » de la fiche d'information.
fn info_line(ui: &mut Ui, title: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(widgets::dim(format!("{title} :")).size(13.0));
        ui.label(egui::RichText::new(value).color(theme::TEXT).size(13.0));
    });
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
