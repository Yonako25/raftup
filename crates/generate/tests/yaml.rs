//! Lecture des YAML des joueurs par le moteur Rust.

#![allow(clippy::unwrap_used)]

use generate::worlds::apquest::{ApQuestOptions, PlayerSprite};
use generate::{read_players, GameOptions};

fn files(texts: &[&str]) -> Vec<(String, String)> {
    texts
        .iter()
        .enumerate()
        .map(|(i, t)| (format!("p{i}.yaml"), t.to_string()))
        .collect()
}

fn apquest(texts: &[&str], seed: u64) -> Vec<(String, ApQuestOptions)> {
    read_players(&files(texts), seed)
        .unwrap()
        .into_iter()
        .map(|p| match p.options {
            GameOptions::ApQuest(o) => (p.name, o),
        })
        .collect()
}

fn reason(texts: &[&str]) -> String {
    read_players(&files(texts), 0).unwrap_err().0
}

#[test]
fn yaml_de_reference() {
    let dir = format!("{}/tests/data", env!("CARGO_MANIFEST_DIR"));
    let tout = std::fs::read_to_string(format!("{dir}/yaml-tout/p1.yaml")).unwrap();
    let mixte = std::fs::read_to_string(format!("{dir}/yaml-mixte/p1.yaml")).unwrap();
    let defaut = std::fs::read_to_string(format!("{dir}/yaml-defaut/p1.yaml")).unwrap();
    let players = apquest(&[&tout, &mixte, &defaut.replace("Alice", "Bob")], 0);
    assert_eq!(players[0].0, "Alice");
    assert_eq!(
        players[0].1,
        ApQuestOptions {
            hard_mode: true,
            hammer: true,
            extra_starting_chest: true,
            start_with_one_confetti_cannon: true,
            trap_chance: 50,
            confetti_explosiveness: 10,
            player_sprite: PlayerSprite::Duck,
        }
    );
    assert!(players[1].1.hard_mode);
    assert_eq!(players[1].1.trap_chance, 100);
    assert_eq!(players[1].1.player_sprite, PlayerSprite::Cat);
    assert_eq!(players[2].1, ApQuestOptions::default());
}

#[test]
fn plusieurs_documents_et_valeurs_yaml_1_1() {
    let text = "name: A\ngame: APQuest\nAPQuest:\n  hammer: 'on'\n  hard_mode: no\n---\nname: B\ngame: APQuest\n";
    let players = apquest(&[text], 0);
    assert_eq!(players.len(), 2);
    assert!(players[0].1.hammer);
    assert!(!players[0].1.hard_mode);
    assert_eq!(players[1].0, "B");
}

#[test]
fn poids_et_random_reproductibles() {
    let text = "name: A\ngame: {APQuest: 1, Other: 0}\nAPQuest:\n  hard_mode: {true: 1, false: 1}\n  trap_chance: random\n  player_sprite: {duck: 0, horse: 5}\nOther: {}\n";
    let mut hard = [false, false];
    for seed in 0..40 {
        let a = apquest(&[text], seed);
        assert_eq!(a, apquest(&[text], seed));
        assert_eq!(a[0].1.player_sprite, PlayerSprite::Horse);
        assert!(a[0].1.trap_chance <= 100);
        hard[usize::from(a[0].1.hard_mode)] = true;
    }
    assert_eq!(hard, [true, true], "les deux valeurs doivent sortir");
}

#[test]
fn options_communes_vides_acceptees() {
    let text = "name: A\ngame: APQuest\nAPQuest:\n  progression_balancing: 50\n  accessibility: full\n  start_inventory: {}\n  local_items: []\n";
    assert_eq!(apquest(&[text], 0)[0].1, ApQuestOptions::default());
}

#[test]
fn ce_qui_passe_au_python() {
    assert!(reason(&["name: A\ngame: A Link to the Past\n"]).contains("ne sait pas encore"));
    assert!(reason(&["name: P{number}\ngame: APQuest\n"]).contains("substitution"));
    assert!(
        reason(&["name: A\ngame: APQuest\nAPQuest:\n  start_inventory: {Key: 1}\n"])
            .contains("start_inventory")
    );
    assert!(
        reason(&["name: A\ngame: APQuest\nAPQuest:\n  accessibility: minimal\n"]).contains("full")
    );
    assert!(
        reason(&["name: A\ngame: APQuest\nAPQuest:\n  trap_chance: 101\n"]).contains("trap_chance")
    );
    assert!(reason(&["name: A\ngame: APQuest\nAPQuest:\n  inconnue: 1\n"]).contains("inconnue"));
    assert!(reason(&["name: A\ngame: APQuest\ntriggers: []\n"]).contains("triggers"));
    assert!(reason(&["name: [\n"]).contains("illisible"));
    assert!(reason(&[""]).contains("aucun joueur"));
}
