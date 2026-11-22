//! Captures de tous les écrans du launcher, pour les **regarder** sans ouvrir de fenêtre.
//!
//! Commande unique (alias de `.cargo/config.toml`) :
//!
//! ```text
//! cargo captures
//! ```
//!
//! Elle écrit un PNG par écran dans `target/captures/` (ou dans `RAFTUP_CAPTURES_DIR`).
//! La CI Linux fait la même chose à chaque pull request et range les images dans
//! l'artefact `captures-interface`.
//!
//! Rendu : egui_kittest et wgpu. Sans carte graphique (CI, conteneur), il faut un pilote
//! Vulkan logiciel : Mesa lavapipe (`apt-get install mesa-vulkan-drivers`).
//!
//! Ces images servent à regarder, pas à comparer : les polices sont rendues un peu
//! autrement sous Linux. Les captures de référence vérifiées pixel par pixel restent
//! celles de `captures.rs`, sous Windows.
//!
//! Données : uniquement de démonstration (catalogue de `profiles/`, faux hub qui fabrique
//! ses images, fausse partie). Aucun réseau, aucun port, aucun fichier du joueur.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use launcher_core::art::ArtKind;
use launcher_core::{GameEvent, HostStatus, NumberedEvent, SeedSummary, ServerStatus, SlotStatus};

use super::bibliotheque::{bibliotheque, bibliotheque_with};
use super::menu::menu;
use super::salon::salon;
use super::{builder, click, demo_catalog, shows, type_into};
use crate::art_ui::{Artwork, Loader};
use crate::screens::friends::{self, FriendsState};
use crate::screens::join::{self, JoinState};
use crate::screens::library::LibraryState;
use crate::screens::{catalog, home, running, settings, setup, welcome};
use crate::worker::{Shared, Worker};

/// Dossier des images : `RAFTUP_CAPTURES_DIR`, sinon `target/captures/` du workspace
/// (ou de `CARGO_TARGET_DIR`, s'il est défini).
fn output_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RAFTUP_CAPTURES_DIR") {
        return PathBuf::from(dir);
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    target.join("captures")
}

/// Hauteur des captures d'écran entier : plus haute que la fenêtre de départ, pour voir
/// les pages longues (fiche du jeu, salon) sans les couper.
const PAGE_HEIGHT: f32 = 1300.0;

/// Agrandit le harnais à la hauteur d'une page entière (même largeur qu'au départ).
fn tall<State>(harness: &mut Harness<'_, State>) {
    harness.set_size(egui::vec2(super::CENTRAL.x, PAGE_HEIGHT));
    harness.run();
}

/// Rend l'écran du harnais en PNG sous le nom `<nom>.png`.
fn save<State>(harness: &mut Harness<'_, State>, name: &str) {
    // La souris sort de la fenêtre : ni pointeur ni survol sur l'image.
    harness.event(egui::Event::PointerGone);
    harness.run();
    let dir = output_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let image = harness
        .render()
        .unwrap_or_else(|e| panic!("rendu wgpu impossible ({e}) : installer mesa-vulkan-drivers"));
    let path = dir.join(format!("{name}.png"));
    image.save(&path).unwrap();
    println!("capture : {}", path.display());
}

/// Comme [`save`], mais la souris reste où elle est (capture d'un survol).
fn save_keep_pointer<State>(harness: &mut Harness<'_, State>, name: &str) {
    let dir = output_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let image = harness
        .render()
        .unwrap_or_else(|e| panic!("rendu wgpu impossible ({e}) : installer mesa-vulkan-drivers"));
    let path = dir.join(format!("{name}.png"));
    image.save(&path).unwrap();
    println!("capture : {}", path.display());
}

/// Laisse tourner l'écran jusqu'à ce que `done` soit vrai (5 s au plus) : les images du
/// faux hub arrivent par un thread, comme dans le vrai launcher.
fn settle<State>(harness: &mut Harness<'_, State>, done: impl Fn(&Harness<'_, State>) -> bool) {
    let start = Instant::now();
    while !done(harness) && start.elapsed() < Duration::from_secs(5) {
        harness.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    harness.run();
}

/// Une image JPEG de démonstration : un dégradé de la couleur donnée, avec des bandes
/// diagonales. Fabriquée ici : aucune illustration réelle d'un jeu n'est utilisée.
fn demo_jpeg(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        let shade = 0.45 + 0.55 * (y as f32 / height as f32);
        let stripe = if (x + y) / 40 % 2 == 0 { 1.0 } else { 0.85 };
        let [r, g, b] = color.map(|c| (c as f32 * shade * stripe) as u8);
        image::Rgb([r, g, b])
    });
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .encode_image(&image)
        .unwrap();
    bytes
}

