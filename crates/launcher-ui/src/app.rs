//! État de l'application, navigation et barre latérale.

use egui::{Align2, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui};

use crate::screens::{running, setup, soon};
use crate::theme;
use crate::widgets;
use crate::worker::{Command, Worker};

/// L'écran affiché.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Page {
    Games,
    /// « Parties » : l'écran d'hébergement, ou la partie en cours si le serveur tourne.
    Matches,
    Catalog,
    /// Écran « Bientôt » ouvert par un bouton (nom de la fonction à venir).
    Soon(&'static str),
}

pub struct App {
    page: Page,
    worker: Worker,
    setup: setup::SetupForm,
    running: running::RunningState,
    /// Erreur à montrer en rouge en haut de l'écran.
    error: Option<String>,
}

impl App {
    pub fn new(worker: Worker) -> App {
        App {
            page: Page::Matches,
            worker,
            setup: setup::SetupForm::default(),
            running: running::RunningState::default(),
            error: None,
        }
    }
}

impl eframe::App for App {
    /// Fermeture de la fenêtre : le worker arrête la partie proprement.
    fn on_exit(&mut self) {
        self.worker.send(Command::Shutdown);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::BG.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // La durée « il y a … » doit avancer même si rien d'autre ne bouge.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        theme::paint_background(&ctx);

        // Une erreur d'une commande (port pris, certificat refusé…) arrive du worker.
        if let Some(message) = self.worker.take_error() {
            self.error = Some(message);
        }

        // Le verrou est tenu le temps du dessin : le worker attend au plus une image.
        let shared = self.worker.lock();
        let hosting = shared.status.is_some();

        let mut goto: Option<Page> = None;
        egui::Panel::left("sidebar")
            .exact_size(220.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(theme::sidebar_fill())
                    .inner_margin(Margin::symmetric(0, 18)),
            )
            .show(ui, |ui| {
                goto = sidebar(ui, &self.page, &shared);
            });

        egui::CentralPanel::no_frame().show(ui, |ui| {
            let next = match &self.page {
                Page::Matches if hosting => running::show(
                    ui,
                    &mut self.running,
                    &mut self.error,
                    &self.worker,
                    &shared,
                ),
                Page::Matches => {
                    setup::show(ui, &mut self.setup, &mut self.error, &self.worker, &shared)
                }
                Page::Games => soon::show(ui, "Jeux"),
                Page::Catalog => soon::show(ui, "Catalogue"),
                Page::Soon(name) => soon::show(ui, name),
            };
            if next.is_some() {
                goto = next;
            }
        });

        drop(shared);
        // Verrou relâché : on peut ouvrir une boîte de dialogue (elle bloque).
        setup::open_pending_dialog(&mut self.setup, &self.worker);
        if let Some(page) = goto {
            self.page = page;
        }
    }
}

/// Barre latérale : logo, liens, état du serveur. Renvoie la page demandée.
fn sidebar(ui: &mut Ui, current: &Page, shared: &crate::worker::Shared) -> Option<Page> {
    let mut goto = None;
    ui.set_width(220.0);
    ui.spacing_mut().item_spacing.y = 4.0;

    // Logo + nom.
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        logo(ui);
        ui.add_space(4.0);
        ui.label(
            RichText::new("RAFTUP")
                .font(egui::FontId::new(22.0, theme::title_bold_family()))
                .extra_letter_spacing(2.0)
                .color(theme::TEXT),
        );
    });
    ui.add_space(18.0);

    let matches = usize::from(shared.status.is_some());
    let links = [
        ("Jeux", 0, Page::Games),
        ("Parties", matches, Page::Matches),
        ("Catalogue", 5, Page::Catalog),
    ];
    for (label, count, page) in links {
        if nav_link(ui, label, count, *current == page) {
            goto = Some(page);
        }
    }

    // Le bas de la barre : carte du serveur et mention de compatibilité.
    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            ui.label(widgets::dim("Compatible avec Archipelago 0.6.8").size(12.0));
        });
        ui.add_space(8.0);
        if let Some(status) = &shared.status {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                Frame::new()
                    .fill(theme::FIELD)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .corner_radius(CornerRadius::same(theme::RADIUS))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(170.0);
                        ui.label(RichText::new("● Serveur en ligne").color(theme::GREEN));
                        let connected = status
                            .server
                            .slots
                            .iter()
                            .filter(|s| !s.is_group && s.connections > 0)
                            .count();
                        let name: String = status.seed.seed_name.chars().take(10).collect();
                        let dots = if status.seed.seed_name.chars().count() > 10 {
                            "…"
                        } else {
                            ""
                        };
                        let plural = if connected > 1 { "s" } else { "" };
                        ui.label(
                            widgets::dim(format!(
                                "seed {name}{dots} · {connected} joueur{plural} connecté{plural}"
                            ))
                            .size(12.0),
                        );
                    });
            });
        }
    });
    goto
}

/// Un lien de navigation : le lien actif a un fond bleu sombre et un bord gauche bleu.
fn nav_link(ui: &mut Ui, label: &str, count: usize, active: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, 0.0, theme::ACCENT_BG);
        let bar = egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height()));
        painter.rect_filled(bar, 0.0, theme::ACCENT);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, Color32::from_white_alpha(10));
    }
    let color = if active { theme::TEXT } else { theme::TEXT_DIM };
    painter.text(
        rect.left_center() + egui::vec2(18.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(15.0),
        color,
    );
    painter.text(
        rect.right_center() - egui::vec2(18.0, 0.0),
        Align2::RIGHT_CENTER,
        count.to_string(),
        egui::FontId::proportional(13.0),
        theme::TEXT_DIM,
    );
    response.clicked()
}

/// Le logo dessiné : deux traits horizontaux, deux verticaux et un petit carré central.
fn logo(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), Sense::hover());
    let painter = ui.painter();
    let stroke = Stroke::new(2.5, theme::ACCENT);
    let at = |fx: f32, fy: f32| rect.min + egui::vec2(rect.width() * fx, rect.height() * fy);
    for f in [0.33, 0.67] {
        painter.line_segment([at(0.04, f), at(0.96, f)], stroke);
        painter.line_segment([at(f, 0.04), at(f, 0.96)], stroke);
    }
    let square = egui::Rect::from_center_size(rect.center(), egui::vec2(6.0, 6.0));
    painter.rect_filled(square, 0.0, theme::ACCENT);
}
