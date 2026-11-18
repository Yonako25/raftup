//! Lecture des YAML des joueurs pour le moteur Rust.
//!
//! Le moteur Rust ne prend une génération que s'il comprend **tout** le YAML (ADR 0027,
//! § 6) : jeu pris en charge, options connues, aucune option avancée (plando, `item_links`,
//! `start_inventory`…). Sinon [`read_players`] renvoie [`Unsupported`] avec la raison, et
//! l'appelant passe la génération au générateur Python, qui sait tout lire.
//!
//! Ce qui est compris :
//!
//! - plusieurs documents par fichier (un par joueur) ;
//! - `name`, `game`, `description`, `requires` ;
//! - une valeur simple, ou des poids (`hard_mode: {true: 30, false: 70}`), ou `random` ;
//! - les sections d'autres jeux (quand `game` est tiré au hasard), ignorées.
//!
//! Les tirages (poids, `random`) utilisent un flux propre à chaque joueur, dérivé de la
//! seed : même seed et mêmes fichiers ⇒ mêmes options.

use rand_core::Rng;
use yaml_rust2::{Yaml, YamlLoader};

use crate::worlds::apquest::{self, ApQuestOptions, PlayerSprite};
use crate::{stream, GameOptions, PlayerRequest, SUPPORTED_GAMES};

/// Le YAML ne peut pas être généré par le moteur Rust ; la raison est lisible par l'hôte.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Unsupported(pub String);

fn unsupported(reason: impl Into<String>) -> Unsupported {
    Unsupported(reason.into())
}

/// Options communes à tous les jeux qu'on accepte seulement vides (ou à leur valeur par
/// défaut) : les autres valeurs demandent le générateur Python.
const EMPTY_ONLY: &[&str] = &[
    "local_items",
    "non_local_items",
    "start_inventory",
    "start_inventory_from_pool",
    "start_hints",
    "start_location_hints",
    "exclude_locations",
    "priority_locations",
    "item_links",
    "plando_items",
];

/// Lit les fichiers des joueurs (`(nom du fichier, contenu)`, dans l'ordre des slots).
pub fn read_players(
    files: &[(String, String)],
    seed: u64,
) -> Result<Vec<PlayerRequest>, Unsupported> {
    let mut players = Vec::new();
    for (file, text) in files {
        let docs = YamlLoader::load_from_str(text)
            .map_err(|e| unsupported(format!("{file} : YAML illisible ({e})")))?;
        for doc in docs {
            // Les slots commencent à 1 ; chaque joueur a son propre flux de tirages.
            let slot = u32::try_from(players.len() + 1).unwrap_or(u32::MAX);
            let mut rng = stream("yaml", seed, slot);
            players.push(read_player(file, &doc, &mut rng)?);
        }
    }
    if players.is_empty() {
        return Err(unsupported("aucun joueur dans les fichiers"));
    }
    Ok(players)
}

fn read_player<R: Rng + ?Sized>(
    file: &str,
    doc: &Yaml,
    rng: &mut R,
) -> Result<PlayerRequest, Unsupported> {
    let Yaml::Hash(root) = doc else {
        return Err(unsupported(format!(
            "{file} : le document n'est pas une table"
        )));
    };
    let name = match &doc["name"] {
        Yaml::String(name) => name.clone(),
        _ => return Err(unsupported(format!("{file} : nom de joueur absent"))),
    };
    if name.contains('{') {
        return Err(unsupported(format!(
            "{file} : nom « {name} » avec substitution ({{player}}, {{number}}…)"
        )));
    }
    let game = match pick(&doc["game"], rng)? {
        Yaml::String(game) => game,
        _ => return Err(unsupported(format!("{file} : jeu absent"))),
    };
    if !SUPPORTED_GAMES.contains(&game.as_str()) {
        return Err(unsupported(format!(
            "{name} joue à « {game} », que le moteur Rust ne sait pas encore générer"
        )));
    }
    for (key, value) in root {
        let Yaml::String(key) = key else {
            return Err(unsupported(format!("{file} : clé non textuelle")));
        };
        let known = matches!(key.as_str(), "name" | "game" | "description" | "requires");
        // Une table : section d'options d'un jeu (celui tiré ou un autre, ignoré).
        if !known && !matches!(value, Yaml::Hash(_)) {
            return Err(unsupported(format!(
                "{file} : réglage « {key} » non pris en charge"
            )));
        }
    }
    let section = match &doc[game.as_str()] {
        Yaml::Hash(_) => &doc[game.as_str()],
        Yaml::BadValue | Yaml::Null => &Yaml::Null,
        _ => {
            return Err(unsupported(format!(
                "{file} : section « {game} » mal formée"
            )))
        }
    };
    let options = match game.as_str() {
        apquest::GAME => GameOptions::ApQuest(read_apquest(&name, section, rng)?),
        _ => return Err(unsupported(format!("jeu « {game} » non pris en charge"))),
    };
    Ok(PlayerRequest { name, options })
}

