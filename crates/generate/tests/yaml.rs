//! Lecture des YAML des joueurs par le moteur Rust.

#![allow(clippy::unwrap_used)]

use generate::worlds::apquest::{ApQuestOptions, PlayerSprite};
use generate::{read_players, read_players_with_meta, GameOptions};

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

/// `progression_balancing` lu pour chaque joueur.
fn balancing(texts: &[&str], meta: Option<&str>) -> Vec<u8> {
    read_players_with_meta(&files(texts), meta, 3)
        .unwrap()
        .iter()
        .map(|p| p.progression_balancing)
        .collect()
}

#[test]
fn equilibrage_lu_dans_la_section_du_jeu() {
    let with = |value: &str| {
        format!("name: A\ngame: APQuest\nAPQuest:\n  progression_balancing: {value}\n")
    };
    assert_eq!(balancing(&["name: A\ngame: APQuest\n"], None), [50]);
    assert_eq!(balancing(&[&with("0")], None), [0]);
    assert_eq!(balancing(&[&with("99")], None), [99]);
    assert_eq!(balancing(&[&with("disabled")], None), [0]);
    assert_eq!(balancing(&[&with("normal")], None), [50]);
    assert_eq!(balancing(&[&with("extreme")], None), [99]);
    assert_eq!(balancing(&[&with("\"25\"")], None), [25]);
    // Poids, puis `random` : reproductible à seed égale, toujours dans 0..=99.
    let weighted = with("{10: 0, 70: 5}");
    assert_eq!(balancing(&[&weighted], None), [70]);
    let random = with("random");
    let drawn = balancing(&[&random], None);
    assert!(drawn[0] <= 99);
    assert_eq!(balancing(&[&random], None), drawn);
    // Hors bornes ou forme de hasard inconnue : le générateur Python s'en charge.
    assert!(reason(&[&with("100")]).contains("progression_balancing"));
    assert!(reason(&[&with("random-low")]).contains("progression_balancing"));
}

#[test]
fn meta_yaml_des_prereglages() {
    let a = "name: A\ngame: APQuest\nAPQuest:\n  progression_balancing: 10\n";
    let b = "name: B\ngame: APQuest\n";
    // Préréglage « Long » (ADR 0023) : sa valeur écrase celle des joueurs.
    let long = "meta_description: Raftup, partie longue\nnull:\n  progression_balancing: 20\n  accessibility: full\n";
    assert_eq!(balancing(&[a, b], Some(long)), [20, 20]);
    // Meta sans équilibrage : les valeurs des joueurs restent.
    let only_description = "meta_description: rien\n";
    assert_eq!(balancing(&[a, b], Some(only_description)), [10, 50]);
    // Le meta ne change pas les tirages des joueurs.
    let random = "name: A\ngame: APQuest\nAPQuest:\n  hard_mode: random\n";
    assert_eq!(
        read_players(&files(&[random]), 9).unwrap(),
        read_players_with_meta(&files(&[random]), Some(only_description), 9).unwrap()
    );
    // Préréglage « Rapide » (accès minimal) et autres réglages : Python.
    let meta_reason = |meta: &str| {
        read_players_with_meta(&files(&[b]), Some(meta), 0)
            .unwrap_err()
            .0
    };
    let rapide =
        "meta_description: x\nnull:\n  progression_balancing: 80\n  accessibility: minimal\n";
    assert!(meta_reason(rapide).contains("accessibility"));
    assert!(meta_reason("meta_description: x\nnull:\n  hint_cost: 5\n").contains("hint_cost"));
    assert!(meta_reason("meta_description: x\nAPQuest:\n  hammer: true\n").contains("APQuest"));
    assert!(meta_reason("null: [\n").contains("illisible"));
}
