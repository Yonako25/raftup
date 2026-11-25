//! Écran « Bibliothèque » avec le catalogue de démonstration (`profiles/`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use launcher_core::art::ArtKind;
use launcher_core::{DetectedGame, GameState, InstallStatus, Paths, Store};

use super::{builder, click, count, demo_catalog, shows, shows_part};
use crate::art_ui::{Artwork, Loader};
use crate::screens::library::{self, LibraryState, Scan};
use crate::worker::Shared;

/// Identifiant du jeu du catalogue de démonstration.
const GAME: &str = "slime-rancher";
/// Texte d'aide du champ « Adresse » de « Se connecter à la main ».
const ADDRESS_HINT: &str = "adresse:38281";

pub struct Bibliotheque {
    pub library: LibraryState,
    pub shared: Shared,
    /// Le dossier temporaire doit vivre aussi longtemps que l'écran : il est effacé
    /// quand cette valeur est détruite (`Drop`), donc à la fin du test.
    _root: tempfile::TempDir,
}

/// Une détection « faite » : le jeu trouvé via Steam, ses mods installés.
pub(super) fn scan_ready() -> Scan {
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
/// Sans hub : aucune illustration, les aplats.
pub fn bibliotheque() -> Harness<'static, Bibliotheque> {
    bibliotheque_with(None)
}