/// Fausse source : une jaquette (528 × 748) et une image large (1280 × 720) de
/// démonstration, pour tout jeu demandé.
fn demo_art() -> Artwork {
    let cover = demo_jpeg(528, 748, [76, 154, 255]);
    let hero = demo_jpeg(1280, 720, [167, 139, 250]);
    let loader: Loader = Arc::new(move |_game_id: &str, kind: ArtKind| {
        Some(match kind {
            ArtKind::Cover => cover.clone(),
            ArtKind::Hero => hero.clone(),
        })
    });
    Artwork::with_loader(loader)
}

/// Une fausse partie hébergée : trois joueurs à des stades différents, et un fil
/// d'activité.
fn demo_party() -> Shared {
    let now = crate::format::now();
    let slot =
        |n: u32, name: &str, game: &str, checked: usize, status: u32, connections| SlotStatus {
            team: 0,
            slot: n,
            name: name.to_string(),
            game: game.to_string(),
            is_group: false,
            connections,
            tags: Vec::new(),
            checked,
            total: 120,
            status,
            last_activity: Some(now - 60.0 * f64::from(n)),
            away: None,
            release_votes: 0,
        };
    let events = [
        GameEvent::Joined {
            time: now - 900.0,
            slot: 1,
            name: "Alice".into(),
            game: "Slime Rancher".into(),
            tags: Vec::new(),
        },
        GameEvent::ItemSent {
            time: now - 600.0,
            finder: "Bob".into(),
            receiver: "Alice".into(),
            item: "Jetpack".into(),
            location: "Ranch : coffre 3".into(),
            flags: 1,
        },
        GameEvent::Chat {
            time: now - 300.0,
            slot: 2,
            name: "Bob".into(),
            text: "Il me manque le grappin !".into(),
        },
        GameEvent::Goal {
            time: now - 60.0,
            slot: 3,
            name: "Chloé".into(),
        },
    ];
    Shared {
        status: Some(HostStatus {
            seed: SeedSummary {
                seed_name: "71849263510274".to_string(),
                generator_version: "0.6.8".to_string(),
                slots: Vec::new(),
                players: 3,
                has_password: true,
                release_mode: "auto".to_string(),
                collect_mode: "auto".to_string(),
                hint_cost: 10,
            },
            address: "wss://partie.exemple.fr:38281".to_string(),
            encrypted: true,
            // Certificat généré par Raftup (ADR 0043) : port mixte, empreinte dans les liens.
            fingerprint: Some(launcher_core::Fingerprint::of_der(b"demonstration").to_hex()),
            has_password: true,
            started_at: now - 3600.0,
            save: None,
            resumed: false,
            server: ServerStatus {
                slots: vec![
                    slot(1, "Alice", "Slime Rancher", 54, 20, 1),
                    slot(2, "Bob", "Slime Rancher", 31, 20, 1),
                    slot(3, "Chloé", "Slime Rancher", 120, 30, 0),
                ],
                connections: 2,
            },
        }),
        events: events
            .into_iter()
            .enumerate()
            .map(|(number, event)| NumberedEvent {
                number: number as u64,
                event,
            })
            .collect(),
        ..Shared::default()
    }
}

/// État de l'écran « Partie en cours ».
struct Partie {
    running: running::RunningState,
    error: Option<String>,
    worker: Worker,
    shared: Shared,
    library: LibraryState,
    _root: tempfile::TempDir,
}

/// État de l'écran « Héberger » (sans partie en cours).
struct Heberger {
    form: setup::SetupForm,
    error: Option<String>,
    worker: Worker,
    shared: Shared,
}

/// État de l'écran « Rejoindre ».
struct Rejoindre {
    join: JoinState,
    library: LibraryState,
    _root: tempfile::TempDir,
}

