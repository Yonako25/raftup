//! Spoiler du moteur Rust : en-tête, options, placement complet et parcours par sphères.

#![allow(clippy::unwrap_used)]

use generate::worlds::apquest::{ApQuestOptions, PlayerSprite};
use generate::{generate, GameOptions, GenerateRequest, PlayerRequest};

fn request(seed: u64) -> GenerateRequest {
    GenerateRequest {
        seed,
        players: vec![
            PlayerRequest::new(
                "Alice",
                GameOptions::ApQuest(ApQuestOptions {
                    hammer: true,
                    player_sprite: PlayerSprite::Duck,
                    ..ApQuestOptions::default()
                }),
            ),
            PlayerRequest::new("Bob", GameOptions::ApQuest(ApQuestOptions::default())),
        ],
        time_balance: Default::default(),
    }
}

#[test]
fn spoiler_complet_a_deux_joueurs() {
    let generated = generate(&request(12345)).unwrap();
    let text = generated.spoiler.to_text(true);

    let first = text.lines().next().unwrap();
    assert!(first.starts_with("Raftup "), "{first}");
    assert!(
        first.contains("compatible avec Archipelago 0.6.8"),
        "{first}"
    );
    assert!(first.ends_with("Seed: 00000000000000012345"), "{first}");
    assert!(text.contains("\nPlayers:                         2\n"));
    assert!(text.contains("\nPlayer 1: Alice\n"));
    assert!(text.contains("\nPlayer 2: Bob\n"));
    assert!(text.contains("Hammer:                          Yes\n"));
    assert!(text.contains("Player Sprite:                   Duck\n"));
    assert!(text.contains("Progression Balancing:           50\n"));

    // Un lieu du spoiler par lieu de la seed, chacun avec le joueur entre parenthèses.
    let in_seed: usize = generated
        .multidata
        .locations
        .values()
        .map(|t| t.len())
        .sum();
    assert_eq!(generated.spoiler.locations.len(), in_seed);
    for (location, item) in &generated.spoiler.locations {
        assert!(location.ends_with(" (Alice)") || location.ends_with(" (Bob)"));
        assert!(item.ends_with(')'), "{item}");
        assert!(text.contains(&format!("{location}: {item}\n")));
    }

    // Parcours : au moins une sphère après le départ, chaque étape est un lieu placé.
    // APQuest sans option ne donne aucune progression de départ : sphère 0 vide, non
    // écrite ; le parcours commence à la sphère 1.
    assert!(generated.spoiler.playthrough[0].is_empty());
    assert!(text.contains("\nPlaythrough:\n\n1: {\n"), "{text}");
    assert!(!text.contains("0: {"));
    assert!(generated.spoiler.playthrough.len() > 1);
    for sphere in &generated.spoiler.playthrough[1..] {
        assert!(!sphere.is_empty());
        for step in sphere {
            assert!(step.location.is_some());
        }
    }
}

#[test]
fn spoiler_reproductible_et_sans_parcours_si_demande() {
    let a = generate(&request(7)).unwrap().spoiler;
    let b = generate(&request(7)).unwrap().spoiler;
    assert_eq!(a.to_text(true), b.to_text(true));
    let court = a.to_text(false);
    assert!(court.contains("\n\nLocations:\n\n"));
    assert!(!court.contains("Playthrough"));
}

#[test]
fn un_seul_joueur_sans_parentheses() {
    let mut req = request(3);
    req.players.truncate(1);
    let spoiler = generate(&req).unwrap().spoiler;
    let text = spoiler.to_text(true);
    assert!(!text.contains("(Alice)"), "{text}");
    assert!(text.contains("\nPlayer 1: Alice\n"));
}
