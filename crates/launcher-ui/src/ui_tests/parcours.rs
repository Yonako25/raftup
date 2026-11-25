//! Parcours du joueur (plan de l'interface, étapes 3 à 5) : Accueil, Catalogue,
//! Profil, assistant du premier lancement, « Lancer et connecter » de la partie.

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use launcher_core::{
    DetectedGame, GameState, HostStatus, InstallStatus, Paths, SeedSummary, ServerStatus,
    SlotStatus, Store,
};

use super::{builder, click, demo_catalog, shows, shows_part, type_into};
use crate::app::Page;
use crate::screens::friends::{self, FriendsState};
use crate::screens::home;
use crate::screens::join::JoinState;
use crate::screens::library::{self, LibraryState, Scan};
use crate::screens::running::{self, RunningState};
use crate::screens::welcome::{self, WelcomeState};
use crate::worker::{Shared, Worker};

const GAME: &str = "slime-rancher";

/// Une détection : Slime Rancher trouvé via Steam, mods `installed` ou non.
fn scan(installed: bool) -> Scan {
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
    let receipt = if installed {
        InstallStatus::Installed
    } else {
        InstallStatus::NotInstalled
    };
    status.insert(GAME.to_string(), Ok(receipt));
    Scan { found, status }
}

/// La Bibliothèque des tests ; le dossier temporaire vit dans l'état de l'écran.
fn library(scan: Scan) -> (LibraryState, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let state = LibraryState::for_tests(demo_catalog(), Paths::under(root.path()), scan);
    (state, root)
}

/// Une partie hébergée à deux joueurs, sans mot de passe.
pub(super) fn hosted(has_password: bool) -> HostStatus {
    let slot = |n: u32, name: &str, game: &str, checked: usize| SlotStatus {
        team: 0,
        slot: n,
        name: name.to_string(),
        game: game.to_string(),
        is_group: false,
        connections: 0,
        tags: Vec::new(),
        checked,
        total: 40,
        status: 0,
        last_activity: None,
        away: None,
        release_votes: 0,
    };
    HostStatus {
        seed: SeedSummary {
            seed_name: "12345".to_string(),
            generator_version: "0.6.8".to_string(),
            slots: Vec::new(),
            players: 2,
            has_password,
            release_mode: "auto".to_string(),
            collect_mode: "auto".to_string(),
            hint_cost: 10,
        },
        address: "wss://exemple.fr:38281".to_string(),
        encrypted: true,
        fingerprint: None,
        has_password,
        started_at: crate::format::now() - 60.0,
        save: None,
        resumed: false,
        server: ServerStatus {
            slots: vec![
                slot(1, "Ioka", "Slime Rancher", 10),
                slot(2, "Mara", "A Short Hike", 0),
            ],
            connections: 0,
        },
    }
}

// ---------------------------------------------------------------------------
// Accueil
// ---------------------------------------------------------------------------

struct Accueil {
    shared: Shared,
    library: LibraryState,
    join: JoinState,
    asked: Option<Page>,
    _root: tempfile::TempDir,
}

fn accueil(scan: Scan, shared: Shared, join: JoinState) -> Harness<'static, Accueil> {
    let (library, root) = library(scan);
    let state = Accueil {
        shared,
        library,
        join,
        asked: None,
        _root: root,
    };
    let mut harness = builder().build_ui_state(
        |ui, s: &mut Accueil| {
            if let Some(page) = home::show(ui, &s.shared, &mut s.library, &s.join) {
                s.asked = Some(page);
            }
        },
        state,
    );
    harness.run();
    harness
}

#[test]
fn accueil_jeu_pret_propose_un_salon() {
    let mut harness = accueil(
        scan(true),
        Shared::default(),
        JoinState::without_clipboard(),
    );
    // Vitrine : le prochain pas par-dessus l'image du jeu mis en avant.
    assert!(shows(&harness, "PROCHAIN PAS"));
    assert!(shows(&harness, "Slime Rancher est prêt"));
    // Plus de panneau « Parties en cours » sur l'Accueil.
    assert!(!shows(&harness, "Parties en cours"));
    assert!(shows(&harness, "Tes jeux"));
    click(&mut harness, "Nouvelle partie");
    assert_eq!(harness.state().asked, Some(Page::Create));

    // La carte de Slime Rancher sélectionne le jeu et ouvre la Bibliothèque.
    let mut harness = accueil(
        scan(true),
        Shared::default(),
        JoinState::without_clipboard(),
    );
    click(&mut harness, "Fiche de Slime Rancher");
    assert_eq!(harness.state().asked, Some(Page::Library));
    let ready: Vec<_> = harness.state().library.summaries();
    assert!(ready[0].ready);
}

