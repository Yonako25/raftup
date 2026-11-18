//! Redimensionnement de la fenêtre sans bordure : Windows ne fournit plus de bords à
//! saisir, on détecte donc nous-mêmes le pointeur près des bords et des coins.
//!
//! Le choix de la direction est une fonction pure (`direction_at`), testée ; `handle`
//! ne fait que la brancher sur le curseur et sur `ViewportCommand::BeginResize`.

use egui::{CursorIcon, Pos2, Rect, ResizeDirection, ViewportCommand};

/// Épaisseur de la poignée sur un bord, en points.
const EDGE: f32 = 5.0;
/// Côté de la zone d'un coin, en points (plus large que le bord, plus facile à viser).
const CORNER: f32 = 12.0;

/// La direction de redimensionnement sous `pos`, si le pointeur est près d'un bord
/// ou d'un coin de `window`. `None` au milieu de la fenêtre ou hors de celle-ci.
pub fn direction_at(pos: Pos2, window: Rect) -> Option<ResizeDirection> {
    if !window.contains(pos) {
        return None;
    }
    // Distance à chaque bord : une zone de coin demande d'être près de DEUX bords.
    let left = pos.x - window.left();
    let right = window.right() - pos.x;
    let top = pos.y - window.top();
    let bottom = window.bottom() - pos.y;
    // Un coin est une bande en L : près des deux bords (12 pt), et collé à l'un d'eux
    // (5 pt). Un carré plein de 12 pt mordrait sur le bouton « Fermer », en haut à droite.
    let on_edge = left < EDGE || right < EDGE || top < EDGE || bottom < EDGE;
    let near_left = left < CORNER && on_edge;
    let near_right = right < CORNER && on_edge;
    let near_top = top < CORNER && on_edge;
    let near_bottom = bottom < CORNER && on_edge;

    match (near_left, near_right, near_top, near_bottom) {
        (true, _, true, _) => Some(ResizeDirection::NorthWest),
        (_, true, true, _) => Some(ResizeDirection::NorthEast),
        (true, _, _, true) => Some(ResizeDirection::SouthWest),
        (_, true, _, true) => Some(ResizeDirection::SouthEast),
        _ if left < EDGE => Some(ResizeDirection::West),
        _ if right < EDGE => Some(ResizeDirection::East),
        _ if top < EDGE => Some(ResizeDirection::North),
        _ if bottom < EDGE => Some(ResizeDirection::South),
        _ => None,
    }
}

/// Le curseur à montrer pour une direction.
fn cursor(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    }
}

/// À appeler à chaque image, après le dessin : curseur + début du redimensionnement.
/// Inactif quand la fenêtre est agrandie ou en plein écran.
pub fn handle(ctx: &egui::Context) {
    let (maximized, fullscreen) = ctx.input(|i| {
        (
            i.viewport().maximized.unwrap_or(false),
            i.viewport().fullscreen.unwrap_or(false),
        )
    });
    if maximized || fullscreen {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let Some(direction) = direction_at(pos, ctx.content_rect()) else {
        return;
    };
    ctx.set_cursor_icon(cursor(direction));
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Rect {
        Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 800.0))
    }

    #[test]
    fn milieu_et_hors_fenetre_sans_direction() {
        assert_eq!(direction_at(Pos2::new(500.0, 400.0), window()), None);
        assert_eq!(direction_at(Pos2::new(-1.0, 400.0), window()), None);
        assert_eq!(direction_at(Pos2::new(1001.0, 400.0), window()), None);
    }

    #[test]
    fn les_quatre_bords() {
        let w = window();
        assert_eq!(
            direction_at(Pos2::new(2.0, 400.0), w),
            Some(ResizeDirection::West)
        );
        assert_eq!(
            direction_at(Pos2::new(998.0, 400.0), w),
            Some(ResizeDirection::East)
        );
        assert_eq!(
            direction_at(Pos2::new(500.0, 2.0), w),
            Some(ResizeDirection::North)
        );
        assert_eq!(
            direction_at(Pos2::new(500.0, 798.0), w),
            Some(ResizeDirection::South)
        );
        // Au-delà de l'épaisseur du bord : rien.
        assert_eq!(direction_at(Pos2::new(6.0, 400.0), w), None);
    }

    #[test]
    fn les_quatre_coins() {
        let w = window();
        assert_eq!(
            direction_at(Pos2::new(3.0, 3.0), w),
            Some(ResizeDirection::NorthWest)
        );
        assert_eq!(
            direction_at(Pos2::new(997.0, 3.0), w),
            Some(ResizeDirection::NorthEast)
        );
        assert_eq!(
            direction_at(Pos2::new(3.0, 797.0), w),
            Some(ResizeDirection::SouthWest)
        );
        assert_eq!(
            direction_at(Pos2::new(997.0, 797.0), w),
            Some(ResizeDirection::SouthEast)
        );
        // Près d'un bord mais loin de l'autre : bord simple, pas coin.
        assert_eq!(
            direction_at(Pos2::new(3.0, 100.0), w),
            Some(ResizeDirection::West)
        );
        // Le coin est une bande en L : à 8 pt des deux bords, on est déjà dans le
        // bouton « Fermer » (en haut à droite), pas de redimensionnement.
        assert_eq!(direction_at(Pos2::new(992.0, 8.0), w), None);
        // Collé au bord droit, encore à moins de 12 pt du haut : coin.
        assert_eq!(
            direction_at(Pos2::new(997.0, 10.0), w),
            Some(ResizeDirection::NorthEast)
        );
    }
}
