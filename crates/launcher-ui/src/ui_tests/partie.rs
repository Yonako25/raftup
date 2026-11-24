//! Écran « Partie en cours » : mise en page du bandeau du haut.

use egui_kittest::kittest::Queryable;
use launcher_core::{HostStatus, SeedSummary, ServerStatus, SlotStatus};

use super::builder;
use super::demo_catalog;
use crate::screens::library::{LibraryState, Scan};
use crate::screens::running::{self, RunningState};
use crate::worker::{Shared, Worker};
use launcher_core::Paths;

struct Partie {
    running: RunningState,
    error: Option<String>,
    worker: Worker,
    shared: Shared,
    /// La Bibliothèque, dont l'écran a besoin pour « Lancer et connecter » ; vide ici.
    library: LibraryState,
    /// Dossier temporaire des chemins de la Bibliothèque, gardé tant que le test vit.
    _root: tempfile::TempDir,
}

/// Une partie avec le sous-titre le plus long : chiffrée par Raftup, reprise d'une
/// sauvegarde, démarrée depuis plus d'une heure.
fn longest_subtitle() -> Shared {
    let slot = SlotStatus {
        team: 0,
        slot: 1,
        name: "Alice".to_string(),
        game: "Slime Rancher".to_string(),
        is_group: false,
        connections: 0,
        tags: Vec::new(),
        checked: 0,
        total: 10,
        status: 0,
        last_activity: None,
        away: None,
        release_votes: 0,
    };
    Shared {
        status: Some(HostStatus {
            seed: SeedSummary {
                seed_name: "71849263510274".to_string(),
                generator_version: "0.6.8".to_string(),
                slots: Vec::new(),
                players: 12,
                has_password: false,
                release_mode: "auto".to_string(),
                collect_mode: "auto".to_string(),
                hint_cost: 10,
            },
            address: "wss://partie.exemple.fr:38281".to_string(),
            encrypted: true,
            fingerprint: Some(launcher_core::Fingerprint::of_der(b"x").to_hex()),
            has_password: false,
            started_at: crate::format::now() - 5.0 * 3600.0,
            save: None,
            resumed: true,
            server: ServerStatus {
                slots: vec![slot; 12],
                connections: 0,
            },
        }),
        ..Shared::default()
    }
}

/// Bug vu sur les captures (22 novembre 2026) : un sous-titre long glissait sous la
/// pastille « En ligne » et les boutons. Il doit maintenant passer à la ligne.
#[test]
fn le_sous_titre_ne_passe_pas_sous_les_boutons() {
    let root = tempfile::tempdir().unwrap();
    let state = Partie {
        library: LibraryState::for_tests(
            demo_catalog(),
            Paths::under(root.path()),
            Scan::default(),
        ),
        _root: root,
        running: RunningState::default(),
        error: None,
        worker: Worker::detached(),
        shared: longest_subtitle(),
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
    let subtitle = harness
        .query_all_by_label_contains("hébergée sur ce PC")
        .next()
        .unwrap()
        .rect();
    for label in ["● En ligne", "Copier les invitations", "■ Arrêter"] {
        let other = harness.get_by_label(label).rect();
        assert!(
            !subtitle.intersects(other),
            "le sous-titre {subtitle:?} chevauche « {label} » {other:?}"
        );
    }
}
