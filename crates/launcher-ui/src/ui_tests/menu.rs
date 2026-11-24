//! Barre latérale : les liens montrés, aucun écran « Bientôt ».

use egui_kittest::kittest::Queryable;
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
                // Sans compte : la carte du compte invite à se connecter.
                let summary = Summary {
                    username: None,
                    online: 0,
                };
                if let Some(page) = sidebar(ui, &s.page, &s.shared, 0, 1, &summary) {
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
    // Les écrans qui existent, en deux sections, et « Paramètres » à part.
    for label in [
        "JEUX",
        "Accueil",
        "Bibliothèque",
        "Catalogue",
        "PARTIES",
        "Nouvelle partie",
        "En cours",
        "Rejoindre",
        "Paramètres",
    ] {
        assert!(shows(&harness, label), "lien manquant : {label}");
    }
    // Les écrans à venir (`Page::Soon`) sont cachés, pas annoncés.
    for label in ["Téléchargements", "Salons", "Mises à jour"] {
        assert!(!shows(&harness, label), "lien à cacher : {label}");
    }
    assert!(!shows_part(&harness, "Bientôt"));
    // Une seule carte pour le compte et les amis ; pas de carte du serveur sans partie.
    assert!(shows(&harness, "Joueur"));
    assert!(shows(&harness, "Se connecter pour voir ses amis"));
    assert!(!shows(&harness, "SERVEUR INTÉGRÉ"));
}

#[test]
fn les_liens_ouvrent_leur_ecran() {
    for (label, page) in [
        ("Accueil", Page::Home),
        ("Catalogue", Page::Catalog),
        ("Nouvelle partie", Page::Create),
        ("Rejoindre", Page::Join),
        // La carte du compte, en bas de la barre (« Joueur » sans compte) : l'écran
        // « Amis » montre aussi le profil.
        ("Joueur", Page::Friends),
    ] {
        let mut harness = menu();
        click(&mut harness, label);
        assert_eq!(harness.state().asked, Some(page), "clic sur {label}");
    }
}

/// Plus de bande de titre : les trois boutons de fenêtre sont sur la ligne de titre de
/// l'écran, dans le coin droit, et les actions de la page se rangent à leur gauche.
#[test]
fn boutons_de_fenetre_sur_la_ligne_de_titre() {
    let mut harness = builder().build_ui_state(
        |ui, _: &mut ()| {
            crate::screens::page(ui, |ui| {
                crate::screens::header(ui, "Parties", "Un écran", |ui| {
                    crate::widgets::primary_button(ui, "Action de la page", true);
                });
            });
            crate::titlebar::show(ui.ctx());
        },
        (),
    );
    harness.run();
    let screen = harness.ctx.content_rect();
    let close = harness.get_by_label("Fermer").rect();
    let maximize = harness.get_by_label("Agrandir").rect();
    let minimize = harness.get_by_label("Réduire").rect();
    let action = harness.get_by_label("Action de la page").rect();
    // « Fermer » le plus à droite, près du bord ; les trois dans l'ordre, sans chevauchement.
    assert!(screen.right() - close.right() < 32.0, "{close:?}");
    assert!(minimize.right() <= maximize.left() + 0.5);
    assert!(maximize.right() <= close.left() + 0.5);
    assert!((close.height() - crate::titlebar::HEIGHT).abs() < 0.5);
    // Même ligne que l'action de la page (milieux alignés), avec un espace net entre eux.
    assert!(
        (close.center().y - action.center().y).abs() < 2.0,
        "{close:?} / {action:?}"
    );
    assert!(
        minimize.left() - action.right() >= 12.0,
        "{action:?} / {minimize:?}"
    );
}

/// Un écran sans ligne de titre (l'Accueil) : les boutons restent dans le coin, en haut.
#[test]
fn boutons_de_fenetre_sans_ligne_de_titre() {
    let mut harness =
        builder().build_ui_state(|ui, _: &mut ()| crate::titlebar::show(ui.ctx()), ());
    harness.run();
    let screen = harness.ctx.content_rect();
    let close = harness.get_by_label("Fermer").rect();
    assert!(screen.right() - close.right() < 32.0, "{close:?}");
    assert!(close.top() - screen.top() < 30.0, "{close:?}");
}

#[test]
fn un_clic_sur_parametres_ouvre_l_ecran() {
    let mut harness = menu();
    click(&mut harness, "Paramètres");
    assert_eq!(harness.state().asked, Some(Page::Settings));
}
