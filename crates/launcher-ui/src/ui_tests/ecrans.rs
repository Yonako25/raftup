//! Écrans « Paramètres », « Rejoindre » et « Amis » (non connecté) : les textes principaux.

use egui_kittest::Harness;
use launcher_core::invite::parse_invite;
use launcher_core::Paths;

use super::{builder, click, count, demo_catalog, shows, shows_part, type_into};
use crate::screens::friends::{self, FriendsState};
use crate::screens::join::{self, JoinState};
use crate::screens::library::{LibraryState, Scan};
use crate::screens::settings;

/// Un écran qui ne dépend que de l'état des amis (« Paramètres » ou « Amis »).
fn with_friends(show: fn(&mut egui::Ui, &mut FriendsState)) -> Harness<'static, FriendsState> {
    let mut harness = builder().build_ui_state(show, FriendsState::for_tests());
    harness.run();
    harness
}

#[test]
fn parametres_montre_l_adresse_du_rafthub() {
    let harness = with_friends(|ui, state| {
        settings::show(ui, state, None);
    });
    assert!(shows(&harness, "Paramètres"));
    assert!(shows(&harness, "Rafthub"));
    assert!(shows_part(&harness, "Le serveur des comptes et des amis"));
    assert!(shows(&harness, "Enregistrer"));
    assert!(shows(&harness, "Par défaut"));
    // Le hub des tests (port 1 de ce PC) est celui « en service ».
    assert!(shows_part(&harness, "En service : 127.0.0.1:1"));
}

#[test]
fn amis_non_connecte_propose_de_se_connecter() {
    let harness = with_friends(|ui, state| {
        friends::show(ui, state);
    });
    assert!(shows(&harness, "Amis"));
    assert!(shows(&harness, "Communauté"));
    assert!(shows_part(
        &harness,
        "Un compte Rafthub sert à retrouver vos amis"
    ));
    // « Se connecter » : le titre du panneau et le bouton (grisé tant que c'est vide).
    assert!(count(&harness, "Se connecter") >= 2);
    assert!(shows(&harness, "Connexion"));
    assert!(shows(&harness, "Nouveau compte"));
    assert!(shows(&harness, "E-mail ou pseudo"));
    assert!(shows(&harness, "Mot de passe"));
    // Rien de ce qui suppose un compte.
    assert!(!shows(&harness, "Se déconnecter"));
}

/// « Rejoindre » a besoin de la Bibliothèque (le lien y mène) : on lui en donne une.
struct Rejoindre {
    join: JoinState,
    library: LibraryState,
    _root: tempfile::TempDir,
}

fn rejoindre() -> Harness<'static, Rejoindre> {
    let root = tempfile::tempdir().unwrap();
    let state = Rejoindre {
        join: JoinState::without_clipboard(),
        library: LibraryState::for_tests(
            demo_catalog(),
            Paths::under(root.path()),
            Scan::default(),
        ),
        _root: root,
    };
    let mut harness = builder().build_ui_state(
        |ui, s: &mut Rejoindre| {
            join::show(ui, &mut s.join, &mut s.library);
        },
        state,
    );
    harness.run();
    harness
}

#[test]
fn rejoindre_attend_un_lien() {
    let harness = rejoindre();
    assert!(shows(&harness, "Rejoindre une partie"));
    assert!(shows(&harness, "Parties › Invitations"));
    assert!(shows(&harness, "Lien d'invitation"));
    assert!(shows(&harness, "Collez le lien d'invitation"));
    assert!(shows(&harness, "Lire le lien"));
    // Rien n'est lu tant qu'aucun lien n'est collé.
    assert!(!shows(&harness, "Vérifiez avant de continuer"));
}

#[test]
fn rejoindre_refuse_un_lien_illisible() {
    let mut harness = rejoindre();
    type_into(&mut harness, "raftup://join?v=1&addr=…", "pas un lien");
    click(&mut harness, "Lire le lien");
    // Le refus affiché est celui de `parse_invite`, mot pour mot.
    let expected = parse_invite("pas un lien").unwrap_err().to_string();
    assert!(shows(&harness, &expected), "refus attendu : {expected}");
}
