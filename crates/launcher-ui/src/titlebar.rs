//! Bande de titre intégrée : la fenêtre est sans bordure (pas de barre de Windows), Raftup
//! dessine la sienne (revue d'ergonomie, point 24, qui revient sur les points 19 et 20).
//!
//! La bande prend toute la largeur, en haut, dans la couleur de la barre latérale : elle
//! porte le logo et le nom de Raftup à gauche (au-dessus du menu), les trois boutons réduire
//! / agrandir / fermer à droite, et un trait fin dessous. Glisser la bande déplace la
//! fenêtre ; un double-clic l'agrandit ou la restaure.
//!
//! Chaque écran dessine sa ligne de titre avec [`title_row`] : une simple ligne, puisque les
//! boutons de fenêtre ne sont plus dessus.

use egui::{vec2, Align, Color32, CornerRadius, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use egui::{RichText, ViewportCommand};

use crate::theme;

/// Hauteur de la bande.
pub const HEIGHT: f32 = 40.0;
/// Taille d'un bouton de fenêtre.
const BUTTON_WIDTH: f32 = 40.0;
const BUTTON_HEIGHT: f32 = 30.0;
/// Écart entre deux boutons de fenêtre.
const BUTTON_GAP: f32 = 2.0;
/// Marge entre « Fermer » et le bord droit de la fenêtre.
const RIGHT_MARGIN: f32 = 6.0;

/// Les trois boutons de la fenêtre.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Caption {
    Minimize,
    Maximize,
    Close,
}

/// Ligne de titre d'un écran : `add_contents` y met le titre et, à droite (mise en page de
/// droite à gauche), les actions de la page. Gardée comme point commun des écrans, pour
/// que leurs titres restent alignés.
pub fn title_row<R>(
    ui: &mut Ui,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.horizontal(add_contents)
}

/// Dessine la bande en haut de `ui`. À appeler **avant** la barre latérale et l'écran :
/// un panneau du haut déclaré en premier prend toute la largeur.
pub fn show(ui: &mut Ui) {
    egui::Panel::top("titlebar")
        .exact_size(HEIGHT)
        .resizable(false)
        .frame(egui::Frame::new().fill(theme::sidebar_fill()))
        .show(ui, contents);
}

/// Le contenu de la bande : poignée, logo et nom, boutons, trait du bas.
fn contents(ui: &mut Ui) {
    // `Context` est un pointeur partagé : le cloner est peu coûteux et évite d'emprunter `ui`.
    let ctx = ui.ctx().clone();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));

    // Toute la bande sert de poignée. Elle est déclarée avant les boutons : en cas de
    // chevauchement, egui donne le clic au dernier widget ajouté, donc aux boutons.
    let bar = ui.max_rect();
    let grip = ui.interact(bar, ui.scope_id().with("grip"), Sense::click_and_drag());
    if grip.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if grip.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }

    // Trait fin sous la bande.
    ui.painter().hline(
        bar.x_range(),
        bar.bottom() - 0.5,
        Stroke::new(1.0, theme::BORDER),
    );

    ui.horizontal_centered(|ui| {
        // Logo et nom, alignés sur les liens du menu (18 px du bord).
        ui.add_space(18.0);
        crate::app::logo(ui, 22.0);
        ui.add_space(2.0);
        ui.label(
            RichText::new("RAFTUP")
                .font(egui::FontId::new(16.0, theme::title_bold_family()))
                .extra_letter_spacing(2.0)
                .color(theme::TEXT),
        );

        // Les boutons au bord droit, « Fermer » le plus à droite.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(RIGHT_MARGIN);
            ui.spacing_mut().item_spacing.x = BUTTON_GAP;
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
    });
}

/// Un bouton de fenêtre, au style du thème : icône grise, fond de champ au survol (rouge
/// pour « Fermer »), angles de 2 px. Renvoie vrai au clic.
fn caption_button(ui: &mut Ui, kind: Caption, maximized: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(vec2(BUTTON_WIDTH, BUTTON_HEIGHT), Sense::click());
    // Le bouton est peint à la main : sans nom, un lecteur d'écran (et les tests) ne
    // sauraient pas ce qu'il fait.
    let name = match kind {
        Caption::Minimize => "Réduire",
        Caption::Maximize if maximized => "Restaurer",
        Caption::Maximize => "Agrandir",
        Caption::Close => "Fermer",
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    let radius = CornerRadius::same(theme::RADIUS);
    if response.hovered() {
        let fill: Color32 = if kind == Caption::Close {
            theme::RED
        } else {
            theme::FIELD
        };
        painter.rect_filled(rect, radius, fill);
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, theme::BORDER),
            StrokeKind::Inside,
        );
    }
    let color = if response.hovered() {
        theme::TEXT
    } else {
        theme::TEXT_DIM
    };
    let stroke = Stroke::new(1.2, color);
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
