//! Boutons de fenêtre intégrés : la fenêtre est sans bordure (pas de barre de Windows) et
//! sans bande de titre (revue d'ergonomie, points 19 et 20).
//!
//! Les trois boutons réduire / agrandir / fermer sont posés dans le coin haut droit, sur la
//! même ligne que le titre de l'écran (« Bibliothèque », « En cours »…). Chaque écran dessine
//! sa ligne de titre avec [`title_row`] : elle laisse la place des boutons à droite (les
//! actions de la page se rangent à leur gauche) et dit à quelle hauteur les poser.
//!
//! Pour déplacer la fenêtre (glisser) ou l'agrandir (double-clic) : la ligne de titre, le
//! logo de la barre latérale, ou la fine bande vide tout en haut de la fenêtre.

use egui::{
    vec2, Color32, Context, CornerRadius, Id, Order, Pos2, Rect, Response, Sense, Stroke,
    StrokeKind, Ui, ViewportCommand,
};

use crate::theme;

/// Taille d'un bouton de fenêtre.
const BUTTON_WIDTH: f32 = 40.0;
pub const HEIGHT: f32 = 32.0;
/// Écart entre deux boutons de fenêtre.
const BUTTON_GAP: f32 = 2.0;
/// Marge entre le dernier bouton (« Fermer ») et le bord droit de la fenêtre : celle des
/// pages (28 px), pour que les boutons s'alignent sur le bord des panneaux et des bandeaux.
const RIGHT_MARGIN: f32 = 28.0;
/// Espace net entre les actions de la page et les boutons de fenêtre.
const ACTIONS_GAP: f32 = 16.0;
/// Largeur du groupe des trois boutons.
const GROUP_WIDTH: f32 = 3.0 * BUTTON_WIDTH + 2.0 * BUTTON_GAP;
/// Place gardée à droite de chaque ligne de titre, mesurée depuis le bord de la fenêtre.
const RESERVED: f32 = RIGHT_MARGIN + GROUP_WIDTH + ACTIONS_GAP;
/// Hauteur de la bande invisible, tout en haut, qui sert aussi à déplacer la fenêtre.
const GRIP_HEIGHT: f32 = 10.0;
/// Hauteur des boutons sur un écran sans ligne de titre (l'Accueil et sa vitrine) : le
/// haut des boutons sur la marge haute des pages (22 px).
const FALLBACK_TOP: f32 = 22.0;

/// Les trois boutons de la fenêtre.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Caption {
    Minimize,
    Maximize,
    Close,
}

/// Clé (dans la mémoire d'egui) du milieu vertical de la ligne de titre dessinée à cette
/// image. L'écran l'écrit, [`show`] la lit puis l'efface : un écran sans ligne de titre
/// ne garde pas la hauteur de l'écran précédent.
fn row_center_key() -> Id {
    Id::unique("raftup_title_row_center")
}

/// Ligne de titre d'un écran : `add_contents` y met le titre et, à droite (mise en page de
/// droite à gauche), les actions de la page. La ligne :
/// - s'arrête avant les boutons de fenêtre, avec un espace net entre les deux groupes ;
/// - se glisse pour déplacer la fenêtre, et un double-clic l'agrandit (hors des boutons) ;
/// - donne sa hauteur aux boutons de fenêtre, qui se posent à son milieu.
pub fn title_row<R>(
    ui: &mut Ui,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let ctx = ui.ctx().clone();
    let id = ui.scope_id().with("title_row");
    // La hauteur de la ligne n'est connue qu'après l'avoir dessinée : on reprend celle de
    // l'image précédente, gardée dans la mémoire d'egui.
    let height: f32 = ctx.data(|d| d.get_temp(id)).unwrap_or(HEIGHT);
    let grip_rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), height));
    // La poignée est déclarée AVANT le contenu : en cas de chevauchement, egui donne le
    // clic au dernier widget ajouté, donc aux boutons de la page.
    grip(ui, grip_rect, id.with("grip"));

    let limit = ctx.content_rect().right() - RESERVED;
    let inner = ui.horizontal(|ui| {
        // Le bord droit de la ligne recule jusqu'avant les boutons de fenêtre : les actions
        // de la page (à droite) s'arrêtent donc là.
        ui.set_max_width((limit - ui.min_rect().left()).max(0.0));
        add_contents(ui)
    });
    let rect = inner.response.rect;
    if (rect.height() - height).abs() > 0.5 {
        ctx.data_mut(|d| d.insert_temp(id, rect.height()));
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(row_center_key(), rect.center().y));
    inner
}

/// Dessine les trois boutons et la bande du haut. À appeler après les panneaux (et donc
/// après la ligne de titre de l'écran) : les deux vont au premier plan (`Order::Foreground`).
pub fn show(ctx: &Context) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let screen = ctx.content_rect();
    // `remove_temp` : lue une seule fois, voir `row_center_key`.
    let row_center: Option<f32> = ctx.data_mut(|d| d.remove_temp(row_center_key()));
    let top = match row_center {
        // Sur le milieu de la ligne de titre ; jamais au-dessus de la bande du haut (la
        // ligne a pu défiler hors de l'écran).
        Some(center) => (center - HEIGHT / 2.0).max(screen.top() + GRIP_HEIGHT),
        None => screen.top() + FALLBACK_TOP,
    };

    // Bande invisible tout en haut, sur toute la largeur.
    egui::Area::new(Id::unique("window_grip"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (_, grip) =
                ui.allocate_exact_size(vec2(screen.width(), GRIP_HEIGHT), Sense::click_and_drag());
            drag_window(ctx, &grip, maximized);
        });

    // Les boutons, « Fermer » le plus à droite. Area déclarée après la bande : elle passe
    // au-dessus.
    let left = screen.right() - RIGHT_MARGIN - GROUP_WIDTH;
    egui::Area::new(Id::unique("window_buttons"))
        .order(Order::Foreground)
        .fixed_pos(Pos2::new(left, top))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.x = BUTTON_GAP;
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
/// bande du haut, la ligne de titre et le logo de la barre latérale.
pub fn drag_window(ctx: &Context, response: &Response, maximized: bool) {
    if response.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if response.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
}

/// Rend une zone (le logo, une ligne de titre) capable de déplacer la fenêtre.
pub fn grip(ui: &Ui, rect: Rect, id: Id) {
    let ctx = ui.ctx().clone();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    drag_window(&ctx, &response, maximized);
}

/// Un bouton de fenêtre, au style du thème : icône grise, fond de champ au survol (rouge
/// pour « Fermer »), angles de 2 px. Renvoie vrai au clic.
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
