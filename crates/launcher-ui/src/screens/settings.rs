//! Écran « Paramètres » : pour l'instant, l'adresse du Rafthub seulement (plan de
//! l'interface, étape 1). Les dossiers, la langue et le thème arriveront à l'étape 5.

use egui::Ui;

use super::friends::{self, FriendsState};
use crate::app::Page;

pub fn show(ui: &mut Ui, friends_state: &mut FriendsState) -> Option<Page> {
    super::page(ui, |ui| {
        super::header(ui, "Raftup", "Paramètres", |_| {});
        friends::hub_panel(ui, friends_state);
    });
    None
}
