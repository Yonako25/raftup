//! Empreinte d'une seed APQuest du moteur Rust seul (NF12).
//!
//! Le même tirage doit donner, octet pour octet, le même fichier `.archipelago` sous
//! Windows, macOS et Linux : la CI lance ce test sur les trois systèmes. Si l'empreinte
//! diffère sur un seul d'entre eux, quelque chose dépend du système (ordre d'une
//! `HashMap`, taille d'un entier, chemin, fin de ligne…) et la seed n'est plus
//! reproductible d'une machine à l'autre.
//!
//! **Mettre à jour la valeur attendue** : seulement quand un changement VOULU du moteur
//! modifie la seed (placement, `slot_data`, écriture du pickle, version de `flate2`…).
//! Lancer `cargo test -p generate --test empreinte`, recopier l'empreinte affichée par
//! l'échec dans `EXPECTED`, et dire dans le message de commit pourquoi elle change.
//! Une empreinte qui change sans raison connue est un bug, pas une valeur à recopier.

#![allow(clippy::unwrap_used)]

use generate::worlds::apquest::ApQuestOptions;
use generate::{generate, GameOptions, GenerateRequest, PlayerRequest};
use sha2::{Digest, Sha256};

/// sha256 du fichier `.archipelago` de la seed décrite dans `request()`.
const EXPECTED: &str = "cc6c6c409803ae75477b56b5fb02bebcf0dbc4070da94977db5f135988024085";

/// Seed fixe à deux joueurs : placement, équilibrage de progression et écriture compris.
fn request() -> GenerateRequest {
    let hard = ApQuestOptions {
        hard_mode: true,
        hammer: true,
        trap_chance: 50,
        ..ApQuestOptions::default()
    };
    GenerateRequest {
        seed: 20_261_119,
        players: vec![
            PlayerRequest::new("Alice", GameOptions::ApQuest(ApQuestOptions::default())),
            PlayerRequest::new("Bob", GameOptions::ApQuest(hard)),
        ],
        time_balance: Default::default(),
    }
}

/// Écrit l'empreinte en hexadécimal minuscule, comme `sha256sum`.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn empreinte_identique_sur_tous_les_systemes() {
    let generated = generate(&request()).unwrap();
    let digest = hex(&Sha256::digest(&generated.archipelago));
    assert_eq!(
        digest, EXPECTED,
        "empreinte de la seed APQuest changée (voir l'en-tête du fichier pour la mettre à jour)"
    );
}
