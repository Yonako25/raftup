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

/// Boîte de dialogue modale de Raftup : fond opaque, bordure claire, et le reste de la
/// fenêtre nettement assombri, pour que la boîte se détache de l'écran derrière elle.
/// `egui::Modal` bloque aussi les clics derrière la boîte.
pub fn modal(id: &str) -> egui::Modal {
    egui::Modal::new(egui::Id::unique(id))
        .backdrop_color(Color32::from_black_alpha(170))
        .frame(
            Frame::new()
                .fill(theme::BG)
                .stroke(Stroke::new(1.0, theme::TEXT_DIM.gamma_multiply(0.6)))
                .corner_radius(CornerRadius::same(theme::RADIUS))
                .inner_margin(Margin::same(20)),
        )
}

/// Accorde un mot au nombre : `plural(1, "paquet", "paquets")` donne « 1 paquet ».
pub fn plural(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
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

/// Bloc repliable « Options avancées » (ou autre titre), fermé par défaut : les réglages
/// d'expert (port, chiffrement, webhook…) y vont pour laisser le chemin principal dégagé.
/// `id` distingue deux blocs du même titre sur des écrans différents.
///
/// `open_now` : vrai pour l'ouvrir de force (une erreur porte sur un de ses champs) ;
/// la fonction le remet à faux, pour que le joueur puisse ensuite le refermer.
///
/// En debug, `RAFTUP_DEV_AVANCE=1` les ouvre tous au départ (captures de la macro).
pub fn advanced<R>(
    ui: &mut Ui,
    id: &str,
    heading: &str,
    open_now: &mut bool,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let open_at_start =
        cfg!(debug_assertions) && std::env::var_os("RAFTUP_DEV_AVANCE").is_some_and(|v| v == "1");
    ui.add_space(6.0);
    let mut header = egui::CollapsingHeader::new(RichText::new(heading).color(theme::TEXT_DIM))
        .id_salt(id)
        .default_open(open_at_start);
    if *open_now {
        // `open(Some(true))` impose l'état pour cette image ; ensuite egui s'en souvient.
        header = header.open(Some(true));
        *open_now = false;
    }
    header
        .show(ui, add_contents)
        // `body_returned` vaut `None` quand le bloc est fermé (son contenu n'est pas dessiné).
        .body_returned
}

/// Bouton principal bleu. Renvoie vrai au clic.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> bool {
    let button = egui::Button::new(
        RichText::new(text)
            .color(Color32::from_rgb(0x08, 0x12, 0x22))
            .strong()
            .family(theme::bold_family()),
    )
    .fill(theme::ACCENT);
    ui.add_enabled(enabled, button).clicked()
}

/// Un bouton d'un sélecteur à segments (« Rapide | Standard | Long ») : bordure grise,
/// fond bleu et bordure bleue quand il est choisi. Renvoie vrai au clic.
pub fn segment(ui: &mut Ui, text: &str, selected: bool) -> bool {
    let (fill, stroke, color) = if selected {
        (theme::ACCENT_BG, theme::ACCENT, theme::TEXT)
    } else {
        (Color32::TRANSPARENT, theme::BORDER, theme::TEXT_DIM)
    };
    let button = egui::Button::new(RichText::new(text).color(color).size(15.0))
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .min_size(egui::vec2(92.0, 40.0));
    ui.add(button).clicked()
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
    // Dessinée à la main, à sa taille exacte : un `Frame` placé dans une mise en page
    // droite-à-gauche prenait toute la hauteur de la ligne (vu sur les captures).
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(13.0), fg);
    let size = galley.size() + egui::vec2(20.0, 8.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    // Texte peint à la main : on donne son nom pour un lecteur d'écran (et les tests).
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), bg);
    let text_pos = rect.center() - galley.size() / 2.0;
    painter.galley(text_pos, galley, fg);
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
                .radio(
                    selected,
                    RichText::new(title_text)
                        .strong()
                        .family(theme::bold_family()),
                )
                .clicked();
            ui.label(description);
            if selected {
                extra(ui);
            }
        });
    clicked
}

