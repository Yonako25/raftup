//! Écran 2 : « Partie en cours ».

use egui::{RichText, ScrollArea, Ui};
use launcher_core::HostStatus;

use crate::app::Page;
use crate::format::{self, Piece, StatusKind};
use crate::theme;
use crate::widgets;
use crate::worker::{Command, Shared, Worker};

/// Ce que l'hôte a saisi ou coché sur cet écran.
#[derive(Default)]
pub struct RunningState {
    /// Message à tous les joueurs, en cours de frappe.
    announce: String,
    /// Case « Détails » du journal.
    details: bool,
    /// La fenêtre « Arrêter le serveur ? » est ouverte.
    confirm_stop: bool,
}

pub fn show(
    ui: &mut Ui,
    state: &mut RunningState,
    error: &mut Option<String>,
    worker: &Worker,
    shared: &Shared,
) -> Option<Page> {
    // `show` n'est appelé que si la partie tourne, mais on reste prudent : pas de `unwrap`.
    let status = shared.status.as_ref()?;
    let now = format::now();
    let mut copy: Option<String> = None;
    let mut ask_stop = false;

    super::page(ui, |ui| {
        let breadcrumb = format!("Parties › seed {}", status.seed.seed_name);
        super::header(ui, &breadcrumb, "Partie en cours", |ui| {
            if widgets::danger_button(ui, "■ Arrêter le serveur") {
                ask_stop = true;
            }
            if widgets::ghost_button(ui, "Copier l'invitation") {
                copy = Some(format::invitation(
                    &status.address,
                    status.has_password,
                    &status.server.slots,
                ));
            }
        });
        if let Some(message) = error.as_deref() {
            widgets::error_banner(ui, message);
            ui.add_space(10.0);
        }

        let mode = if status.encrypted {
            "chiffré (wss)"
        } else {
            "en clair (ws)"
        };
        ui.horizontal(|ui| {
            widgets::pill(
                ui,
                &format!("● En ligne · {mode}"),
                theme::GREEN,
                theme::GREEN_BG,
            );
            ui.label(widgets::dim(format!(
                "Démarré il y a {}",
                format::short_duration(now - status.started_at)
            )));
        });
        ui.add_space(10.0);

        address_panel(ui, status, &mut copy);
        ui.add_space(12.0);
        counters(ui, status);
        ui.add_space(12.0);
        players_panel(ui, status, now);
        ui.add_space(12.0);

        if ui.available_width() >= super::WIDE {
            ui.columns(2, |columns| {
                activity_panel(&mut columns[0], state, worker, shared);
                journal_panel(&mut columns[1], state, shared);
            });
        } else {
            activity_panel(ui, state, worker, shared);
            ui.add_space(12.0);
            journal_panel(ui, state, shared);
        }
    });

    if let Some(text) = copy {
        ui.ctx().copy_text(text);
    }
    if ask_stop {
        state.confirm_stop = true;
    }
    if state.confirm_stop {
        confirm_window(ui.ctx(), state, worker);
    }
    None
}

/// Fenêtre de confirmation avant d'arrêter le serveur.
fn confirm_window(ctx: &egui::Context, state: &mut RunningState, worker: &Worker) {
    egui::Window::new("Arrêter le serveur ?")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Tous les joueurs seront déconnectés.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if widgets::danger_button(ui, "■ Arrêter") {
                    worker.send(Command::Stop);
                    state.confirm_stop = false;
                }
                if widgets::ghost_button(ui, "Annuler") {
                    state.confirm_stop = false;
                }
            });
        });
}

fn address_panel(ui: &mut Ui, status: &HostStatus, copy: &mut Option<String>) {
    widgets::panel(ui, "Adresse à donner aux joueurs", |ui| {
        ui.horizontal(|ui| {
            ui.label(widgets::mono(&status.address, 16.0).color(theme::ITEM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::ghost_button(ui, "Copier l'adresse") {
                    *copy = Some(status.address.clone());
                }
                let password = if status.has_password { "Oui" } else { "Aucun" };
                ui.label(widgets::dim(format!("Mot de passe : {password}")));
            });
        });
    });
}

/// Les trois compteurs : joueurs connectés, lieux visités, objectifs (groupes exclus).
fn counters(ui: &mut Ui, status: &HostStatus) {
    let t = format::totals(&status.server.slots);
    let cards = [
        (
            "Joueurs connectés",
            format!("{} / {}", t.connected, t.players),
        ),
        ("Lieux visités", format!("{} / {}", t.checked, t.locations)),
        ("Objectifs atteints", format!("{} / {}", t.goals, t.players)),
    ];
    ui.columns(3, |columns| {
        for (column, (label, value)) in columns.iter_mut().zip(cards) {
            widgets::panel(column, "", |ui| {
                ui.label(widgets::dim(label).size(13.0));
                ui.label(widgets::title(&value, 28.0));
            });
        }
    });
}

