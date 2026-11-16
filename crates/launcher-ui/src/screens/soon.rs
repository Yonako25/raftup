//! Écran « Bientôt » : pour tout ce qui arrive avec la suite du launcher.

use egui::Ui;

use crate::app::Page;
use crate::widgets;

pub fn show(ui: &mut Ui, name: &str) -> Option<Page> {
    super::page(ui, |ui| {
        ui.label(widgets::title(name, 28.0));
        ui.add_space(6.0);
        widgets::panel(ui, "Bientôt", |ui| {
            ui.label("Cet écran arrive avec la suite du launcher.");
        });
    });
    None
}
