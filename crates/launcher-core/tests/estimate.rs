//! Tests de la durée estimée d'une seed (ADR 0030).

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use launcher_core::estimate::{
    estimate_with, DEFAULT_SECONDS_PER_LOCATION, MAX_SECONDS_PER_LOCATION,
};
use launcher_core::{estimate, estimate_file, Catalog, SeedSummary};
use multidata::MultiData;

fn seed_mit() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago")
}

fn apquest_deux() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../generate/tests/data/apquest-deux.archipelago")
}

/// Une seed écrite à la main, à partir d'une vraie (pour les types du protocole) :
///
/// - Alice (« Slow », 100 s par lieu) : lieux 1, 2, 3 ;
/// - Bob (« Fast », 10 s par lieu) : lieux 11, 12, 13, plus 14 dans aucune sphère ;
/// - un groupe, qui ne joue pas ;
/// - sphères : `{A: 1, B: 11}`, `{A: 2 3, B: 12}`, `{B: 13}`.
fn a_la_main() -> MultiData {
    let (_, mut data) = SeedSummary::load(&seed_mit()).unwrap();
    let model_slot = data.slots.values().next().unwrap().clone();
    let model_item = *data
        .locations
        .values()
        .next()
        .unwrap()
        .values()
        .next()
        .unwrap();
    let mut alice = model_slot.clone();
    alice.name = "Alice".into();
    alice.game = "Slow".into();
    alice.group_members = Vec::new();
    let mut bob = model_slot.clone();
    bob.name = "Bob".into();
    bob.game = "Fast".into();
    bob.group_members = Vec::new();
    let mut group = model_slot;
    group.name = "Groupe".into();
    group.game = "Fast".into();
    group.group_members = vec![1, 2];
    data.slots = BTreeMap::from([(1, alice), (2, bob), (3, group)]);
    let places = |ids: &[i64]| ids.iter().map(|id| (*id, model_item)).collect();
    data.locations = BTreeMap::from([(1, places(&[1, 2, 3])), (2, places(&[11, 12, 13, 14]))]);
    data.spheres = vec![
        BTreeMap::from([(1, vec![1]), (2, vec![11])]),
        BTreeMap::from([(1, vec![2, 3]), (2, vec![12])]),
        BTreeMap::from([(2, vec![13])]),
    ];
    data
}

fn seconds(game: &str) -> Option<u32> {
    match game {
        "Slow" => Some(100),
        "Fast" => Some(10),
        _ => None,
    }
}

#[test]
fn calcul_a_la_main() {
    let e = estimate_with(&a_la_main(), seconds);
    assert_eq!(e.spheres, 3);
    // Le groupe n'est pas un joueur.
    assert_eq!(e.players.len(), 2);
    let alice = &e.players[0];
    assert_eq!((alice.name.as_str(), alice.locations), ("Alice", 3));
    assert_eq!(alice.play_seconds, 300);
    // Sphère 0 : 100 s (Alice est la plus lente) ; sphère 1 : 200 s ; Alice finit à 300 s.
    assert_eq!(alice.finish_seconds, 300);
    assert_eq!(alice.wait_seconds(), 0);
    assert!(alice.from_profile);
    let bob = &e.players[1];
    assert_eq!(bob.play_seconds, 40);
    // Sphère 2 à partir de 300 s, + 10 s, puis le lieu 14 (hors sphère) : 320 s.
    assert_eq!(bob.finish_seconds, 320);
    assert_eq!(bob.wait_seconds(), 280);
    assert_eq!(e.finish_seconds(), 320);
    assert_eq!(e.longest_play_seconds(), 300);
}

#[test]
fn sans_temps_connu_ni_spheres() {
    let mut data = a_la_main();
    data.spheres.clear();
    let e = estimate_with(&data, |_| None);
    assert_eq!(e.spheres, 0);
    for p in &e.players {
        assert!(!p.from_profile);
        assert_eq!(p.seconds_per_location, DEFAULT_SECONDS_PER_LOCATION);
        // Sans sphères : pas d'attente calculable, la fin vaut le temps de jeu.
        assert_eq!(p.finish_seconds, p.play_seconds);
    }
}

#[test]
fn temps_hors_bornes_ignore() {
    let e = estimate_with(&a_la_main(), |_| Some(MAX_SECONDS_PER_LOCATION + 1));
    assert!(e.players.iter().all(|p| !p.from_profile));
    let e = estimate_with(&a_la_main(), |_| Some(0));
    assert!(e
        .players
        .iter()
        .all(|p| p.seconds_per_location == DEFAULT_SECONDS_PER_LOCATION));
}

#[test]
fn vraies_seeds() {
    let catalog = Catalog::embedded().unwrap();
    for path in [seed_mit(), apquest_deux()] {
        let (summary, data) = SeedSummary::load(&path).unwrap();
        let e = estimate(&data, &catalog);
        assert_eq!(e, estimate_file(&path, &catalog).unwrap());
        assert_eq!(e.players.len(), summary.players);
        assert!(!data.spheres.is_empty(), "{}", path.display());
        for p in &e.players {
            assert!(p.locations > 0);
            assert_eq!(
                p.play_seconds,
                p.locations as u64 * u64::from(p.seconds_per_location)
            );
            assert!(p.finish_seconds >= p.play_seconds);
        }
    }
}

#[test]
fn le_temps_du_profil_est_utilise() {
    let text = include_str!("../../../profiles/slime-rancher.toml");
    let catalog = Catalog::from_texts(&[("slime-rancher.toml", text)]).unwrap();
    let mut data = a_la_main();
    if let Some(slot) = data.slots.get_mut(&1) {
        slot.game = "Slime Rancher".into();
    }
    let e = estimate(&data, &catalog);
    let profile = catalog.get("slime-rancher").unwrap();
    let expected = profile
        .game
        .seconds_per_location
        .unwrap_or(DEFAULT_SECONDS_PER_LOCATION);
    assert_eq!(e.players[0].seconds_per_location, expected);
    assert_eq!(
        e.players[0].from_profile,
        profile.game.seconds_per_location.is_some()
    );
}

/// Réglages de l'équilibrage par temps (ADR 0030) : temps du profil, défaut sinon, valeur
/// hors bornes ignorée, un réglage par slot dans l'ordre donné.
#[test]
fn reglages_de_l_equilibrage_par_temps() {
    let settings =
        launcher_core::time_balance_settings(["Slime Rancher", "APQuest", "Bizarre"], |game| {
            match game {
                "Slime Rancher" => Some(240),
                "Bizarre" => Some(0),
                _ => None,
            }
        });
    assert!(settings.enabled);
    assert_eq!(settings.seconds_per_location, vec![240, 120, 120]);

    // Profils embarqués : Slime Rancher a son temps, APQuest (sans profil) le défaut.
    let embedded = launcher_core::time_balance_embedded(["Slime Rancher", "APQuest"]);
    assert_eq!(embedded.seconds_per_location, vec![240, 120]);
}