#[test]
fn accueil_sans_jeu_pret_mene_a_la_bibliotheque() {
    let mut harness = accueil(
        scan(false),
        Shared::default(),
        JoinState::without_clipboard(),
    );
    assert!(shows(&harness, "Prépare ton premier jeu"));
    // Le jeu est sur ce PC (mods pas installés) : il est dans « Tes jeux ».
    assert!(shows(&harness, "Fiche de Slime Rancher"));
    click(&mut harness, "Ouvrir le Catalogue");
    assert_eq!(harness.state().asked, Some(Page::Library));
}

#[test]
fn accueil_partie_hebergee_et_invitation() {
    let shared = Shared {
        status: Some(hosted(false)),
        ..Shared::default()
    };
    let join = JoinState::with_invite(
        "raftup://join?v=1&addr=wss%3A%2F%2Fami.fr%3A38281&slot=Ioka&game=Slime%20Rancher",
    );
    let mut harness = accueil(scan(true), shared, join);
    // La partie passe avant l'invitation.
    // La vitrine nomme la partie et dit où elle en est.
    assert!(shows(&harness, "Partie Slime Rancher et A Short Hike"));
    assert!(shows_part(
        &harness,
        "Une partie tourne sur ce PC · 10 / 80 lieux"
    ));
    assert!(shows(&harness, "Invitation reçue"));
    assert!(shows(&harness, "Une partie de Slime Rancher t'attend."));
    click(&mut harness, "Ouvrir la partie");
    assert_eq!(harness.state().asked, Some(Page::Hosted));
}

// ---------------------------------------------------------------------------
// Catalogue (l'ancienne Bibliothèque et l'ancien Catalogue réunis, revue point 25)
// ---------------------------------------------------------------------------

struct Catalogue {
    library: LibraryState,
    shared: Shared,
    _root: tempfile::TempDir,
}

fn catalogue_with(library: LibraryState, root: tempfile::TempDir) -> Harness<'static, Catalogue> {
    let state = Catalogue {
        library,
        shared: Shared::default(),
        _root: root,
    };
    let mut harness = builder().build_ui_state(
        |ui, s: &mut Catalogue| {
            library::show(ui, &mut s.library, &s.shared, None);
        },
        state,
    );
    harness.run();
    harness
}

fn catalogue(scan: Scan) -> Harness<'static, Catalogue> {
    let (library, root) = library(scan);
    catalogue_with(library, root)
}

/// Le vrai catalogue (tous les profils), avec une détection donnée.
fn full_catalogue(scan: Scan) -> Harness<'static, Catalogue> {
    let root = tempfile::tempdir().unwrap();
    let catalog = launcher_core::Catalog::embedded().unwrap();
    let library = LibraryState::for_tests(catalog, Paths::under(root.path()), scan);
    catalogue_with(library, root)
}

#[test]
fn catalogue_cherche_un_jeu() {
    let mut harness = catalogue(scan(false));
    assert!(shows(&harness, "Catalogue"));
    assert!(shows_part(&harness, "1 jeu · 0 prêt à jouer"));
    assert!(shows(&harness, "Fiche de Slime Rancher"));
    // Sans hub, la jaquette est l'aplat avec le titre : pas d'image.
    assert!(!shows(&harness, "Jaquette de Slime Rancher"));
    type_into(&mut harness, "Chercher un jeu ou un auteur", "hollow");
    assert!(!shows(&harness, "Fiche de Slime Rancher"));
    // Par l'auteur des mods aussi.
    let mut harness = catalogue(scan(false));
    type_into(&mut harness, "Chercher un jeu ou un auteur", "creeper");
    assert!(shows(&harness, "Fiche de Slime Rancher"));
}

/// Jeu trouvé sans le mod : « Installer le mod » sous sa jaquette (et dans sa fiche).
/// Jeu prêt : pas de bouton sous la jaquette.
#[test]
fn catalogue_bouton_installer_le_mod_sous_la_jaquette() {
    let harness = catalogue(scan(false));
    assert!(shows(&harness, "Mod non installé"));
    assert!(super::count(&harness, "Installer le mod") >= 1);
    let harness = catalogue(scan(true));
    assert!(shows(&harness, "Prêt"));
    assert!(!shows(&harness, "Installer le mod"));
}

