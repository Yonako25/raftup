//! APQuest, porté en Rust depuis Archipelago 0.6.8 (`worlds/apquest`, licence MIT,
//! notice dans `THIRD_PARTY_NOTICES.md`).
//!
//! Le monde : un petit jeu d'aventure à 6 ou 7 coffres. Les noms, numéros, règles, options et
//! `slot_data` sont ceux de l'original : le client APQuest d'Archipelago doit pouvoir jouer
//! une seed produite ici sans modification.

use logic::{Classification, ItemKindId, LogicError, MultiWorld, Rule, SlotId};
use rand_core::Rng;
use serde_json::{json, Value};

use crate::datapackage::PackageSource;

/// Nom du jeu, tel que l'écrivent les YAML et le datapackage.
pub const GAME: &str = "APQuest";

/// `ITEM_NAME_TO_ID` de `items.py`.
pub const ITEMS: &[(&str, i64)] = &[
    ("Key", 1),
    ("Sword", 2),
    ("Shield", 3),
    ("Hammer", 4),
    ("Health Upgrade", 5),
    ("Confetti Cannon", 6),
    ("Math Trap", 7),
];

/// `LOCATION_NAME_TO_ID` de `locations.py`.
pub const LOCATIONS: &[(&str, i64)] = &[
    ("Top Left Room Chest", 1),
    ("Top Middle Chest", 2),
    ("Bottom Left Chest", 3),
    ("Bottom Left Extra Chest", 4),
    ("Bottom Right Room Left Chest", 5),
    ("Bottom Right Room Right Chest", 6),
    ("Right Room Enemy Drop", 10),
];

/// Description du datapackage d'APQuest (aucun groupe déclaré).
pub const PACKAGE: PackageSource<'static> = PackageSource {
    items: ITEMS,
    locations: LOCATIONS,
    item_groups: &[],
    location_groups: &[],
};

/// Apparence du joueur (`PlayerSprite`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerSprite {
    #[default]
    Human = 0,
    Duck = 1,
    Horse = 2,
    Cat = 3,
}

impl PlayerSprite {
    /// Lit un nom d'option (`human`, `duck`, `horse`, `cat` ou l'alias `kitty`).
    pub fn from_name(name: &str) -> Option<PlayerSprite> {
        match name.to_ascii_lowercase().as_str() {
            "human" => Some(PlayerSprite::Human),
            "duck" => Some(PlayerSprite::Duck),
            "horse" => Some(PlayerSprite::Horse),
            "cat" | "kitty" => Some(PlayerSprite::Cat),
            _ => None,
        }
    }

    /// Lit une valeur numérique d'option.
    pub fn from_value(value: i64) -> Option<PlayerSprite> {
        match value {
            0 => Some(PlayerSprite::Human),
            1 => Some(PlayerSprite::Duck),
            2 => Some(PlayerSprite::Horse),
            3 => Some(PlayerSprite::Cat),
            _ => None,
        }
    }
}

/// Les 7 options d'APQuest (`options.py`), avec leurs valeurs par défaut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApQuestOptions {
    pub hard_mode: bool,
    pub hammer: bool,
    pub extra_starting_chest: bool,
    pub start_with_one_confetti_cannon: bool,
    /// 0 à 100.
    pub trap_chance: u8,
    /// 0 à 10.
    pub confetti_explosiveness: u8,
    pub player_sprite: PlayerSprite,
}

impl Default for ApQuestOptions {
    fn default() -> Self {
        ApQuestOptions {
            hard_mode: false,
            hammer: false,
            extra_starting_chest: false,
            start_with_one_confetti_cannon: false,
            trap_chance: 0,
            confetti_explosiveness: 3,
            player_sprite: PlayerSprite::Human,
        }
    }
}

impl ApQuestOptions {
    /// Vérifie les bornes des options numériques ; renvoie le nom de l'option fautive.
    pub fn check(&self) -> Result<(), (&'static str, String)> {
        if self.trap_chance > 100 {
            return Err(("trap_chance", "doit être entre 0 et 100".into()));
        }
        if self.confetti_explosiveness > 10 {
            return Err(("confetti_explosiveness", "doit être entre 0 et 10".into()));
        }
        Ok(())
    }

    /// Le `slot_data` envoyé au client (`fill_slot_data` de `world.py`).
    pub fn slot_data(&self) -> Value {
        json!({
            "hard_mode": i64::from(self.hard_mode),
            "hammer": i64::from(self.hammer),
            "extra_starting_chest": i64::from(self.extra_starting_chest),
            "confetti_explosiveness": self.confetti_explosiveness,
            "player_sprite": self.player_sprite as i64,
        })
    }
}

/// Ce que le monde d'un joueur ajoute au multiworld, en plus de ses régions et lieux.
pub(crate) struct BuiltPlayer {
    /// Objets à placer, un élément par exemplaire.
    pub pool: Vec<ItemKindId>,
    /// Inventaire de départ.
    pub start: Vec<ItemKindId>,
    pub slot_data: Value,
}