/// Tous les écrans, l'un après l'autre : deux rendus wgpu en même temps font planter
/// certains pilotes (voir `captures.rs`). Chaque harnais est détruit avant le suivant.
#[test]
#[ignore = "rendu wgpu : lancer `cargo captures` (pilote Vulkan logiciel sous Linux)"]
fn captures_interface() {
    // Menu (barre latérale seule).
    {
        let mut harness = menu();
        save(&mut harness, "01-menu");
    }
    // Bibliothèque sans hub : aplats colorés.
    {
        let mut harness = bibliotheque();
        save(&mut harness, "02-bibliotheque-sans-hub");
    }
    // Bibliothèque avec le faux hub : jaquettes et image large de la fiche.
    {
        let mut harness = bibliotheque_with(Some(demo_art()));
        settle(&mut harness, |h| {
            shows(h, "Jaquette de Slime Rancher") && shows(h, "Illustration de Slime Rancher")
        });
        save(&mut harness, "03-bibliotheque-illustrations");
    }
    // Bibliothèque : jaquette survolée (soulevée, liseré bleu), et un message éphémère.
    {
        let mut harness = bibliotheque_with(Some(demo_art()));
        settle(&mut harness, |h| shows(h, "Jaquette de Slime Rancher"));
        harness.get_by_label("Fiche de Slime Rancher").hover();
        // L'animation dure 0,15 s : quelques images pour qu'elle aille au bout.
        for _ in 0..30 {
            harness.step();
        }
        save_keep_pointer(&mut harness, "03b-survol-jaquette");
    }
    {
        let mut toasts = crate::toasts::Toasts::default();
        toasts.push_aged(
            "Slime Rancher installé.",
            crate::toasts::ToastKind::Success,
            1.0,
        );
        toasts.push_aged(
            "Téléchargement impossible : le serveur de l'auteur ne répond pas.",
            crate::toasts::ToastKind::Error,
            1.0,
        );
        let mut harness = builder().build_ui_state(
            |ui, toasts: &mut crate::toasts::Toasts| {
                toasts.show(ui.ctx());
            },
            toasts,
        );
        // Les messages redessinent sans arrêt (fondus) : quelques images, sans `run`.
        harness.run_steps(3);
        save_keep_pointer(&mut harness, "03c-messages-ephemeres");
    }
    // Fiche d'un jeu, avec « Se connecter à la main » ouvert.
    {
        let mut harness = bibliotheque_with(Some(demo_art()));
        settle(&mut harness, |h| {
            shows(h, "Jaquette de Slime Rancher") && shows(h, "Illustration de Slime Rancher")
        });
        tall(&mut harness);
        click(&mut harness, "Se connecter à la main");
        save(&mut harness, "04-fiche-du-jeu");
    }
    // Salon de préparation, équipage rempli.
    {
        let mut harness = salon();
        type_into(
            &mut harness,
            "Ton nom dans la partie (16 caractères au plus)",
            "Alice",
        );
        type_into(&mut harness, "Pseudo", "Bob");
        click(&mut harness, "Ajouter");
        tall(&mut harness);
        save(&mut harness, "05-salon");
    }
    // Rejoindre : un lien d'invitation reçu.
    {
        let root = tempfile::tempdir().unwrap();
        let mut join_state = JoinState::without_clipboard();
        join::receive(
            &mut join_state,
            "raftup://join?v=1&addr=wss://partie.exemple.fr:38281&slot=Bob&game=Slime%20Rancher",
        );
        let state = Rejoindre {
            join: join_state,
            library: LibraryState::for_tests(
                demo_catalog(),
                launcher_core::Paths::under(root.path()),
                super::bibliotheque::scan_ready(),
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
        save(&mut harness, "06-rejoindre");
    }
    // Héberger (aucune partie en cours).
    {
        let state = Heberger {
            form: setup::SetupForm::with_webhook(String::new()),
            error: None,
            worker: Worker::detached(),
            shared: Shared::default(),
        };
        let mut harness = builder().build_ui_state(
            |ui, s: &mut Heberger| {
                setup::show(ui, &mut s.form, &mut s.error, &s.worker, &s.shared);
            },
            state,
        );
        tall(&mut harness);
        save(&mut harness, "07-heberger");
        // Les mêmes, avec « Options avancées » ouvert : le choix du chiffrement.
        click(&mut harness, "Options avancées");
        save(&mut harness, "07b-heberger-options");
    }
    // Partie en cours : onglet Équipage, puis onglet Salon.
    for (tab, name) in [
        (None, "08-partie-en-cours"),
        (Some("Salon"), "09-partie-salon"),
    ] {
        // La Bibliothèque donne les jaquettes et l'image du bandeau (faux hub).
        let root = tempfile::tempdir().unwrap();
        let mut library = LibraryState::for_tests(
            demo_catalog(),
            launcher_core::Paths::under(root.path()),
            super::bibliotheque::scan_ready(),
        );
        library.art = demo_art();
        let state = Partie {
            library,
            _root: root,
            running: running::RunningState::default(),
            error: None,
            worker: Worker::detached(),
            shared: demo_party(),
        };
        let mut harness = builder().build_ui_state(
            |ui, s: &mut Partie| {
                // Comme `App` : la Bibliothèque range les images arrivées.
                s.library.background(ui.ctx());
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
        settle(&mut harness, |h| shows(h, "Jaquette de Slime Rancher"));
        if let Some(tab) = tab {
            click(&mut harness, tab);
        }
        tall(&mut harness);
        save(&mut harness, name);
    }
    // Amis (sans compte) et Paramètres.
    {
        let mut harness = builder().build_ui_state(
            |ui, s: &mut FriendsState| {
                friends::show(ui, s);
            },
            FriendsState::for_tests(),
        );
        tall(&mut harness);
        save(&mut harness, "10-amis");
    }
    {
        let mut harness = builder().build_ui_state(
            |ui, s: &mut FriendsState| {
                settings::show(ui, s, None);
            },
            FriendsState::for_tests(),
        );
        harness.run();
        save(&mut harness, "11-parametres");
    }
    // Accueil, avec la fausse partie en cours et le faux hub.
    {
        struct Accueil {
            shared: Shared,
            library: LibraryState,
            join: JoinState,
            _root: tempfile::TempDir,
        }
        let root = tempfile::tempdir().unwrap();
        let mut library = LibraryState::for_tests(
            demo_catalog(),
            launcher_core::Paths::under(root.path()),
            super::bibliotheque::scan_ready(),
        );
        library.art = demo_art();
        let state = Accueil {
            shared: demo_party(),
            library,
            join: JoinState::without_clipboard(),
            _root: root,
        };
        let mut harness = builder().build_ui_state(
            |ui, s: &mut Accueil| {
                s.library.background(ui.ctx());
                home::show(ui, &s.shared, &mut s.library, &s.join);
            },
            state,
        );
        settle(&mut harness, |h| shows(h, "Jaquette de Slime Rancher"));
        tall(&mut harness);
        save(&mut harness, "12-accueil");
    }
    // Catalogue (« Parcourir »), avec le faux hub.
    {
        struct Catalogue {
            state: catalog::CatalogState,
            library: LibraryState,
            _root: tempfile::TempDir,
        }
        let root = tempfile::tempdir().unwrap();
        // Le vrai catalogue embarqué : une dizaine de jeux, pour voir la grille entière
        // (avec un seul jeu, le bug de la grille du 22 novembre 2026 ne se voyait pas).
        let mut library = LibraryState::for_tests(
            launcher_core::Catalog::embedded().unwrap(),
            launcher_core::Paths::under(root.path()),
            super::bibliotheque::scan_ready(),
        );
        library.art = demo_art();
        let state = Catalogue {
            state: catalog::CatalogState::default(),
            library,
            _root: root,
        };
        let mut harness = builder().build_ui_state(
            |ui, s: &mut Catalogue| {
                s.library.background(ui.ctx());
                catalog::show(ui, &mut s.state, &mut s.library);
            },
            state,
        );
        settle(&mut harness, |h| shows(h, "Jaquette de Slime Rancher"));
        save(&mut harness, "13-catalogue");
        // Fiche d'information d'un jeu, ouverte d'un clic sur sa carte.
        click(&mut harness, "Fiche de Slime Rancher");
        settle(&mut harness, |h| shows(h, "Illustration de Slime Rancher"));
        tall(&mut harness);
        save(&mut harness, "13b-catalogue-fiche");
    }
    // Mon profil (sans compte).
    {
        let mut harness = builder().build_ui_state(
            |ui, s: &mut FriendsState| {
                friends::show_profile(ui, s);
            },
            FriendsState::for_tests(),
        );
        harness.run();
        save(&mut harness, "14-profil");
    }
    // Assistant du premier lancement, premier pas.
    {
        struct Bienvenue {
            welcome: welcome::WelcomeState,
            library: LibraryState,
            _root: tempfile::TempDir,
        }
        let root = tempfile::tempdir().unwrap();
        let state = Bienvenue {
            welcome: welcome::WelcomeState::opened(),
            library: LibraryState::for_tests(
                demo_catalog(),
                launcher_core::Paths::under(root.path()),
                super::bibliotheque::scan_ready(),
            ),
            _root: root,
        };
        let mut harness = builder().build_ui_state(
            |ui, s: &mut Bienvenue| {
                let games = s.library.summaries();
                welcome::show(ui.ctx(), &mut s.welcome, None, &games);
            },
            state,
        );
        harness.run();
        save(&mut harness, "15-assistant");
    }
    // Le dossier contient bien une image par écran.
    let count = std::fs::read_dir(output_dir())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .is_ok_and(|e| e.path().extension().is_some_and(|x| x == "png"))
        })
        .count();
    assert!(count >= 15, "{count} captures seulement");
}