/// Petit titre en capitales espacées (« FIL D'ACTIVITÉ »), comme sur la maquette.
pub fn caps(text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .size(11.0)
        .extra_letter_spacing(1.5)
        .color(theme::TEXT_DIM)
}

/// Barre de progression fine : fond gris, partie remplie de la couleur donnée.
pub fn bar(ui: &mut Ui, width: f32, fraction: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 4.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, theme::BORDER);
    let mut filled = rect;
    filled.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    painter.rect_filled(filled, 0.0, color);
}

/// Onglets soulignés. `tabs` donne le texte et le compteur de chaque onglet ;
/// `current` est l'indice de l'onglet ouvert, modifié au clic.
pub fn tabs(ui: &mut Ui, tabs: &[(&str, Option<usize>)], current: &mut usize) {
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (index, (label, count)) in tabs.iter().enumerate() {
            let active = index == *current;
            let mut text = label.to_string();
            if let Some(n) = count {
                text.push_str(&format!("  {n}"));
            }
            let color = if active { theme::TEXT } else { theme::TEXT_DIM };
            let response = ui.add(
                egui::Button::new(RichText::new(text).size(15.0).color(color))
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::NONE)
                    .min_size(egui::vec2(0.0, 38.0)),
            );
            if active {
                // Le soulignement bleu de l'onglet ouvert.
                let rect = response.rect;
                let line = egui::Rect::from_min_max(
                    egui::pos2(rect.left(), rect.bottom() - 2.0),
                    rect.right_bottom(),
                );
                ui.painter().rect_filled(line, 0.0, theme::ACCENT);
            }
            if response.clicked() {
                *current = index;
            }
        }
    });
    // Trait fin sous toute la rangée d'onglets.
    ui.painter().hline(
        ui.max_rect().x_range(),
        row.response.rect.bottom(),
        Stroke::new(1.0, theme::BORDER),
    );
}

/// Texte en police monospace de la taille voulue.
pub fn mono(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text.into()).font(FontId::monospace(size))
}

#[cfg(test)]
mod tests {
    use super::plural;

    #[test]
    fn accord_au_nombre() {
        assert_eq!(
            plural(0, "paquet vérifié", "paquets vérifiés"),
            "0 paquets vérifiés"
        );
        assert_eq!(
            plural(1, "paquet vérifié", "paquets vérifiés"),
            "1 paquet vérifié"
        );
        assert_eq!(plural(3, "jeu ajouté", "jeux ajoutés"), "3 jeux ajoutés");
    }
}

/// Durée du fondu d'apparition d'un écran ou d'une fiche, en secondes.
pub const FADE_SECONDS: f32 = 0.18;

/// Opacité d'un fondu d'apparition, `elapsed` secondes après le changement : de 0 à 1
/// en [`FADE_SECONDS`], rapide au début puis plus doux (« ease-out »).
pub fn fade_in(elapsed: f32) -> f32 {
    let x = (elapsed / FADE_SECONDS).clamp(0.0, 1.0);
    1.0 - (1.0 - x) * (1.0 - x)
}

/// Applique le fondu d'apparition à `ui` (tout ce qui est dessiné ensuite) ; demande
/// une nouvelle image tant qu'il n'est pas fini.
pub fn apply_fade(ui: &mut Ui, since: std::time::Instant) {
    let opacity = fade_in(since.elapsed().as_secs_f32());
    if opacity < 1.0 {
        ui.multiply_opacity(opacity);
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod fade_tests {
    use super::*;

    #[test]
    fn fondu_de_0_a_1() {
        assert_eq!(fade_in(0.0), 0.0);
        assert_eq!(fade_in(FADE_SECONDS), 1.0);
        assert_eq!(fade_in(10.0), 1.0);
        assert_eq!(fade_in(-1.0), 0.0);
        // À mi-chemin, déjà aux trois quarts : le début est rapide.
        assert!((fade_in(FADE_SECONDS / 2.0) - 0.75).abs() < 1e-6);
    }
}
