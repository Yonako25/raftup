//! Messages éphémères (« toasts ») : une petite carte apparaît en bas à droite de la
//! fenêtre après une action (« Slime Rancher installé. »), reste quelques secondes, puis
//! s'efface en fondu. Ils complètent les bandeaux des écrans, qui restent affichés.

use std::time::Instant;

use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke};

use crate::theme;

/// Durée d'apparition (fondu d'entrée), en secondes.
const FADE_IN: f32 = 0.18;
/// Durée pendant laquelle le message reste pleinement visible.
const VISIBLE: f32 = 4.0;
/// Durée du fondu de sortie.
const FADE_OUT: f32 = 0.5;
/// Nombre de messages affichés en même temps au plus (les plus anciens partent).
const MAX_SHOWN: usize = 3;

/// Sorte de message : la couleur de sa bordure en dépend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
}

struct Toast {
    text: String,
    kind: ToastKind,
    born: Instant,
}

/// La pile des messages en cours.
#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

/// Opacité d'un message `age` secondes après son arrivée : fondu d'entrée, pleine
/// opacité, fondu de sortie, puis 0 (le message est alors retiré).
pub fn toast_alpha(age: f32) -> f32 {
    if age < 0.0 {
        0.0
    } else if age < FADE_IN {
        age / FADE_IN
    } else if age < FADE_IN + VISIBLE {
        1.0
    } else {
        (1.0 - (age - FADE_IN - VISIBLE) / FADE_OUT).max(0.0)
    }
}

impl Toasts {
    /// Ajoute un message ; au-delà de [`MAX_SHOWN`], le plus ancien est retiré.
    pub fn push(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.items.push(Toast {
            text: text.into(),
            kind,
            born: Instant::now(),
        });
        if self.items.len() > MAX_SHOWN {
            self.items.remove(0);
        }
    }

    /// Nombre de messages encore visibles (pour les tests).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Tests et captures : un message arrivé il y a `age` secondes (déjà visible).
    #[cfg(test)]
    pub fn push_aged(&mut self, text: &str, kind: ToastKind, age: f32) {
        self.push(text, kind);
        if let Some(last) = self.items.last_mut() {
            last.born = Instant::now() - std::time::Duration::from_secs_f32(age);
        }
    }

    /// Dessine les messages en bas à droite, du plus récent (en bas) au plus ancien.
    pub fn show(&mut self, ctx: &egui::Context) {
        // Les messages finis disparaissent.
        self.items
            .retain(|toast| toast_alpha(toast.born.elapsed().as_secs_f32()) > 0.0);
        if self.items.is_empty() {
            return;
        }
        // Une image de plus tant qu'un message est à l'écran (fondus, puis retrait).
        ctx.request_repaint();
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-20.0, -20.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_max_width(340.0);
                for toast in &self.items {
                    let alpha = toast_alpha(toast.born.elapsed().as_secs_f32());
                    // `scope` : l'opacité ne vaut que pour ce message.
                    ui.scope(|ui| {
                        ui.multiply_opacity(alpha);
                        card(ui, toast);
                    });
                    ui.add_space(8.0);
                }
            });
    }
}

/// Une carte de message : fond de panneau, liseré vert ou rouge, texte clair.
fn card(ui: &mut egui::Ui, toast: &Toast) {
    let (border, mark) = match toast.kind {
        ToastKind::Success => (theme::GREEN, "●"),
        ToastKind::Error => (theme::RED, "●"),
    };
    Frame::new()
        .fill(theme::sidebar_fill())
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(14, 10))
        .shadow(egui::epaint::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(120),
        })
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(mark).color(border).strong());
                ui.label(RichText::new(&toast.text).color(theme::TEXT).size(14.0));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_message_apparait_reste_puis_s_efface() {
        assert_eq!(toast_alpha(-1.0), 0.0);
        assert_eq!(toast_alpha(0.0), 0.0);
        assert!((toast_alpha(FADE_IN / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(toast_alpha(1.0), 1.0);
        // Juste avant la fin de la période visible (pas pile dessus : arrondi des flottants).
        assert_eq!(toast_alpha(FADE_IN + VISIBLE - 0.01), 1.0);
        assert!((toast_alpha(FADE_IN + VISIBLE + FADE_OUT / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(toast_alpha(FADE_IN + VISIBLE + FADE_OUT), 0.0);
        assert_eq!(toast_alpha(60.0), 0.0);
    }

    #[test]
    fn au_plus_trois_messages() {
        let mut toasts = Toasts::default();
        for i in 0..5 {
            toasts.push(format!("message {i}"), ToastKind::Success);
        }
        assert_eq!(toasts.len(), MAX_SHOWN);
        assert_eq!(toasts.items[0].text, "message 2");
    }
}
