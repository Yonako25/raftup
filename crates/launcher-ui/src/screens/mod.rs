//! Les écrans du launcher. Chaque écran dessine dans un `Ui` et renvoie,
//! s'il y en a une, la page vers laquelle l'utilisateur veut aller.

pub mod create;
pub mod estimate;
pub mod friends;
pub mod generation;
pub mod generator_setup;
pub mod hosting;
pub mod join;
pub mod library;
pub mod matches;
pub mod native_panel;
pub mod prefs;
pub mod presets;
pub mod registry_panel;
pub mod running;
pub mod security;
pub mod settings;
pub mod setup;
pub mod soon;

use egui::{Margin, ScrollArea, Ui};

/// Largeur à partir de laquelle deux panneaux se placent côte à côte.
pub const WIDE: f32 = 980.0;

/// Zone défilante avec les marges de la page.
pub fn page<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(28, 22))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    add_contents(ui)
                })
                .inner
        })
        .inner
}

/// En-tête d'écran : fil d'Ariane, titre, et boutons alignés à droite (ajoutés par `buttons`).
pub fn header(ui: &mut Ui, breadcrumb: &str, heading: &str, buttons: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(crate::widgets::dim(breadcrumb).size(13.0));
            ui.label(crate::widgets::title(heading, 28.0));
        });
        // De droite à gauche : le premier bouton ajouté est le plus à droite.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            buttons(ui)
        });
    });
    ui.add_space(10.0);
}