/// Ordre de la grille : prêts à jouer, puis sur ce PC sans le mod, puis absents.
#[test]
fn catalogue_jeux_prets_en_premier() {
    // Un jeu prêt (Slime Rancher) et un jeu trouvé sans le mod (Hollow Knight) ; les
    // autres ne sont pas sur ce PC.
    let mut detected = scan(true);
    detected.found.insert(
        "hollow-knight".to_string(),
        DetectedGame {
            game_id: "hollow-knight".to_string(),
            store: Store::Steam,
            install_dir: PathBuf::from("C:/Jeux/Hollow Knight"),
            build_id: None,
            state: GameState::Ready,
        },
    );
    detected
        .status
        .insert("hollow-knight".to_string(), Ok(InstallStatus::NotInstalled));
    let root = tempfile::tempdir().unwrap();
    let catalog = launcher_core::Catalog::embedded().unwrap();
    let library = LibraryState::for_tests(catalog, Paths::under(root.path()), detected);
    let order = library.catalog_order();
    assert_eq!(order[0], "slime-rancher");
    assert_eq!(order[1], "hollow-knight");
    assert!(order.len() > 2);
    // Le premier jeu (prêt) est sélectionné : sa fiche est ouverte.
    let harness = catalogue_with(library, root);
    assert!(shows(&harness, "Lancer le jeu"));
}

/// Bug signalé le 22 novembre 2026 : avec le vrai catalogue, toutes les cartes tenaient
/// sur une rangée qui sortait de l'écran. Le dernier jeu doit être dans la largeur.
#[test]
fn catalogue_complet_tient_dans_la_largeur() {
    let harness = full_catalogue(Scan::default());
    let order = harness.state().library.catalog_order();
    let catalog = harness.state().library.catalog().unwrap();
    let last = catalog
        .get(order.last().unwrap())
        .unwrap()
        .game
        .name
        .clone();
    let width = harness.ctx.content_rect().width();
    let label = format!("Fiche de {last}");
    let card = harness.get_by_label(&label);
    assert!(
        card.rect().right() <= width,
        "le dernier jeu sort de l'écran : {:?} (largeur {width})",
        card.rect()
    );
}

/// Un jeu pas sur ce PC : sa fiche dit comment indiquer son dossier, montre ce que Raftup
/// installerait, et ne propose ni « Lancer » ni « Installer le mod ».
#[test]
fn catalogue_fiche_d_un_jeu_absent() {
    let mut harness = full_catalogue(Scan::default());
    click(&mut harness, "Fiche de Hollow Knight");
    assert!(shows(&harness, "Pas sur ce PC"));
    assert!(shows(&harness, "Indiquer le dossier du jeu"));
    assert!(shows_part(&harness, "Profil non testé"));
    assert!(!shows(&harness, "Lancer le jeu"));
    assert!(!shows(&harness, "Installer le mod"));
    click(&mut harness, "Ce que Raftup installe");
    assert!(shows_part(&harness, "Hollow Knight Modding API"));
    assert!(shows_part(&harness, "Auteurs des mods"));
}

// ---------------------------------------------------------------------------
// Profil
// ---------------------------------------------------------------------------

#[test]
fn profil_sans_compte_propose_de_se_connecter() {
    struct Profil {
        friends: FriendsState,
        asked: Option<Page>,
    }
    let state = Profil {
        friends: FriendsState::for_tests(),
        asked: None,
    };
    let mut harness = builder().build_ui_state(
        |ui, s: &mut Profil| {
            if let Some(page) = friends::show_profile(ui, &mut s.friends) {
                s.asked = Some(page);
            }
        },
        state,
    );
    harness.run();
    assert!(shows(&harness, "Mon profil"));
    assert!(shows(&harness, "Sans compte"));
    click(&mut harness, "Se connecter ou créer un compte");
    assert_eq!(harness.state().asked, Some(Page::Friends));
}

// ---------------------------------------------------------------------------
// Assistant du premier lancement
// ---------------------------------------------------------------------------

struct Bienvenue {
    welcome: WelcomeState,
    library: LibraryState,
    outcome: welcome::Outcome,
    _root: tempfile::TempDir,
}

fn bienvenue(scan: Scan) -> Harness<'static, Bienvenue> {
    let (library, root) = library(scan);
    let state = Bienvenue {
        welcome: WelcomeState::opened(),
        library,
        outcome: welcome::Outcome::default(),
        _root: root,
    };
    let mut harness = builder().build_ui_state(
        |ui, s: &mut Bienvenue| {
            let games = s.library.summaries();
            let outcome = welcome::show(ui.ctx(), &mut s.welcome, None, &games);
            if outcome != welcome::Outcome::default() {
                s.outcome = outcome;
            }
        },
        state,
    );
    harness.run();
    harness
}

