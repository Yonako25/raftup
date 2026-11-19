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
//! Un `meta.yaml` (préréglages « Rapide » et « Long », ADR 0023) est compris s'il ne
//! règle que `progression_balancing` et `accessibility` dans sa catégorie `null:` (tous
//! les jeux) : ses valeurs écrasent celles des joueurs, comme dans Archipelago.
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
    read_players_with_meta(files, None, seed)
}

/// Comme [`read_players`], avec le contenu d'un `meta.yaml` s'il y en a un.
pub fn read_players_with_meta(
    files: &[(String, String)],
    meta: Option<&str>,
    seed: u64,
) -> Result<Vec<PlayerRequest>, Unsupported> {
    // Le meta a son propre flux (slot 0) : il ne change pas les tirages des joueurs.
    let balancing = match meta {
        Some(text) => read_meta(text, &mut stream("meta", seed, 0))?,
        None => None,
    };
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
    if let Some(value) = balancing {
        for player in &mut players {
            player.progression_balancing = value;
        }
    }
    Ok(players)
}

/// Lit un `meta.yaml` ; renvoie la valeur de `progression_balancing` imposée à tous, s'il
/// y en a une. Tout autre réglage demande le générateur Python.
fn read_meta<R: Rng + ?Sized>(text: &str, rng: &mut R) -> Result<Option<u8>, Unsupported> {
    let docs = YamlLoader::load_from_str(text)
        .map_err(|e| unsupported(format!("meta.yaml illisible ({e})")))?;
    let mut balancing = None;
    for doc in docs {
        let Yaml::Hash(root) = doc else {
            return Err(unsupported("meta.yaml : le document n'est pas une table"));
        };
        for (key, value) in &root {
            match key {
                Yaml::String(k) if k == "meta_description" => {}
                // `null:` en YAML : la catégorie qui vaut pour tous les jeux.
                Yaml::Null => {
                    let Yaml::Hash(options) = value else {
                        return Err(unsupported("meta.yaml : catégorie « null » mal formée"));
                    };
                    for (option, raw) in options {
                        let name = match option {
                            Yaml::String(o) => o.as_str(),
                            _ => "",
                        };
                        let bad = |what: &str| {
                            unsupported(format!("meta.yaml : option « {name} » : {what}"))
                        };
                        match name {
                            "progression_balancing" => {
                                balancing = Some(
                                    progression_balancing(raw, rng)
                                        .ok_or_else(|| bad("0 à 99 attendu"))?,
                                );
                            }
                            "accessibility" => {
                                if !is_full(raw, rng)? {
                                    return Err(bad(
                                        "seule la valeur « full » est prise en charge",
                                    ));
                                }
                            }
                            _ => return Err(bad("pas encore prise en charge par le moteur Rust")),
                        }
                    }
                }
                Yaml::String(k) => {
                    return Err(unsupported(format!(
                    "meta.yaml : catégorie « {k} » pas encore prise en charge par le moteur Rust"
                )))
                }
                _ => return Err(unsupported("meta.yaml : clé non textuelle")),
            }
        }
    }
    Ok(balancing)
}

/// `accessibility` vaut `full` (nom ou numéro 0) ?
fn is_full<R: Rng + ?Sized>(raw: &Yaml, rng: &mut R) -> Result<bool, Unsupported> {
    let value = pick(raw, rng)?;
    Ok(
        matches!(&value, Yaml::String(s) if s.eq_ignore_ascii_case("full"))
            || matches!(value, Yaml::Integer(0)),
    )
}

/// `progression_balancing` : 0 à 99, `disabled` (0), `normal` (50), `extreme` (99) ou
/// `random`. Les autres formes de hasard (`random-low`…) demandent le générateur Python.
fn progression_balancing<R: Rng + ?Sized>(raw: &Yaml, rng: &mut R) -> Option<u8> {
    let value = pick(raw, rng).ok()?;
    if let Yaml::String(s) = &value {
        match s.to_ascii_lowercase().as_str() {
            "disabled" => return Some(0),
            "normal" => return Some(fill::DEFAULT_BALANCING),
            "extreme" => return Some(fill::MAX_BALANCING),
            _ => {}
        }
    }
    range_value(value, 0, fill::MAX_BALANCING, rng)
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
    let mut common = Common::default();
    let options = match game.as_str() {
        apquest::GAME => GameOptions::ApQuest(read_apquest(&name, section, rng, &mut common)?),
        _ => return Err(unsupported(format!("jeu « {game} » non pris en charge"))),
    };
    Ok(PlayerRequest {
        name,
        options,
        progression_balancing: common.progression_balancing,
    })
}

/// Options communes à tous les jeux, lues dans la section du jeu.
struct Common {
    progression_balancing: u8,
}

impl Default for Common {
    fn default() -> Self {
        Common {
            progression_balancing: fill::DEFAULT_BALANCING,
        }
    }
}

/// Lit la section APQuest. Les options absentes gardent leur valeur par défaut.
fn read_apquest<R: Rng + ?Sized>(
    player: &str,
    section: &Yaml,
    rng: &mut R,
    common: &mut Common,
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
            "progression_balancing" => {
                common.progression_balancing =
                    progression_balancing(raw, rng).ok_or_else(|| bad("0 à 99 attendu"))?
            }
            "accessibility" => {
                if !is_full(raw, rng)? {
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
    range_value(value, min, max, rng)
}

/// Comme [`range`], pour une valeur déjà tirée.
fn range_value<R: Rng + ?Sized>(value: Yaml, min: u8, max: u8, rng: &mut R) -> Option<u8> {
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
