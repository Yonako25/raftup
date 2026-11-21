//! Barre latérale : les liens montrés, aucun écran « Bientôt ».

use egui_kittest::Harness;

use super::{builder, click, shows, shows_part};
use crate::app::{sidebar, Page};
use crate::screens::friends::Summary;
use crate::worker::Shared;

pub struct Menu {
    /// L'écran affiché (le lien actif).
    pub page: Page,
    pub shared: Shared,
    /// La page demandée par le dernier clic, s'il y en a eu un.
    pub asked: Option<Page>,
}

pub fn menu() -> Harness<'static, Menu> {
    let state = Menu {
        page: Page::Library,
        shared: Shared::default(),
        asked: None,
    };
    let mut harness = builder()
        // La barre seule, à la hauteur de la fenêtre de départ du launcher.
        .with_size(egui::vec2(240.0, 820.0))
        .build_ui_state(
            |ui, s: &mut Menu| {
                // Sans compte : la carte « Amis » invite à se connecter.
                let summary = Summary {
                    username: None,
                    online: 0,
                };
                if let Some(page) = sidebar(ui, &s.page, &s.shared, 1, &summary) {
                    s.asked = Some(page);
                }
            },
            state,
        );
    harness.run();
    harness
}

#[test]
fn le_menu_ne_montre_aucune_entree_bientot() {
    let harness = menu();
    // Les écrans qui existent.
    for label in ["Bibliothèque", "En cours", "Invitations", "Paramètres"] {
        assert!(shows(&harness, label), "lien manquant : {label}");
    }
    // Les écrans à venir (`Page::Soon`) sont cachés, pas annoncés.
    for label in [
        "Accueil",
        "Téléchargements",
        "Salons",
        "Parcourir",
        "Mises à jour",
    ] {
        assert!(!shows(&harness, label), "lien à cacher : {label}");
    }
    assert!(!shows_part(&harness, "Bientôt"));
    // La section « CATALOGUE », vidée, disparaît avec son titre.
    assert!(!shows(&harness, "CATALOGUE"));
}

#[test]
fn un_clic_sur_parametres_ouvre_l_ecran() {
    let mut harness = menu();
    click(&mut harness, "Paramètres");
    assert_eq!(harness.state().asked, Some(Page::Settings));
}