/// Comme [`bibliotheque`], avec des illustrations données par `art` (faux hub).
pub(super) fn bibliotheque_with(art: Option<Artwork>) -> Harness<'static, Bibliotheque> {
    let root = tempfile::tempdir().unwrap();
    let mut library =
        LibraryState::for_tests(demo_catalog(), Paths::under(root.path()), scan_ready());
    if let Some(art) = art {
        library.art = art;
    }
    let state = Bibliotheque {
        library,
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
    let mut harness = bibliotheque();
    assert!(shows(&harness, "Catalogue"));
    assert!(shows(&harness, "1 jeu · 1 prêt à jouer"));
    // Le nom du jeu : sous la jaquette de la carte, et en titre de la fiche.
    assert!(count(&harness, "Slime Rancher") >= 2);
    // La fiche : origine du profil, bouton principal, emplacement, étapes.
    // Le cas normal (catalogue de Raftup) ne dit pas son origine.
    assert!(!shows(&harness, "Profil du catalogue de Raftup"));
    assert!(shows(&harness, "Lancer le jeu"));
    assert!(shows(&harness, "Détecté via Steam"));
    // La sorte de monde : badge sur la jaquette, badge et phrase dans la fiche.
    assert!(count(&harness, "Classique") >= 1);
    assert!(shows_part(
        &harness,
        "Monde classique : la partie est préparée"
    ));
    // Aucun nom de projet tiers à l'écran (revue d'ergonomie, point 23).
    assert!(!shows_part(&harness, "Archipelago"));
    // « Réinstaller » d'un jeu prêt : petit bouton à côté de « Désinstaller ».
    assert!(shows(&harness, "Réinstaller"));
    assert!(shows(&harness, "Désinstaller le mod"));
    // Les étapes du profil sont repliées : un clic les montre.
    assert!(!shows(&harness, "MelonLoader"));
    click(&mut harness, "Ce que Raftup installe");
    assert!(shows(&harness, "MelonLoader"));
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

// ---------------------------------------------------------------------------
// Illustrations des jeux (ADR 0051)
// ---------------------------------------------------------------------------

const COVER_LABEL: &str = "Jaquette de Slime Rancher";
const HERO_LABEL: &str = "Illustration de Slime Rancher";
/// Bandeau en aplat en haut de la fiche, quand il n'y a pas d'image large.
const FLAT_HERO_LABEL: &str = "Aplat de Slime Rancher";

/// Un vrai petit JPEG (4 × 6 pixels), pour passer par le décodage de l'interface.
fn tiny_jpeg() -> Vec<u8> {
    let image = image::RgbImage::from_pixel(4, 6, image::Rgb([200, 80, 40]));
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .encode_image(&image)
        .unwrap();
    bytes
}

/// Laisse tourner l'écran jusqu'à ce que `done` soit vrai (le chargement se fait
/// dans un thread à part), 5 s au plus.
fn run_until(
    harness: &mut Harness<'_, Bibliotheque>,
    done: impl Fn(&Harness<'_, Bibliotheque>) -> bool,
) -> bool {
    let start = Instant::now();
    loop {
        harness.run();
        if done(harness) {
            return true;
        }
        if start.elapsed() > Duration::from_secs(5) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn sans_image_la_jaquette_est_dessinee() {
    let harness = bibliotheque();
    assert!(!shows(&harness, COVER_LABEL));
    assert!(!shows(&harness, HERO_LABEL));
    // La fiche garde son bandeau, en aplat coloré.
    assert!(shows(&harness, FLAT_HERO_LABEL));
    // Le titre reste écrit dans l'aplat et sous la carte.
    assert!(count(&harness, "Slime Rancher") >= 2);
}

#[test]
fn jaquette_et_illustration_lues_sur_le_disque() {
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&asked);
    let jpeg = tiny_jpeg();
    // Fausse source : une image pour Slime Rancher seulement (créateur ou Steam).
    let loader: Loader = Arc::new(move |game_id: &str, _kind: ArtKind| {
        if game_id != GAME {
            return None;
        }
        counter.fetch_add(1, Ordering::SeqCst);
        Some(jpeg.clone())
    });
    let mut harness = bibliotheque_with(Some(Artwork::with_loader(loader)));
    assert!(run_until(&mut harness, |h| shows(h, COVER_LABEL)
        && shows(h, HERO_LABEL)));
    // L'image large a pris la place de l'aplat.
    assert!(!shows(&harness, FLAT_HERO_LABEL));
    // Une lecture par image (jaquette, image large), pas une par image de l'écran.
    harness.run();
    assert_eq!(asked.load(Ordering::SeqCst), 2);
    // La fiche reste complète.
    assert!(shows(&harness, "Lancer le jeu"));
}

#[test]
fn image_absente_ou_illisible_jaquette_dessinee() {
    for answer in [None, Some(b"pas une image".to_vec())] {
        let asked = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&asked);
        let loader: Loader = Arc::new(move |game_id: &str, _kind: ArtKind| {
            if game_id == GAME {
                counter.fetch_add(1, Ordering::SeqCst);
            }
            answer.clone()
        });
        let mut harness = bibliotheque_with(Some(Artwork::with_loader(loader)));
        // Les deux demandes ont reçu leur réponse.
        assert!(run_until(&mut harness, |_| asked.load(Ordering::SeqCst) == 2));
        for _ in 0..5 {
            harness.run();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!shows(&harness, COVER_LABEL));
        assert!(!shows(&harness, HERO_LABEL));
        assert!(shows(&harness, FLAT_HERO_LABEL));
        assert!(count(&harness, "Slime Rancher") >= 2);
        // Pas redemandé à chaque image.
        assert_eq!(asked.load(Ordering::SeqCst), 2);
    }
}

/// Bug vu sur les captures (22 novembre 2026) : avec plusieurs jeux, les cartes
/// restaient sur une seule rangée qui débordait à droite et poussait la fiche hors de
/// l'écran. Avec tout le catalogue embarqué, chaque carte et la fiche tiennent dans la
/// largeur de l'écran.
#[test]
fn plusieurs_jeux_la_grille_passe_a_la_ligne() {
    let root = tempfile::tempdir().unwrap();
    let catalog = launcher_core::Catalog::embedded().unwrap();
    let names: Vec<String> = catalog.games().map(|p| p.game.name.clone()).collect();
    assert!(names.len() >= 4, "le test a besoin de plusieurs jeux");
    let library = LibraryState::for_tests(catalog, Paths::under(root.path()), scan_ready());
    let state = Bibliotheque {
        library,
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
    let right_edge = super::CENTRAL.x;
    for name in &names {
        for node in harness.query_all_by_label(name) {
            assert!(
                node.rect().right() <= right_edge,
                "« {name} » dépasse le bord droit : {:?}",
                node.rect()
            );
        }
    }
    // La fiche du jeu choisi est à l'écran.
    assert!(shows(&harness, "Ce que Raftup installe"));
    let profile = harness.get_by_label("Ce que Raftup installe").rect();
    assert!(
        profile.right() <= right_edge,
        "fiche hors de l'écran : {profile:?}"
    );
}
