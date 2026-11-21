//! Écran « Bibliothèque » avec le catalogue de démonstration (`profiles/`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use launcher_core::{DetectedGame, GameState, InstallStatus, Paths, Store};

use super::{builder, click, count, demo_catalog, shows};
use crate::screens::library::{self, LibraryState, Scan};
use crate::worker::Shared;

/// Identifiant du jeu du catalogue de démonstration.
const GAME: &str = "slime-rancher";
/// Texte d'aide du champ « Adresse » de « Se connecter à la main ».
const ADDRESS_HINT: &str = "ws://hote:38281";

pub struct Bibliotheque {
    pub library: LibraryState,
    pub shared: Shared,
    /// Le dossier temporaire doit vivre aussi longtemps que l'écran : il est effacé
    /// quand cette valeur est détruite (`Drop`), donc à la fin du test.
    _root: tempfile::TempDir,
}

/// Une détection « faite » : le jeu trouvé via Steam, ses mods installés.
fn scan_ready() -> Scan {
    let mut found = BTreeMap::new();
    found.insert(
        GAME.to_string(),
        DetectedGame {
            game_id: GAME.to_string(),
            store: Store::Steam,
            install_dir: PathBuf::from("C:/Jeux/Slime Rancher"),
            build_id: None,
            state: GameState::Ready,
        },
    );
    let mut status = BTreeMap::new();
    status.insert(GAME.to_string(), Ok(InstallStatus::Installed));
    Scan { found, status }
}

/// La Bibliothèque, jeu prêt. Les dossiers de Raftup sont dans un dossier temporaire.
pub fn bibliotheque() -> Harness<'static, Bibliotheque> {
    let root = tempfile::tempdir().unwrap();
    let state = Bibliotheque {
        library: LibraryState::for_tests(demo_catalog(), Paths::under(root.path()), scan_ready()),
        shared: Shared::default(),
        _root: root,
    };
    let mut harness = builder().build_ui_state(
        |ui, s: &mut Bibliotheque| {
            library::show(ui, &mut s.library, &s.shared, None);
        },
        state,
    );
    harness.run();
    harness
}

/// Vrai si le champ « Adresse » du formulaire de connexion est à l'écran.
fn address_field_shown(harness: &Harness<'_, Bibliotheque>) -> bool {
    harness
        .query_by(|node| node.data().placeholder() == Some(ADDRESS_HINT))
        .is_some()
}

#[test]
fn la_carte_et_la_fiche_du_jeu_s_affichent() {
    let harness = bibliotheque();
    assert!(shows(&harness, "Bibliothèque"));
    assert!(shows(&harness, "1 jeu · 1 prêt"));
    // Le nom du jeu : sous la jaquette de la carte, et en titre de la fiche.
    assert!(count(&harness, "Slime Rancher") >= 2);
    // La fiche : origine du profil, bouton principal, emplacement, étapes.
    assert!(shows(&harness, "Profil du catalogue de Raftup"));
    assert!(shows(&harness, "Lancer le jeu"));
    assert!(shows(&harness, "Détecté via Steam"));
    assert!(shows(&harness, "MelonLoader"));
    assert!(shows(&harness, "Désinstaller le mod"));
}

#[test]
fn se_connecter_a_la_main_est_replie() {
    let mut harness = bibliotheque();
    assert!(shows(&harness, "CONNEXION À UNE PARTIE"));
    assert!(shows(&harness, "Se connecter à la main"));
    // Replié : le formulaire n'est pas dessiné.
    assert!(!address_field_shown(&harness));

    // Un clic sur l'en-tête l'ouvre.
    click(&mut harness, "Se connecter à la main");
    assert!(address_field_shown(&harness));
}