/// Construit le monde APQuest d'un joueur dans le multiworld (`create_regions`,
/// `set_rules`, `create_items`).
pub(crate) fn create<R: Rng + ?Sized>(
    w: &mut MultiWorld,
    p: SlotId,
    o: &ApQuestOptions,
    rng: &mut R,
) -> Result<BuiltPlayer, LogicError> {
    use Classification as C;

    // Objets : toutes les sortes existent, même celles que les options n'utilisent pas.
    let key = w.add_item_kind(p, "Key", Some(1), C::PROGRESSION)?;
    let sword_class = Classification {
        useful: true,
        ..C::PROGRESSION
    };
    let sword = w.add_item_kind(p, "Sword", Some(2), sword_class)?;
    let shield = w.add_item_kind(p, "Shield", Some(3), C::PROGRESSION)?;
    let hammer = w.add_item_kind(p, "Hammer", Some(4), C::PROGRESSION)?;
    // Les améliorations de vie ne débloquent quelque chose qu'en mode difficile.
    let health_class = if o.hard_mode {
        C::PROGRESSION
    } else {
        C::USEFUL
    };
    let health = w.add_item_kind(p, "Health Upgrade", Some(5), health_class)?;
    let confetti = w.add_item_kind(p, "Confetti Cannon", Some(6), C::FILLER)?;
    let math_trap = w.add_item_kind(p, "Math Trap", Some(7), C::TRAP)?;
    let button = w.add_item_kind(p, "Top Left Room Button Pressed", None, C::PROGRESSION)?;
    let victory = w.add_item_kind(p, "Victory", None, C::PROGRESSION)?;

    // Régions (`regions.py`) ; « Overworld », créée en premier, est le départ.
    let overworld = w.add_region(p, "Overworld")?;
    let top_left = w.add_region(p, "Top Left Room")?;
    let bottom_right = w.add_region(p, "Bottom Right Room")?;
    let right = w.add_region(p, "Right Room")?;
    let final_boss = w.add_region(p, "Final Boss Room")?;
    let top_middle = if o.hammer {
        Some(w.add_region(p, "Top Middle Room")?)
    } else {
        None
    };

    // Entrées et leurs règles (`regions.py`, `rules.py`).
    w.add_entrance(
        overworld,
        bottom_right,
        "Overworld to Bottom Right Room",
        Rule::has(sword),
    )?;
    w.add_entrance(overworld, right, "Overworld to Right Room", Rule::True)?;
    w.add_entrance(
        right,
        final_boss,
        "Right Room to Final Boss Room",
        Rule::has(button),
    )?;
    w.add_entrance(
        overworld,
        top_left,
        "Overworld to Top Left Room",
        Rule::has(key),
    )?;
    if let Some(top_middle) = top_middle {
        w.add_entrance(
            overworld,
            top_middle,
            "Overworld to Top Middle Room",
            Rule::has(hammer),
        )?;
    }

    // Lieux (`locations.py`) et leurs règles.
    let mut enemy = Rule::has(sword);
    if o.hard_mode {
        enemy = Rule::And(vec![
            enemy,
            Rule::Or(vec![Rule::has(shield), Rule::has(health)]),
        ]);
    }
    w.add_location(overworld, "Bottom Left Chest", Some(3), Rule::True)?;
    w.add_location(
        bottom_right,
        "Bottom Right Room Left Chest",
        Some(5),
        Rule::True,
    )?;
    w.add_location(
        bottom_right,
        "Bottom Right Room Right Chest",
        Some(6),
        Rule::True,
    )?;
    w.add_location(top_left, "Top Left Room Chest", Some(1), Rule::True)?;
    w.add_location(right, "Right Room Enemy Drop", Some(10), enemy)?;
    w.add_location(
        top_middle.unwrap_or(overworld),
        "Top Middle Chest",
        Some(2),
        Rule::True,
    )?;
    if o.extra_starting_chest {
        w.add_location(overworld, "Bottom Left Extra Chest", Some(4), Rule::True)?;
    }

    // Événements : le bouton et la victoire, posés d'office.
    let button_location = w.add_location(top_left, "Top Left Room Button", None, Rule::True)?;
    w.place(button_location, button)?;
    let mut boss = Rule::HasAll(vec![sword, shield]);
    if o.hard_mode {
        boss = Rule::And(vec![boss, Rule::has_count(health, 2)]);
    }
    let boss_location = w.add_location(final_boss, "Final Boss Defeated", None, boss)?;
    w.place(boss_location, victory)?;
    w.set_goal(p, Rule::has(victory))?;

    // Objets du pool (`items.py`), puis remplissage jusqu'au nombre de lieux.
    let mut pool = vec![key, sword, shield, health, health];
    if o.hammer {
        pool.push(hammer);
    }
    let unfilled = w
        .locations()
        .iter()
        .filter(|l| l.slot == p && l.item.is_none())
        .count();
    while pool.len() < unfilled {
        // `randint(0, 99) < trap_chance` en Python.
        let trap = fill::below(rng, 100) < u64::from(o.trap_chance);
        pool.push(if trap { math_trap } else { confetti });
    }

    let start = if o.start_with_one_confetti_cannon {
        vec![confetti]
    } else {
        Vec::new()
    };
    Ok(BuiltPlayer {
        pool,
        start,
        slot_data: o.slot_data(),
    })
}
