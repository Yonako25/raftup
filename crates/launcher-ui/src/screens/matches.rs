//! Écran « En cours » : la liste des parties lancées.
//!
//! Pour l'instant, la seule partie connue est celle que ce PC héberge. Les parties
//! rejointes chez un ami arriveront avec « Rejoindre par lien ».

use egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};

use crate::app::Page;
use crate::format;
use crate::theme;
use crate::widgets;
use crate::worker::Shared;

pub fn show(ui: &mut Ui, shared: &Shared) -> Option<Page> {
    let mut goto = None;
    super::page(ui, |ui| {
        // Titre et sous-titre sur la même ligne, comme sur la maquette.
        ui.horizontal(|ui| {
            ui.label(widgets::title("En cours", 28.0));
            ui.label(widgets::dim("Les parties lancées").size(14.0));
        });
        ui.add_space(14.0);

        match &shared.status {
            Some(_) => {
                list_frame(ui, |ui| {
                    if hosted_row(ui, shared) {
                        goto = Some(Page::Hosted);
                    }
                });
            }
            None => {
                if empty_state(ui, shared.starting) {
                    goto = Some(Page::Hosted);
                }
            }
        }

        ui.add_space(10.0);
        if ui
            .link("Les salons en préparation sont dans « Salons »")
            .clicked()
        {
            goto = Some(Page::Soon("Salons"));
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
    open
}

/// Aucune partie : un panneau qui propose d'en héberger une. Renvoie vrai au clic.
fn empty_state(ui: &mut Ui, starting: bool) -> bool {
    let mut clicked = false;
    widgets::panel(ui, "Aucune partie en cours", |ui| {
        ui.label(widgets::dim(
            "Héberge une seed générée par Archipelago : tes amis la rejoignent avec l'adresse que Raftup te donne.",
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