fn players_panel(ui: &mut Ui, status: &HostStatus, now: f64) {
    widgets::panel(ui, "Joueurs", |ui| {
        egui::Grid::new("players")
            .num_columns(4)
            .spacing([24.0, 10.0])
            .min_col_width(110.0)
            .show(ui, |ui| {
                for head in ["Joueur", "Statut", "Lieux visités", "Dernière activité"] {
                    ui.label(widgets::dim(head).size(12.0));
                }
                ui.end_row();

                for slot in &status.server.slots {
                    // Colonne 1 : pastille, nom, jeu et tags.
                    ui.horizontal(|ui| {
                        widgets::badge(ui, 32.0, &format::initials(&slot.game), slot.is_group);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(RichText::new(&slot.name).strong());
                            ui.label(widgets::dim(format::game_with_tags(slot)).size(12.0));
                        });
                    });
                    // Colonne 2 : statut.
                    let (text, kind) = format::slot_status(slot);
                    let color = match kind {
                        StatusKind::Goal => theme::ACCENT,
                        StatusKind::Offline => theme::TEXT_DIM,
                        StatusKind::Online => theme::GREEN,
                    };
                    ui.label(RichText::new(text).color(color));
                    // Colonne 3 : barre de progression.
                    ui.horizontal(|ui| {
                        let fraction = if slot.total == 0 {
                            0.0
                        } else {
                            slot.checked as f32 / slot.total as f32
                        };
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .desired_width(110.0)
                                .desired_height(8.0)
                                .fill(theme::ACCENT)
                                .corner_radius(egui::CornerRadius::same(theme::RADIUS)),
                        );
                        ui.label(
                            widgets::dim(format!("{} / {}", slot.checked, slot.total)).size(13.0),
                        );
                    });
                    // Colonne 4 : dernière activité.
                    ui.label(widgets::dim(format::ago(now, slot.last_activity)));
                    ui.end_row();
                }
            });
    });
}

/// Un morceau de phrase : nom en gras, objet en bleu clair, le reste normal.
fn piece_label(ui: &mut Ui, piece: &Piece) {
    match piece {
        Piece::Name(s) => ui.label(RichText::new(s).strong()),
        Piece::Item(s) => ui.label(RichText::new(s).color(theme::ITEM)),
        Piece::Text(s) => ui.label(s),
    };
}

fn activity_panel(ui: &mut Ui, state: &mut RunningState, worker: &Worker, shared: &Shared) {
    widgets::panel(ui, "Activité", |ui| {
        ScrollArea::vertical()
            .id_salt("activity")
            .max_height(260.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if shared.events.is_empty() {
                    ui.label(widgets::dim("Rien pour l'instant."));
                }
                // Le plus récent en haut : on parcourt la liste à l'envers.
                for numbered in shared.events.iter().rev().take(300) {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let time = format::local_time(format::event_time(&numbered.event), false);
                        ui.label(widgets::mono(format!("{time}  "), 12.0).color(theme::TEXT_DIM));
                        for piece in format::activity(&numbered.event) {
                            piece_label(ui, &piece);
                        }
                    });
                }
            });

        ui.add_space(8.0);
        ui.label(widgets::dim("Message à tous les joueurs").size(13.0));
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut state.announce)
                    .desired_width((ui.available_width() - 100.0).max(80.0)),
            );
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (widgets::primary_button(ui, "Envoyer", true) || enter)
                && !state.announce.trim().is_empty()
            {
                worker.send(Command::Announce(std::mem::take(&mut state.announce)));
            }
        });
    });
}

fn journal_panel(ui: &mut Ui, state: &mut RunningState, shared: &Shared) {
    widgets::panel(ui, "", |ui| {
        ui.horizontal(|ui| {
            ui.label(widgets::title("Journal du serveur", 17.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut state.details, "Détails");
            });
        });
        egui::Frame::new()
            .fill(theme::LOG_BG)
            .corner_radius(egui::CornerRadius::same(theme::RADIUS))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ScrollArea::vertical()
                    .id_salt("journal")
                    .max_height(260.0)
                    .auto_shrink([false, true])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &shared.logs {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let color = match line.level.as_str() {
                                    "ERROR" => theme::RED_TEXT,
                                    "WARN" => theme::AMBER,
                                    _ => theme::TEXT,
                                };
                                let time = format::local_time(line.time, true);
                                ui.label(widgets::mono(time, 12.0).color(theme::TEXT_DIM));
                                ui.label(
                                    widgets::mono(format!("{:<5}", line.level), 12.0).color(color),
                                );
                                ui.label(widgets::mono(&line.text, 12.0).color(color));
                                if state.details && !line.fields.is_empty() {
                                    ui.label(
                                        widgets::mono(&line.fields, 12.0).color(theme::TEXT_DIM),
                                    );
                                }
                            });
                        }
                    });
            });
    });
}
