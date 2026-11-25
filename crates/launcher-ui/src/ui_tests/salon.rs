//! Écran « Créer une partie », présenté comme le « Salon de préparation » (ADR 0044).

use egui_kittest::Harness;
use launcher_core::crew::{self, CrewMember};

use super::{builder, click, demo_catalog, field, shows, shows_part, type_into};
use crate::screens::create::{self, CreateForm};
use crate::worker::{Shared, Worker};

/// Texte d'aide du champ « Ton pseudo ».
const MY_NAME: &str = "Ton nom dans la partie (16 caractères au plus)";
/// Texte d'aide du pseudo de la ligne « Ajouter un joueur ».
const NEW_NAME: &str = "Pseudo";

/// Ce que `create::show` reçoit à chaque image, gardé par le harnais.
pub struct Salon {
    pub form: CreateForm,
    pub error: Option<String>,
    /// Worker détaché : un clic sur « Générer » ne démarrerait aucun serveur.
    pub worker: Worker,
    pub shared: Shared,
}

/// Le salon avec le catalogue de démonstration (Slime Rancher, choisi d'office).
/// `'static` : le harnais ne garde aucune référence vers une variable du test.
pub fn salon() -> Harness<'static, Salon> {
    let state = Salon {
        form: CreateForm::for_tests(&demo_catalog()),
        error: None,
        worker: Worker::detached(),
        shared: Shared::default(),
    };
    let mut harness = builder().build_ui_state(
        |ui, s: &mut Salon| {
            // Champs distincts de `s` : Rust permet de les emprunter en même temps.
            create::show(ui, &mut s.form, &mut s.error, &s.worker, &s.shared, None);
            // Les boutons de fenêtre, comme dans `App` (après l'écran).
            crate::titlebar::show(ui.ctx());
        },
        state,
    );
    harness.run();
    harness
}

/// Le refus que « Générer et héberger » donnerait tout de suite.
fn refusal(harness: &Harness<'_, Salon>) -> String {
    match create::build_request(&harness.state().form) {
        Ok(_) => panic!("le formulaire devrait être refusé"),
        Err(message) => message,
    }
}

#[test]
fn le_salon_s_affiche() {
    let harness = salon();
    assert!(shows(&harness, "Salon de préparation"));
    assert!(shows(&harness, "Tu organises · 1 joueur"));
    assert!(shows(&harness, "Ton monde"));
    assert!(shows(&harness, "Équipage"));
    assert!(shows(&harness, "AJOUTER UN JOUEUR"));
    // Les versions figées tiennent en une ligne (le panneau « Profil de session » est retiré).
    assert!(!shows(&harness, "PROFIL DE SESSION"));
    assert!(shows_part(&harness, "Générateur 0.6.8 · Raftup "));
    assert!(!shows_part(&harness, "Archipelago"));
    assert!(shows(&harness, "LANCER LA PARTIE"));
    assert!(shows(&harness, "Générer et héberger"));
    // L'hôte sans pseudo s'appelle « Toi » ; son jeu, seul au catalogue, est déjà choisi.
    assert!(shows(&harness, "Toi"));
    assert!(shows_part(&harness, "Slime Rancher · "));
    // Les options d'expert sont repliées : le port n'est pas à l'écran.
    assert!(shows(&harness, "Options avancées"));
    assert!(!shows(&harness, "Port"));
}

#[test]
fn ajouter_un_joueur_le_met_dans_l_equipage() {
    let mut harness = salon();
    type_into(&mut harness, MY_NAME, "Alice");
    type_into(&mut harness, NEW_NAME, "Bob");
    click(&mut harness, "Ajouter");

    assert!(shows(&harness, "Alice (toi)"));
    assert!(shows(&harness, "Bob"));
    assert!(shows(&harness, "Tu organises · 2 joueurs"));
    // Le champ est vidé pour le joueur suivant, et Bob peut être retiré.
    let value = field(&harness, NEW_NAME).value().unwrap_or_default();
    assert_eq!(value, "");
    assert!(shows(&harness, "Retirer"));
}

#[test]
fn un_pseudo_en_double_est_refuse() {
    let mut harness = salon();
    type_into(&mut harness, MY_NAME, "Alice");
    type_into(&mut harness, NEW_NAME, "Alice");
    click(&mut harness, "Ajouter");

    // Le message affiché est celui de `crew::check_name`, mot pour mot.
    let taken = CrewMember {
        name: "Alice".to_string(),
        ap_game: String::new(),
    };
    let expected = crew::check_name("Alice", &[taken]).unwrap_err().to_string();
    assert!(shows(&harness, &expected), "refus attendu : {expected}");
    // Personne n'a été ajouté.
    assert!(shows(&harness, "Tu organises · 1 joueur"));
}

#[test]
fn lancer_la_partie_annonce_le_refus_reel() {
    let mut harness = salon();
    // Sans pseudo : c'est le pseudo qui manque.
    let first = refusal(&harness);
    assert_eq!(first, "Indique ton pseudo.");
    assert!(shows(&harness, &first), "refus attendu : {first}");

    // Avec un pseudo : plus rien ne bloque. Le chiffrement par défaut ne demande
    // aucun fichier (certificat généré par Raftup, ADR 0043).
    type_into(&mut harness, MY_NAME, "Alice");
    match create::build_request(&harness.state().form) {
        Ok((_, host)) => assert!(
            matches!(host.encryption, launcher_core::Encryption::Generated { .. }),
            "{:?}",
            host.encryption
        ),
        // Seul refus possible : pas de dossier personnel sur la machine de test.
        Err(message) => assert!(message.contains("Dossiers de Raftup"), "{message}"),
    }
    assert!(!shows(&harness, &first));
}
