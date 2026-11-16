//! Identité visuelle (SPEC § 7) : palette acier + bleu électrique, angles de 2 px,
//! polices Chakra Petch (titres) et Inter (interface).

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Mesh, Pos2, Rect, Shape,
    Stroke, TextStyle,
};

// Les couleurs sont des constantes : `Color32::from_rgb` peut s'évaluer à la compilation.
pub const BG: Color32 = Color32::from_rgb(0x16, 0x18, 0x1B);
pub const FIELD: Color32 = Color32::from_rgb(0x11, 0x13, 0x16);
pub const BORDER: Color32 = Color32::from_rgb(0x30, 0x34, 0x3A);
pub const TEXT: Color32 = Color32::from_rgb(0xEC, 0xEE, 0xF1);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0xA7, 0xAD, 0xB5);
pub const ACCENT: Color32 = Color32::from_rgb(0x4C, 0x9A, 0xFF);
pub const ACCENT_BG: Color32 = Color32::from_rgb(0x16, 0x29, 0x4A);
pub const ITEM: Color32 = Color32::from_rgb(0x8B, 0xBD, 0xFF);
pub const GREEN: Color32 = Color32::from_rgb(0x4C, 0xC3, 0x8A);
pub const GREEN_BG: Color32 = Color32::from_rgb(0x13, 0x28, 0x1F);
pub const AMBER: Color32 = Color32::from_rgb(0xF2, 0xB4, 0x5C);
pub const RED: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
pub const RED_TEXT: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x8E);
pub const LOG_BG: Color32 = Color32::from_rgb(0x0E, 0x10, 0x13);

/// Angle maximal de la charte : 2 px.
pub const RADIUS: u8 = 2;

/// Fond des panneaux : rgba(32,35,40,0.78).
pub fn panel_fill() -> Color32 {
    Color32::from_rgba_unmultiplied(32, 35, 40, 199)
}

/// Fond de la barre latérale : rgba(17,19,22,0.72).
pub fn sidebar_fill() -> Color32 {
    Color32::from_rgba_unmultiplied(17, 19, 22, 184)
}

/// Famille de police des titres (Chakra Petch SemiBold).
pub fn title_family() -> FontFamily {
    FontFamily::Name("Title".into())
}

/// Famille de police du logo et des gros titres (Chakra Petch Bold).
pub fn title_bold_family() -> FontFamily {
    FontFamily::Name("TitleBold".into())
}

pub fn title_font(size: f32) -> FontId {
    FontId::new(size, title_family())
}

/// Installe les polices et le style egui (appelé une fois au démarrage).
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    // Le thème de la charte est sombre, quel que soit le thème du système.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(style);
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    // Les fichiers TTF sont copiés dans le binaire à la compilation (`include_bytes!`) :
    // le launcher marche hors ligne et ne dépend pas des polices du système.
    let list: [(&str, &[u8]); 3] = [
        ("Inter", include_bytes!("../fonts/Inter.ttf")),
        (
            "ChakraPetch-SemiBold",
            include_bytes!("../fonts/ChakraPetch-SemiBold.ttf"),
        ),
        (
            "ChakraPetch-Bold",
            include_bytes!("../fonts/ChakraPetch-Bold.ttf"),
        ),
    ];
    for (name, bytes) in list {
        fonts.font_data.insert(
            name.to_string(),
            std::sync::Arc::new(FontData::from_static(bytes)),
        );
    }
    // Inter passe devant les polices par défaut pour le texte courant ;
    // celles-ci restent derrière pour les symboles qu'Inter n'a pas.
    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        family.insert(0, "Inter".to_string());
    }
    fonts.families.insert(
        title_family(),
        vec!["ChakraPetch-SemiBold".to_string(), "Inter".to_string()],
    );
    fonts.families.insert(
        title_bold_family(),
        vec!["ChakraPetch-Bold".to_string(), "Inter".to_string()],
    );
    ctx.set_fonts(fonts);
}

fn style(style: &mut egui::Style) {
    let radius = CornerRadius::same(RADIUS);
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = BG;
    v.window_fill = Color32::from_rgb(0x20, 0x23, 0x28);
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = radius;
    v.menu_corner_radius = radius;
    v.extreme_bg_color = FIELD; // fond des champs de saisie
    v.faint_bg_color = Color32::from_white_alpha(6);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT_BG;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;

    let widgets = &mut v.widgets;
    for w in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        w.corner_radius = radius;
    }
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.noninteractive.bg_fill = panel_fill();
    widgets.inactive.bg_fill = Color32::from_rgb(0x20, 0x23, 0x28);
    widgets.inactive.weak_bg_fill = Color32::from_rgb(0x20, 0x23, 0x28);
    widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.hovered.bg_fill = Color32::from_rgb(0x2A, 0x2E, 0x35);
    widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2A, 0x2E, 0x35);
    widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.active.bg_fill = ACCENT_BG;
    widgets.active.weak_bg_fill = ACCENT_BG;
    widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.active.fg_stroke = Stroke::new(1.0, TEXT);

    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(12.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(13.0));
    style
        .text_styles
        .insert(TextStyle::Heading, title_font(24.0));
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.interact_size.y = 32.0;
}

/// Un halo radial : un éventail de triangles dont le centre est coloré et le bord transparent.
/// (egui n'a pas de dégradé radial ; un maillage avec des couleurs aux sommets s'en charge.)
fn halo(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    const STEPS: usize = 48;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    for i in 0..=STEPS {
        let angle = i as f32 / STEPS as f32 * std::f32::consts::TAU;
        let edge = center + egui::vec2(angle.cos(), angle.sin()) * radius;
        mesh.colored_vertex(edge, Color32::TRANSPARENT);
    }
    for i in 0..STEPS as u32 {
        mesh.add_triangle(0, i + 1, i + 2);
    }
    painter.add(Shape::mesh(mesh));
}

/// Fond d'ambiance : deux halos discrets, bleu en haut à gauche et violet en bas à droite.
pub fn paint_background(ctx: &egui::Context) {
    let rect: Rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    let size = rect.width().max(rect.height());
    halo(
        &painter,
        rect.left_top(),
        size * 0.75,
        Color32::from_rgba_unmultiplied(76, 154, 255, 46), // 0,18
    );
    halo(
        &painter,
        rect.right_bottom(),
        size * 0.65,
        Color32::from_rgba_unmultiplied(167, 139, 250, 31), // 0,12
    );
}