/// Lit la section APQuest. Les options absentes gardent leur valeur par défaut.
fn read_apquest<R: Rng + ?Sized>(
    player: &str,
    section: &Yaml,
    rng: &mut R,
) -> Result<ApQuestOptions, Unsupported> {
    let mut o = ApQuestOptions::default();
    let Yaml::Hash(entries) = section else {
        return Ok(o);
    };
    for (key, raw) in entries {
        let Yaml::String(key) = key else {
            return Err(unsupported(format!("{player} : option non textuelle")));
        };
        let bad = |what: &str| unsupported(format!("{player} : option « {key} » : {what}"));
        match key.as_str() {
            "hard_mode" => o.hard_mode = toggle(raw, rng).ok_or_else(|| bad("oui/non attendu"))?,
            "hammer" => o.hammer = toggle(raw, rng).ok_or_else(|| bad("oui/non attendu"))?,
            "extra_starting_chest" => {
                o.extra_starting_chest = toggle(raw, rng).ok_or_else(|| bad("oui/non attendu"))?
            }
            "start_with_one_confetti_cannon" => {
                o.start_with_one_confetti_cannon =
                    toggle(raw, rng).ok_or_else(|| bad("oui/non attendu"))?
            }
            "trap_chance" => {
                o.trap_chance = range(raw, 0, 100, rng).ok_or_else(|| bad("0 à 100 attendu"))?
            }
            "confetti_explosiveness" => {
                o.confetti_explosiveness =
                    range(raw, 0, 10, rng).ok_or_else(|| bad("0 à 10 attendu"))?
            }
            "player_sprite" => {
                o.player_sprite = sprite(raw, rng).ok_or_else(|| bad("apparence inconnue"))?
            }
            // L'équilibrage de progression n'est pas encore fait (ADR 0027, étape 5) :
            // la valeur est acceptée et sans effet pour l'instant.
            "progression_balancing" => {}
            "accessibility" => {
                let value = pick(raw, rng)?;
                let full = matches!(&value, Yaml::String(s) if s.eq_ignore_ascii_case("full"))
                    || matches!(value, Yaml::Integer(0));
                if !full {
                    return Err(bad("seule la valeur « full » est prise en charge"));
                }
            }
            other if EMPTY_ONLY.contains(&other) => {
                if !is_empty(raw) {
                    return Err(bad("pas encore pris en charge par le moteur Rust"));
                }
            }
            _ => return Err(bad("inconnue du moteur Rust")),
        }
    }
    Ok(o)
}

/// Une valeur simple, ou un tirage pondéré si c'est une table `{valeur: poids}`.
fn pick<R: Rng + ?Sized>(value: &Yaml, rng: &mut R) -> Result<Yaml, Unsupported> {
    let Yaml::Hash(weights) = value else {
        return Ok(value.clone());
    };
    let mut total: u64 = 0;
    let mut choices = Vec::new();
    for (choice, weight) in weights {
        let weight = match weight {
            Yaml::Integer(w) if *w >= 0 => *w as u64,
            _ => return Err(unsupported("poids non entier ou négatif")),
        };
        if weight > 0 {
            total = total.saturating_add(weight);
            choices.push((choice, total));
        }
    }
    if total == 0 {
        return Err(unsupported("tous les poids valent 0"));
    }
    let drawn = fill::below(rng, total);
    // Premier choix dont le cumul dépasse le tirage.
    let chosen = choices
        .iter()
        .find(|(_, cumulative)| drawn < *cumulative)
        .map(|(choice, _)| (*choice).clone())
        .unwrap_or(Yaml::Null);
    Ok(chosen)
}

fn is_random(value: &Yaml) -> bool {
    matches!(value, Yaml::String(s) if s.eq_ignore_ascii_case("random"))
}

/// Option oui/non (`Toggle`). YAML 1.1 (celui d'Archipelago) lit aussi `on`, `off`,
/// `yes`, `no` comme des booléens : on les accepte en texte.
fn toggle<R: Rng + ?Sized>(raw: &Yaml, rng: &mut R) -> Option<bool> {
    let value = pick(raw, rng).ok()?;
    if is_random(&value) {
        return Some(fill::below(rng, 2) == 1);
    }
    match value {
        Yaml::Boolean(b) => Some(b),
        Yaml::Integer(0) => Some(false),
        Yaml::Integer(1) => Some(true),
        Yaml::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" | "1" => Some(true),
            "false" | "off" | "no" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Option numérique bornée (`Range`) ; `random` tire uniformément entre les bornes.
fn range<R: Rng + ?Sized>(raw: &Yaml, min: u8, max: u8, rng: &mut R) -> Option<u8> {
    let value = pick(raw, rng).ok()?;
    if is_random(&value) {
        let span = u64::from(max - min) + 1;
        return u8::try_from(u64::from(min) + fill::below(rng, span)).ok();
    }
    let number = match value {
        Yaml::Integer(n) => n,
        Yaml::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    u8::try_from(number)
        .ok()
        .filter(|n| (min..=max).contains(n))
}

/// Apparence du joueur (`Choice`) : nom, alias ou numéro ; `random` parmi les quatre.
fn sprite<R: Rng + ?Sized>(raw: &Yaml, rng: &mut R) -> Option<PlayerSprite> {
    let value = pick(raw, rng).ok()?;
    if is_random(&value) {
        return PlayerSprite::from_value(fill::below(rng, 4) as i64);
    }
    match value {
        Yaml::Integer(n) => PlayerSprite::from_value(n),
        Yaml::String(s) => PlayerSprite::from_name(&s),
        _ => None,
    }
}

fn is_empty(value: &Yaml) -> bool {
    match value {
        Yaml::Null => true,
        Yaml::Array(items) => items.is_empty(),
        Yaml::Hash(entries) => entries.is_empty(),
        _ => false,
    }
}
