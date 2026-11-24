//! Boutons de fenêtre intégrés : la fenêtre est sans bordure (pas de barre de Windows) et
//! sans bande de titre (demande du 24 novembre 2026, revue d'ergonomie, point 19).
//!
//! Les trois boutons réduire / agrandir / fermer flottent dans le coin haut droit, par-dessus
//! la page. Pour déplacer la fenêtre (glisser) ou l'agrandir (double-clic), on attrape le
//! haut de la barre latérale (le logo, voir `app::sidebar`) ou la fine bande vide tout en
//! haut de la fenêtre.

use egui::{
    vec2, Color32, Context, Id, Order, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    ViewportCommand,
};

use crate::theme;

/// Hauteur des boutons de fenêtre. Les boutons des en-têtes de page commencent plus bas :
/// ils ne passent pas dessous.
pub const HEIGHT: f32 = 30.0;
/// Hauteur de la bande invisible, tout en haut, qui sert à déplacer la fenêtre. Elle reste
/// fine : en dessous commencent les pages (marge de 22 px) et le logo de la barre latérale.
const GRIP_HEIGHT: f32 = 10.0;
/// Largeur d'un bouton de fenêtre.
const BUTTON_WIDTH: f32 = 46.0;

/// Les trois boutons de la fenêtre.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Caption {
    Minimize,
    Maximize,
    Close,
}

/// Dessine les trois boutons dans le coin haut droit et la bande pour déplacer la fenêtre.
/// À appeler après les panneaux : les deux vont dans la couche du premier plan
/// (`Order::Foreground`), au-dessus des pages.
pub fn show(ctx: &Context) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let screen = ctx.content_rect();
    let buttons_width = 3.0 * BUTTON_WIDTH;

    // Bande invisible tout en haut, à gauche des boutons.
    egui::Area::new(Id::unique("window_grip"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let size = vec2((screen.width() - buttons_width).max(0.0), GRIP_HEIGHT);
            let (_, grip) = ui.allocate_exact_size(size, Sense::click_and_drag());
            drag_window(ctx, &grip, maximized);
        });

    // Les boutons, collés au bord droit, le plus à droite étant « Fermer ».
    egui::Area::new(Id::unique("window_buttons"))
        .order(Order::Foreground)
        .fixed_pos(Pos2::new(screen.right() - buttons_width, screen.top()))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal(|ui| {
                if caption_button(ui, Caption::Minimize, maximized) {
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                }
                if caption_button(ui, Caption::Maximize, maximized) {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                }
                if caption_button(ui, Caption::Close, maximized) {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            });
        });
}

/// Glisser déplace la fenêtre, un double-clic l'agrandit ou la restaure. Partagé par la
/// bande du haut et le logo de la barre latérale.
pub fn drag_window(ctx: &Context, response: &Response, maximized: bool) {
    if response.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if response.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
}

/// Rend une zone déjà dessinée (le logo) capable de déplacer la fenêtre.
pub fn grip(ui: &Ui, rect: Rect, id: Id) {
    let ctx = ui.ctx().clone();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    drag_window(&ctx, &response, maximized);
}

/// Un bouton de fenêtre ; renvoie vrai au clic.
fn caption_button(ui: &mut Ui, kind: Caption, maximized: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON_WIDTH, HEIGHT), Sense::click());
    // Le bouton est peint à la main : sans nom, un lecteur d'écran (et les tests) ne
    // sauraient pas ce qu'il fait.
    let name = match kind {
        Caption::Minimize => "Réduire",
        Caption::Maximize if maximized => "Restaurer",
        Caption::Maximize => "Agrandir",
        Caption::Close => "Fermer",
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
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
