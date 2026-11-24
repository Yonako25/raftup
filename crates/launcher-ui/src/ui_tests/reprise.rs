//! « En cours » : les parties arrêtées de ce PC, à reprendre (revue d'ergonomie, point 21).
//! Sans disque : la liste est donnée toute faite (`PastGames::with_games`).

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use launcher_core::past_games::PastGame;
use launcher_core::seed::{SeedSlot, SeedSummary};

use super::{builder, click, shows, shows_part};
use crate::screens::hub_game::HubGames;
use crate::screens::matches;
use crate::screens::past_games::PastGames;
use crate::screens::setup::{self, SetupForm};
use crate::worker::Shared;

pub(super) fn game(seed_name: &str, games: &[&str], saved: bool) -> PastGame {
    let slots = games
        .iter()
        .enumerate()
        .map(|(i, game)| SeedSlot {
            slot: i as u32 + 1,
            name: format!("Joueur{i}"),
            game: game.to_string(),
            is_group: false,
            members: Vec::new(),
            locations: 10,
        })
        .collect();
    let hour_ago = SystemTime::now() - Duration::from_secs(3600);
    PastGame {
        seed: PathBuf::from(format!(
            "generations/{seed_name}/AP_{seed_name}.archipelago"
        )),
        summary: SeedSummary {
            seed_name: seed_name.to_string(),
            generator_version: "0.6.8".into(),
            slots,
            players: games.len(),
            has_password: false,
            release_mode: "auto".into(),
            collect_mode: "auto".into(),
            hint_cost: 10,
        },
        last_saved: saved.then_some(hour_ago),
        generated_at: Some(hour_ago),
    }
}

struct EnCours {
    shared: Shared,
    hub: HubGames,
    past: PastGames,
}

fn en_cours(shared: Shared, past: Vec<PastGame>) -> Harness<'static, EnCours> {
    let state = EnCours {
        shared,
        hub: HubGames::default(),
        past: PastGames::with_games(past),
    };
    let mut harness = builder().build_ui_state(
        |ui, s: &mut EnCours| {
            matches::show(ui, &s.shared, None, &mut s.hub, &mut s.past);
        },
        state,
    );
    harness.run();
    harness
}

#[test]
fn les_parties_arretees_se_reprennent() {
    let mut harness = en_cours(
        Shared::default(),
        vec![
            game("111", &["Slime Rancher", "Slime Rancher"], true),
            game("222", &["Hollow Knight"], false),
        ],
    );
    assert!(shows(&harness, "À REPRENDRE SUR CE PC"));
    assert!(shows(&harness, "Partie Slime Rancher"));
    assert!(shows_part(&harness, "2 joueurs · dernière partie il y a"));
    assert!(shows(&harness, "Partie Hollow Knight"));
    assert!(shows_part(&harness, "jamais lancée"));
    // Des parties à reprendre : « En cours » n'est pas vide.
    assert!(!shows(&harness, "Aucune partie en cours"));

    // La première « Reprendre » est celle de la première partie de la liste.
    harness
        .query_all_by_label("Reprendre")
        .next()
        .expect("bouton Reprendre")
        .click();
    harness.run();
    assert_eq!(
        harness.state_mut().past.take_resume(),
        Some(PathBuf::from("generations/111/AP_111.archipelago"))
    );
    // Une seule fois.
    assert_eq!(harness.state_mut().past.take_resume(), None);
}

/// Une partie tourne sur ce PC : elle n'est pas dans la liste, et « Reprendre » ne fait
/// rien (un seul serveur à la fois).
#[test]
fn pas_de_reprise_pendant_une_partie() {
    let mut running = super::parcours::hosted(false);
    running.seed.seed_name = "111".into();
    let shared = Shared {
        status: Some(running),
        ..Shared::default()
    };
    let mut harness = en_cours(
        shared,
        vec![
            game("111", &["Slime Rancher"], true),
            game("222", &["Hollow Knight"], true),
        ],
    );
    assert!(shows(&harness, "Partie Hollow Knight"));
    // La partie qui tourne n'apparaît qu'une fois, en haut (pas dans « À reprendre »).
    assert_eq!(harness.query_all_by_label("Reprendre").count(), 1);
    click(&mut harness, "Reprendre");
    assert_eq!(harness.state_mut().past.take_resume(), None);
}

/// Rien sur ce PC, rien sur Rafthub, rien à reprendre : l'état vide reste.
#[test]
fn sans_partie_l_etat_vide() {
    let harness = en_cours(Shared::default(), Vec::new());
    assert!(shows(&harness, "Aucune partie en cours"));
    assert!(!shows(&harness, "À REPRENDRE SUR CE PC"));
}

/// « Reprendre » relance sans mot de passe, avec les réglages du serveur de l'écran
/// « Héberger un fichier ».
#[test]
fn reprendre_relance_sans_mot_de_passe() {
    let form = SetupForm::with_webhook(String::new());
    let seed = PathBuf::from("generations/111/AP_111.archipelago");
    let options = setup::relaunch_options(&form, seed.clone()).unwrap();
    assert_eq!(options.seed, seed);
    assert_eq!(options.password, None);
    assert_eq!(options.port, 38281);
    assert!(!options.disable_save);
}
