//! Écran « Paramètres » (plan de l'interface, étapes 1 et 5) : l'adresse du Rafthub et
//! les dossiers où Raftup range ses fichiers.
//!
//! Le thème et la langue de la maquette ne sont pas encore là : le launcher n'a qu'un
//! thème (Acier) et qu'une langue, et les ajouter demande des décisions (questions
//! notées dans la PR de l'étape 5). Un réglage absent est caché, pas annoncé.

use egui::Ui;
use launcher_core::Paths;

use super::friends::{self, FriendsState};
use crate::app::Page;
use crate::widgets;

/// Les dossiers de Raftup, dans l'ordre affiché : (nom, explication, chemin).
pub fn folders(paths: &Paths) -> [(&'static str, &'static str, String); 3] {
    [
        (
            "Configuration",
            "Réglages du launcher (config.toml).",
            paths.config_dir.display().to_string(),
        ),
        (
            "Données",
            "Reçus d'installation, générateur, parties générées et sauvegardes.",
            paths.data_dir.display().to_string(),
        ),
        (
            "Cache",
            "Téléchargements et illustrations ; peut être vidé sans rien perdre.",
            paths.cache_dir.display().to_string(),
        ),
    ]
}

/// `paths` : les dossiers de Raftup s'ils sont connus (`None` : panneau caché).
pub fn show(ui: &mut Ui, friends_state: &mut FriendsState, paths: Option<&Paths>) -> Option<Page> {
    super::page(ui, |ui| {
        super::header(ui, "Raftup", "Paramètres", |_| {});
        friends::hub_panel(ui, friends_state);
        if let Some(paths) = paths {
            ui.add_space(12.0);
            folders_panel(ui, paths);
        }
    });
    None
}

/// Panneau « Dossiers » : où sont les fichiers, avec un bouton pour copier le chemin.
fn folders_panel(ui: &mut Ui, paths: &Paths) {
    let mut copy = None;
    widgets::panel(ui, "Dossiers", |ui| {
        for (name, text, path) in folders(paths) {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(name).strong());
                    ui.label(widgets::dim(text).size(12.0));
                    ui.label(widgets::mono(&path, 13.0));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::ghost_button(ui, "Copier") {
                        copy = Some(path.clone());
                    }
                });
            });
            ui.add_space(6.0);
        }
    });
    if let Some(path) = copy {
        ui.ctx().copy_text(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trois_dossiers_dans_l_ordre() {
        let paths = Paths::under(std::path::Path::new("/r"));
        let names: Vec<&str> = folders(&paths).iter().map(|f| f.0).collect();
        assert_eq!(names, ["Configuration", "Données", "Cache"]);
        assert!(folders(&paths)[2].2.ends_with("cache"));
    }
}
