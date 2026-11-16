//! Petits composants réutilisés par les écrans : panneaux, boutons, pastilles…

use egui::{Color32, CornerRadius, FontId, Frame, Margin, RichText, Sense, Shape, Stroke, Ui};

use crate::theme;

/// Rend un texte en police de titre.
pub fn title(text: &str, size: f32) -> RichText {
    RichText::new(text)
        .font(theme::title_font(size))
        .color(theme::TEXT)
}

/// Texte secondaire (gris).
pub fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(theme::TEXT_DIM)
}

/// Panneau semi-transparent à bordure fine ; `heading` est son titre.
pub fn panel<R>(ui: &mut Ui, heading: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(theme::panel_fill())
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            // Le panneau prend toute la largeur disponible.
            ui.set_width(ui.available_width());
            if !heading.is_empty() {
                ui.label(title(heading, 17.0));
                ui.add_space(6.0);
            }
            add_contents(ui)
        })
        .inner
}

/// Bouton principal bleu. Renvoie vrai au clic.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> bool {
    let button = egui::Button::new(
        RichText::new(text)
            .color(Color32::from_rgb(0x08, 0x12, 0x22))
            .strong(),
    )
    .fill(theme::ACCENT);
    ui.add_enabled(enabled, button).clicked()
}

/// Bouton discret (fond de panneau, bordure grise).
pub fn ghost_button(ui: &mut Ui, text: &str) -> bool {
    ui.add(egui::Button::new(RichText::new(text).color(theme::TEXT)))
        .clicked()
}

/// Bouton d'action destructrice (bordure rouge, texte rouge clair).
pub fn danger_button(ui: &mut Ui, text: &str) -> bool {
    let button = egui::Button::new(RichText::new(text).color(theme::RED_TEXT))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, theme::RED));
    ui.add(button).clicked()
}

/// Pastille de texte coloré sur fond coloré (« ● En ligne »).
pub fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(fg).size(13.0));
        });
}

/// Message d'erreur rouge.
pub fn error_banner(ui: &mut Ui, message: &str) {
    Frame::new()
        .fill(Color32::from_rgba_unmultiplied(229, 72, 77, 36))
        .stroke(Stroke::new(1.0, theme::RED))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(message).color(theme::RED_TEXT));
        });
}

/// Pastille carrée avec les initiales du jeu ; vide et en pointillés pour un groupe.
pub fn badge(ui: &mut Ui, size: f32, label: &str, is_group: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    let painter = ui.painter();
    let radius = CornerRadius::same(theme::RADIUS);
    if is_group {
        let points = vec![
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
            rect.left_top(),
        ];
        painter.add(Shape::dashed_line(
            &points,
            Stroke::new(1.0, theme::TEXT_DIM),
            3.0,
            3.0,
        ));
    } else {
        painter.rect_filled(rect, radius, theme::ACCENT_BG);
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            theme::title_font(size * 0.38),
            theme::ITEM,
        );
    }
}

/// Une carte de choix (bouton radio + description). Renvoie vrai si on vient de cliquer dessus.
pub fn choice_card<R>(
    ui: &mut Ui,
    selected: bool,
    title_text: &str,
    description: RichText,
    extra: impl FnOnce(&mut Ui) -> R,
) -> bool {
    let stroke = if selected {
        Stroke::new(1.0, theme::ACCENT)
    } else {
        Stroke::new(1.0, theme::BORDER)
    };
    let mut clicked = false;
    Frame::new()
        .fill(theme::FIELD)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            clicked = ui
                .radio(selected, RichText::new(title_text).strong())
                .clicked();
            ui.label(description);
            if selected {
                extra(ui);
            }
        });
    clicked
}

/// Texte en police monospace de la taille voulue.
pub fn mono(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text.into()).font(FontId::monospace(size))
}
