//! Barre de titre intégrée : remplace celle de Windows (la fenêtre est sans bordure).
//!
//! Elle sert à déplacer la fenêtre (glisser), à l'agrandir (double-clic) et porte les
//! trois boutons réduire / agrandir / fermer.

use egui::{
    vec2, Align, Color32, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, ViewportCommand,
};

use crate::theme;

/// Hauteur de la barre.
pub const HEIGHT: f32 = 34.0;
/// Largeur d'un bouton de fenêtre.
const BUTTON_WIDTH: f32 = 46.0;

/// Les trois boutons de la fenêtre.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Caption {
    Minimize,
    Maximize,
    Close,
}

/// Dessine la barre dans `ui` (le panneau du haut, sans marge).
pub fn show(ui: &mut Ui) {
    // `Context` est un pointeur partagé : le cloner est peu coûteux et évite d'emprunter `ui`.
    let ctx = ui.ctx().clone();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));

    // Toute la barre sert de poignée. Elle est déclarée avant les boutons : en cas de
    // chevauchement, egui donne le clic au dernier widget ajouté, donc aux boutons.
    let bar = ui.max_rect();
    let grip = ui.interact(bar, ui.scope_id().with("grip"), Sense::click_and_drag());
    if grip.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if grip.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }

    // Trait fin sous la barre.
    ui.painter().hline(
        bar.x_range(),
        bar.bottom() - 0.5,
        Stroke::new(1.0, theme::BORDER),
    );

    // Les boutons sont collés au bord droit, le plus à droite étant « Fermer ».
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if caption_button(ui, Caption::Close, maximized) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if caption_button(ui, Caption::Maximize, maximized) {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }
        if caption_button(ui, Caption::Minimize, maximized) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
    });
}

/// Un bouton de fenêtre ; renvoie vrai au clic.
fn caption_button(ui: &mut Ui, kind: Caption, maximized: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON_WIDTH, HEIGHT), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        // « Fermer » devient rouge au survol, comme sous Windows.
        let fill = if kind == Caption::Close {
            theme::RED
        } else {
            Color32::from_white_alpha(18)
        };
        painter.rect_filled(rect, 0.0, fill);
    }
    let color = if response.hovered() {
        theme::TEXT
    } else {
        theme::TEXT_DIM
    };
    let stroke = Stroke::new(1.0, color);
    let c = rect.center();
    let half = 5.0;
    match kind {
        Caption::Minimize => {
            painter.hline(c.x - half..=c.x + half, c.y.round() + 0.5, stroke);
        }
        Caption::Maximize if maximized => {
            // Deux carrés qui se chevauchent : « restaurer ».
            let front =
                Rect::from_min_size(Pos2::new(c.x - half, c.y - half + 2.0), vec2(8.0, 8.0));
            let back = front.translate(vec2(2.0, -2.0));
            painter.rect_stroke(front, 0.0, stroke, StrokeKind::Inside);
            painter.line_segment([back.left_top(), back.right_top()], stroke);
            painter.line_segment([back.right_top(), back.right_bottom()], stroke);
        }
        Caption::Maximize => {
            let square = Rect::from_center_size(c, vec2(half * 2.0, half * 2.0));
            painter.rect_stroke(square, 0.0, stroke, StrokeKind::Inside);
        }
        Caption::Close => {
            painter.line_segment([c + vec2(-half, -half), c + vec2(half, half)], stroke);
            painter.line_segment([c + vec2(-half, half), c + vec2(half, -half)], stroke);
        }
    }
    response.clicked()
}
