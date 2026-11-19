//! Équilibrage par temps de jeu branché dans le générateur (ADR 0030).
//!
//! - désactivé (par défaut) : la seed est exactement celle d'avant l'option (l'empreinte de
//!   `empreinte.rs` le vérifie aussi) et aucun rapport n'est rendu ;
//! - activé : même seed et mêmes réglages ⇒ mêmes octets, la seed reste jouable, et
//!   l'attente simulée ne monte jamais.

#![allow(clippy::unwrap_used)]

use generate::worlds::apquest::ApQuestOptions;
use generate::{generate, GameOptions, GenerateRequest, PlayerRequest, TimeBalance};

fn tout() -> ApQuestOptions {
    ApQuestOptions {
        hard_mode: true,
        hammer: true,
        extra_starting_chest: true,
        ..ApQuestOptions::default()
    }
}

fn players() -> Vec<PlayerRequest> {
    ["A", "B", "C"]
        .into_iter()
        .map(|name| PlayerRequest::new(name, GameOptions::ApQuest(tout())))
        .collect()
}

/// Activé, avec des temps par lieu très différents : A est lent, B et C rapides.
fn enabled() -> TimeBalance {
    TimeBalance {
        enabled: true,
        seconds_per_location: vec![600, 10, 10],
        ..TimeBalance::default()
    }
}

#[test]
fn desactive_meme_seed_qu_avant() {
    for seed in 0..10 {
        let plain = generate(&GenerateRequest::new(seed, players())).unwrap();
        assert!(plain.time_balance.is_none());
        // Réglages présents mais `enabled: false` : rien ne change non plus.
        let off = GenerateRequest {
            time_balance: TimeBalance {
                enabled: false,
                ..enabled()
            },
            ..GenerateRequest::new(seed, players())
        };
        let off = generate(&off).unwrap();
        assert_eq!(off.archipelago, plain.archipelago);
        assert!(off.time_balance.is_none());
    }
}

#[test]
fn active_reproductible_et_utile() {
    let mut moved = false;
    for seed in 0..30 {
        let request = GenerateRequest {
            time_balance: enabled(),
            ..GenerateRequest::new(seed, players())
        };
        // `generate` vérifie que la seed reste jouable : une erreur ferait échouer ici.
        let first = generate(&request).unwrap();
        let again = generate(&request).unwrap();
        assert_eq!(first.archipelago, again.archipelago, "seed {seed}");

        let report = first.time_balance.unwrap();
        assert!(report.idle_after <= report.idle_before, "seed {seed}");
        if report.moved > 0 {
            moved = true;
            let plain = generate(&GenerateRequest::new(seed, players())).unwrap();
            assert_ne!(plain.multidata.locations, first.multidata.locations);
        }
    }
    assert!(moved, "l'équilibrage par temps n'a jamais rien avancé");
}