#[test]
fn assistant_en_trois_pas_jusqu_au_salon() {
    let mut harness = bienvenue(scan(true));
    assert!(shows(&harness, "BIENVENUE · 1 / 3"));
    assert!(shows(&harness, "Ton compte Rafthub"));
    click(&mut harness, "Continuer sans compte");
    assert!(shows(&harness, "Tes jeux"));
    assert!(shows(&harness, "Slime Rancher"));
    click(&mut harness, "Suivant");
    assert!(shows(&harness, "Ton premier jeu"));
    assert!(shows_part(&harness, "Slime Rancher est prêt"));
    click(&mut harness, "Créer un salon");
    assert!(!harness.state().welcome.is_open());
    assert_eq!(harness.state().outcome.page, Some(Page::Create));
    assert_eq!(harness.state().outcome.select.as_deref(), Some(GAME));
}

#[test]
fn assistant_passe_se_ferme() {
    let mut harness = bienvenue(scan(false));
    click(&mut harness, "Passer l'assistant");
    assert!(!harness.state().welcome.is_open());
    assert!(!shows(&harness, "Ton compte Rafthub"));
    assert_eq!(harness.state().outcome, welcome::Outcome::default());
}

// ---------------------------------------------------------------------------
// Partie en cours : « Lancer et connecter »
// ---------------------------------------------------------------------------

struct Partie {
    running: RunningState,
    error: Option<String>,
    worker: Worker,
    shared: Shared,
    library: LibraryState,
    _root: tempfile::TempDir,
}

fn partie(status: HostStatus) -> Harness<'static, Partie> {
    let (library, root) = library(scan(true));
    let state = Partie {
        library,
        _root: root,
        running: RunningState::default(),
        error: None,
        worker: Worker::detached(),
        shared: Shared {
            status: Some(status),
            ..Shared::default()
        },
    };
    let mut harness = builder().build_ui_state(
        |ui, s: &mut Partie| {
            running::show(
                ui,
                &mut s.running,
                &mut s.error,
                &s.worker,
                &s.shared,
                &mut s.library,
            );
        },
        state,
    );
    harness.run();
    harness
}

#[test]
fn lancer_et_connecter_apres_le_choix_du_slot() {
    let mut harness = partie(hosted(true));
    assert!(shows(&harness, "Je joue :"));
    assert!(shows_part(&harness, "Choisis ton joueur"));
    // Deux joueurs : pas de choix d'office, le bouton ne fait rien.
    click(&mut harness, "Lancer et connecter");
    assert_eq!(
        running::take_launch_request(&mut harness.state_mut().running),
        None
    );

    // La liste déroulante n'a pas de libellé : on la trouve par le texte affiché.
    harness
        .get_by(|node| node.data().value() == Some("choisis ton joueur"))
        .click();
    harness.run();
    click(&mut harness, "Ioka · Slime Rancher");
    assert!(shows(&harness, "10 / 40 lieux"));
    click(&mut harness, "Lancer et connecter");
    let request = running::take_launch_request(&mut harness.state_mut().running).unwrap();
    assert_eq!(request.ap_game, "Slime Rancher");
    assert_eq!(request.slot, "Ioka");
    assert_eq!(request.address, "wss://exemple.fr:38281");
    assert!(request.needs_password);
}

#[test]
fn lancer_et_connecter_prepare_la_fiche() {
    use crate::screens::library::{launch_into, LaunchInto};
    let ctx = egui::Context::default();
    // Mot de passe demandé : la fiche est prête, le joueur le tape.
    let (mut state, _root) = library(scan(true));
    let outcome = launch_into(&mut state, &ctx, "Slime Rancher", "wss://h:1", "Ioka", true);
    assert_eq!(outcome, LaunchInto::NeedsPassword);
    // Mods pas installés : la fiche s'ouvre pour l'installation.
    let (mut state, _root) = library(scan(false));
    let outcome = launch_into(
        &mut state,
        &ctx,
        "Slime Rancher",
        "wss://h:1",
        "Ioka",
        false,
    );
    assert_eq!(outcome, LaunchInto::NotReady);
    // Jeu hors catalogue : rien à lancer d'ici.
    let outcome = launch_into(&mut state, &ctx, "A Short Hike", "wss://h:1", "Mara", false);
    assert_eq!(outcome, LaunchInto::OutsideCatalog);
}
