//! Écran « En cours » : la liste des parties lancées.
//!
//! Pour l'instant, la seule partie connue est celle que ce PC héberge, avec la
//! progression de chaque joueur (plan de l'interface, étape 3). Une partie rejointe
//! chez un ami n'est pas encore suivie : il faudrait un client de suivi connecté à
//! son serveur (question laissée ouverte, voir la PR de l'étape 3).
//!
//! En dessous, les parties hébergées sur Rafthub par le compte connecté (ADR 0052) :
//! « Ouvrir » mène à leur écran (`hub_game`).

use egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};

use super::hub_game::{self, HubGames};
use crate::app::Page;
use crate::format;
use crate::theme;
use crate::widgets;
use crate::worker::Shared;

/// `error` : erreur du worker à montrer (« La partie s'est arrêtée d'elle-même »…).
pub fn show(ui: &mut Ui, shared: &Shared, error: Option<&str>, hub: &mut HubGames) -> Option<Page> {
    let mut goto = None;
    super::page(ui, |ui| {
        // Titre et sous-titre sur la même ligne, comme sur la maquette.
        ui.horizontal(|ui| {
            ui.label(widgets::title("En cours", 28.0));
            ui.label(widgets::dim("Les parties lancées").size(14.0));
        });
        ui.add_space(14.0);
        if let Some(message) = error {
            widgets::error_banner(ui, message);
            ui.add_space(10.0);
        }

        if shared.status.is_some() {
            list_frame(ui, |ui| {
                if hosted_row(ui, shared) {
                    goto = Some(Page::Hosted);
                }
            });
        } else if hub.count() == 0 && empty_state(ui, shared.starting) {
            // Rien sur ce PC ni sur Rafthub : le panneau propose d'héberger.
            goto = Some(Page::Hosted);
        }
        // Parties hébergées sur Rafthub : `open` garde l'identifiant cliqué, la
        // sélection se fait après la boucle (on ne modifie pas `hub` en le parcourant).
        let mut open = None;
        if hub.count() > 0 {
            ui.add_space(12.0);
            list_frame(ui, |ui| {
                for (index, game) in hub.games().iter().enumerate() {
                    if index > 0 {
                        ui.separator();
                    }
                    // `push_id` : deux lignes ont les mêmes boutons « Ouvrir ».
                    ui.push_id(&game.id, |ui| {
                        if hub_game::row(ui, game) {
                            open = Some(game.id.clone());
                        }
                    });
                }
            });
        }
        if let Some(id) = open {
            hub_game::select(hub, &id);
            goto = Some(Page::HubGame);
        }
    });
    goto
}

/// Cadre de la liste : les lignes sont collées, séparées par un trait fin.
fn list_frame(ui: &mut Ui, add_rows: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(theme::panel_fill())
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            // La liste ne s'étire pas au-delà de 1 000 px sur un grand écran.
            ui.set_width(ui.available_width().min(1000.0));
            add_rows(ui);
        });
}

/// La ligne de la partie hébergée. Renvoie vrai si on demande à l'ouvrir.
fn hosted_row(ui: &mut Ui, shared: &Shared) -> bool {
    let Some(status) = &shared.status else {
        return false;
    };
    let players: Vec<_> = status
        .server
        .slots
        .iter()
        .filter(|slot| !slot.is_group)
        .collect();
    let connected = players.iter().filter(|slot| slot.connections > 0).count();
    // La « jaquette » : un aplat avec les initiales du premier jeu (SPEC § 7 :
    // jamais de visuel officiel sans autorisation).
    let first_game = players.first().map(|slot| slot.game.as_str()).unwrap_or("");

    let mut open = false;
    ui.horizontal(|ui| {
        widgets::badge(ui, 52.0, &format::initials(first_game), false);
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(
                RichText::new(format!("seed {}", status.seed.seed_name))
                    .size(17.0)
                    .strong()
                    .family(theme::bold_family())
                    .color(theme::TEXT),
            );
            let line = match connected {
                0 => "En ligne, personne n'est connecté".to_string(),
                1 => "En ligne · 1 joueur connecté".to_string(),
                n => format!("En ligne · {n} joueurs connectés"),
            };
            ui.label(RichText::new(line).color(theme::GREEN));
            let count = players.len();
            let plural = if count > 1 { "s" } else { "" };
            ui.label(
                widgets::dim(format!(
                    "{count} joueur{plural} · sur ce PC · démarrée il y a {}",
                    format::short_duration(format::now() - status.started_at)
                ))
                .size(13.0),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            open = widgets::primary_button(ui, "   Ouvrir   ", true);
        });
    });
    // La progression de chacun, sous la ligne de la partie.
    ui.add_space(8.0);
    for slot in &players {
        ui.horizontal(|ui| {
            ui.add_space(58.0);
            ui.label(RichText::new(&slot.name).color(theme::TEXT));
            ui.label(widgets::dim(&slot.game).size(12.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(widgets::dim(progress(slot.checked, slot.total)).size(12.0));
                let color = if slot.status >= 30 {
                    theme::GREEN
                } else {
                    theme::ACCENT
                };
                widgets::bar(ui, 160.0, fraction(slot.checked, slot.total), color);
            });
        });
    }
    open
}

/// Part des lieux visités, de 0 à 1 (0 pour un slot sans lieu).
pub fn fraction(checked: usize, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        (checked as f32 / total as f32).clamp(0.0, 1.0)
    }
}

/// « 12 / 40 lieux ».
pub fn progress(checked: usize, total: usize) -> String {
    format!("{checked} / {total} lieux")
}

/// Aucune partie : un panneau qui propose d'en héberger une. Renvoie vrai au clic.
fn empty_state(ui: &mut Ui, starting: bool) -> bool {
    let mut clicked = false;
    widgets::panel(ui, "Aucune partie en cours", |ui| {
        ui.label(widgets::dim(
            "Hébergez une seed générée par Archipelago : vos amis la rejoignent avec l'adresse que Raftup vous donne.",
        ));
        ui.add_space(8.0);
        let text = if starting {
            "Démarrage…"
        } else {
            "Héberger une partie"
        };
        clicked = widgets::primary_button(ui, text, true);
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_des_lieux_visites() {
        assert_eq!(fraction(0, 0), 0.0);
        assert_eq!(fraction(10, 40), 0.25);
        // Jamais plus que plein, même si le serveur compte plus de lieux visités.
        assert_eq!(fraction(50, 40), 1.0);
        assert_eq!(progress(12, 40), "12 / 40 lieux");
    }
}
