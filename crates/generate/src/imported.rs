//! Mondes exportés depuis Archipelago (ADR 0028, étape 1).
//!
//! Le worker Python construit les mondes jusqu'à `pre_fill` puis les décrit dans un
//! fichier JSON (`raftup_export.py`, `format_version` 2). Ce module lit ce fichier et en
//! fait un [`logic::MultiWorld`] prêt à être rempli par `fill`.
//!
//! Le fichier vient du code d'un apworld, qu'on ne contrôle pas : on le lit comme une
//! donnée hostile. Taille bornée, nombre d'éléments plafonné, chaque nom vérifié ; une
//! erreur n'arrête que l'import, jamais le programme.
//!
//! Ce qui n'est pas encore pris en charge rend le monde **non exportable**
//! ([`ImportError::Unsupported`]) : la génération reste alors au générateur Python.
//! À cette étape : règle opaque (lambda que le sondage n'a pas su traduire), règle
//! d'objet que l'export n'a pas su calculer, monde qui redéfinit `collect`. Sont pris en
//! charge : lieux prioritaires et exclus (`progress_type`), règles d'objet (`item_rule`,
//! `always_allow`, donc `local_items` / `non_local_items`), objets à placer tôt
//! (`early_items`, `local_early_items`) et `accessibility` (ADR 0029).
//!
//! **Contrôle croisé** (ADR 0028, § 4) : l'export contient des inventaires tirés au
//! hasard et la valeur de chaque règle calculée par le vrai Archipelago. On recalcule
//! chaque valeur avec la règle traduite ; au moindre désaccord, le monde reste en Python.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use logic::{
    Accessibility, Classification, EntranceId, ItemKindId, ItemRule, ItemRuleId, LocationId,
    LogicError, MultiWorld, Progress, Reachability, RegionId, Rule, SlotId, State,
};
use multidata::GamePackage;
use serde::Deserialize;
use thiserror::Error;

use crate::datapackage::{game_package, PackageSource};

/// Version du format d'export comprise ici (2 : règles d'objet, objets à placer tôt,
/// accessibilité, ADR 0029).
pub const FORMAT_VERSION: u32 = 2;
/// Taille maximale du fichier d'export (comme une seed, NF08).
pub const MAX_EXPORT_BYTES: usize = 64 * 1024 * 1024;
/// Nombre maximal d'éléments (régions, entrées, lieux, objets) dans un export.
pub const MAX_ELEMENTS: usize = 1_000_000;
/// Nombre maximal de combinaisons pour traduire « au moins N parmi ces règles ».
pub const MAX_COMBINATIONS: usize = 256;

/// Erreurs de l'import.
#[derive(Debug, Error)]
pub enum ImportError {
    #[error("export trop gros : {0} octets")]
    TooLarge(usize),
    #[error("export illisible : {0}")]
    Json(#[from] serde_json::Error),
    #[error("format d'export {0} inconnu (attendu : {FORMAT_VERSION})")]
    Format(u32),
    #[error("export incohérent : {0}")]
    Invalid(String),
    /// Le monde ne peut pas (encore) passer par le moteur Rust : générateur Python.
    #[error("{player} ({game}) : {reason}")]
    Unsupported {
        player: String,
        game: String,
        reason: String,
    },
    #[error("monde mal construit : {0}")]
    Logic(#[from] LogicError),
    #[error("placement impossible : {0}")]
    Fill(#[from] fill::FillError),
}

fn invalid(message: impl Into<String>) -> ImportError {
    ImportError::Invalid(message.into())
}

// ---- Le format JSON (ce qu'écrit `raftup_export.py`) ----

/// L'export complet.
#[derive(Debug, Clone, Deserialize)]
pub struct Export {
    pub format_version: u32,
    pub seed_name: String,
    pub players: Vec<PlayerExport>,
    pub pool: Vec<ItemExport>,
    /// Règles d'objet, partagées entre les lieux (ADR 0029).
    #[serde(default)]
    pub item_rules: Vec<ItemRuleExport>,
    pub datapackage: IndexMap<String, PackageExport>,
    /// Contrôle croisé ; absent seulement dans les exports écrits à la main des tests.
    #[serde(default)]
    pub checks: Option<Checks>,
}

/// Une règle d'objet : quelles sortes d'objets du pool un lieu accepte, calculé par
/// Archipelago (`item_rule`, `always_allow`) sur chaque sorte. Objets : `(joueur, nom)`.
#[derive(Debug, Clone, Deserialize)]
pub struct ItemRuleExport {
    /// Vrai : seuls les objets de `list` sont acceptés ; faux : ils sont refusés.
    pub allow_only: bool,
    pub list: Vec<(u32, String)>,
    /// Objets toujours acceptés (`always_allow`), même dans un lieu exclu.
    #[serde(default)]
    pub always: Vec<(u32, String)>,
}

/// Inventaires du contrôle croisé et résultats du vrai Archipelago.
#[derive(Debug, Clone, Deserialize)]
pub struct Checks {
    /// Chaque inventaire : `(joueur, objet, nombre)`.
    pub samples: Vec<Vec<(u32, String, u32)>>,
    pub rules: Vec<CheckedRule>,
    /// Contrôle des règles d'objet : `(joueur du lieu, lieu, joueur de l'objet, objet,
    /// résultat)`, le résultat étant `Location.can_fill` d'Archipelago sans l'accès
    /// (`'1'`, `'0'`, ou `'e'` si Python a planté).
    #[serde(default)]
    pub item_checks: Vec<(u32, String, u32, String, String)>,
}

/// Une règle et sa valeur sur chaque inventaire : `'1'` vraie, `'0'` fausse, `'e'` la
/// règle Python a levé une exception.
#[derive(Debug, Clone, Deserialize)]
pub struct CheckedRule {
    /// `entrance`, `location` ou `goal`.
    pub kind: String,
    pub player: u32,
    /// Nom de l'entrée ou du lieu (du joueur, pour `goal`).
    pub name: String,
    pub results: String,
}

/// Un joueur.
#[derive(Debug, Clone, Deserialize)]
pub struct PlayerExport {
    pub slot: u32,
    pub name: String,
    pub game: String,
    pub origin: String,
    pub regions: Vec<String>,
    pub entrances: Vec<EntranceExport>,
    pub locations: Vec<LocationExport>,
    pub precollected: Vec<ItemExport>,
    pub goal: RuleExport,
    /// Le monde redéfinit `collect`, `collect_item` ou `remove`.
    #[serde(default)]
    pub custom_collect: bool,
    /// Option `progression_balancing` du joueur (0 à 99, 50 par défaut).
    #[serde(default = "default_balancing")]
    pub progression_balancing: u8,
    /// Option `accessibility` : 0 `full`, 1 `items`, 2 `minimal`.
    #[serde(default)]
    pub accessibility: u8,
    /// Objets à placer tôt, n'importe où (`early_items`) : nom → nombre.
    #[serde(default)]
    pub early_items: BTreeMap<String, u32>,
    /// Objets à placer tôt chez le joueur (`local_early_items`) : nom → nombre.
    #[serde(default)]
    pub local_early_items: BTreeMap<String, u32>,
    /// Une règle d'objet que l'export n'a pas su calculer (exception, `always_allow` qui
    /// lit l'inventaire, `can_fill` redéfini) : le monde reste en Python.
    #[serde(default)]
    pub item_rule_error: Option<String>,
    pub required_client_version: Vec<u32>,
}

fn default_balancing() -> u8 {
    fill::DEFAULT_BALANCING
}

/// Une entrée (`to` vaut `null` si elle n'est reliée à rien).
#[derive(Debug, Clone, Deserialize)]
pub struct EntranceExport {
    pub name: String,
    pub from: String,
    pub to: Option<String>,
    pub rule: RuleExport,
}

/// Un lieu.
#[derive(Debug, Clone, Deserialize)]
pub struct LocationExport {
    pub name: String,
    pub address: Option<i64>,
    pub region: String,
    pub rule: RuleExport,
    /// 1 = normal, 2 = prioritaire, 3 = exclu (`LocationProgressType`).
    pub progress_type: u8,
    pub locked: bool,
    /// Objet déjà posé (événement, objet verrouillé par le monde).
    pub item: Option<ItemExport>,
    /// Index de la règle d'objet du lieu dans [`Export::item_rules`] ; `None` : tout objet
    /// du pool est accepté.
    #[serde(default)]
    pub item_rule: Option<u32>,
}

/// Un objet (un exemplaire).
#[derive(Debug, Clone, Deserialize)]
pub struct ItemExport {
    pub player: u32,
    pub name: String,
    pub code: Option<i64>,
    /// Drapeaux `ItemClassification` d'Archipelago.
    pub classification: u32,
}

/// Le datapackage d'un jeu, tel que le donne `World.get_data_package_data`.
#[derive(Debug, Clone, Deserialize)]
pub struct PackageExport {
    pub item_name_to_id: IndexMap<String, i64>,
    pub location_name_to_id: IndexMap<String, i64>,
    pub item_name_groups: IndexMap<String, Vec<String>>,
    pub location_name_groups: IndexMap<String, Vec<String>>,
    pub checksum: String,
}

/// Une règle, dans le vocabulaire de `rule_builder`.
///
/// `#[serde(tag = "t")]` : le champ `"t"` du JSON dit quelle variante lire.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum RuleExport {
    True,
    False,
    /// Règle que l'export n'a pas su traduire (`why` : son nom Python).
    Opaque {
        why: String,
    },
    And {
        of: Vec<RuleExport>,
    },
    Or {
        of: Vec<RuleExport>,
    },
    /// Au moins `count` des sous-règles.
    AtLeast {
        count: u32,
        of: Vec<RuleExport>,
    },
    Has {
        player: u32,
        item: String,
        count: u32,
    },
    HasAll {
        player: u32,
        items: Vec<String>,
    },
    HasAny {
        player: u32,
        items: Vec<String>,
    },
    HasAllCounts {
        player: u32,
        items: Vec<(String, u32)>,
    },
    HasAnyCount {
        player: u32,
        items: Vec<(String, u32)>,
    },
    /// Somme des exemplaires des objets de la liste ≥ `count`.
    HasFromList {
        player: u32,
        items: Vec<String>,
        count: u32,
    },
    /// Nombre d'objets **différents** de la liste ≥ `count`.
    HasFromListUnique {
        player: u32,
        items: Vec<String>,
        count: u32,
    },
    CanReachLocation {
        player: u32,
        location: String,
    },
    CanReachRegion {
        player: u32,
        region: String,
    },
    CanReachEntrance {
        player: u32,
        entrance: String,
    },
}

impl RuleExport {
    /// Première règle opaque de l'arbre, s'il y en a une.
    fn first_opaque(&self) -> Option<&str> {
        match self {
            RuleExport::Opaque { why } => Some(why),
            RuleExport::And { of } | RuleExport::Or { of } | RuleExport::AtLeast { of, .. } => {
                of.iter().find_map(RuleExport::first_opaque)
            }
            _ => None,
        }
    }

    /// Objets cités par la règle : `(joueur, nom)`.
    fn items(&self, out: &mut Vec<(u32, String)>) {
        match self {
            RuleExport::Has { player, item, .. } => out.push((*player, item.clone())),
            RuleExport::HasAll { player, items }
            | RuleExport::HasAny { player, items }
            | RuleExport::HasFromList { player, items, .. }
            | RuleExport::HasFromListUnique { player, items, .. } => {
                out.extend(items.iter().map(|i| (*player, i.clone())));
            }
            RuleExport::HasAllCounts { player, items }
            | RuleExport::HasAnyCount { player, items } => {
                out.extend(items.iter().map(|(i, _)| (*player, i.clone())));
            }
            RuleExport::And { of } | RuleExport::Or { of } | RuleExport::AtLeast { of, .. } => {
                for rule in of {
                    rule.items(out);
                }
            }
            _ => {}
        }
    }
}

/// Lit le fichier d'export.
pub fn read_export(bytes: &[u8]) -> Result<Export, ImportError> {
    if bytes.len() > MAX_EXPORT_BYTES {
        return Err(ImportError::TooLarge(bytes.len()));
    }
    // `serde_json` refuse une imbrication de plus de 128 niveaux : pas de débordement de
    // pile possible sur une règle construite pour nuire.
    let export: Export = serde_json::from_slice(bytes)?;
    if export.format_version != FORMAT_VERSION {
        return Err(ImportError::Format(export.format_version));
    }
    let elements = export.pool.len()
        + export
            .players
            .iter()
            .map(|p| p.regions.len() + p.entrances.len() + p.locations.len() + p.precollected.len())
            .sum::<usize>()
        + export
            .item_rules
            .iter()
            .map(|r| r.list.len() + r.always.len())
            .sum::<usize>();
    if elements > MAX_ELEMENTS {
        return Err(invalid(format!("{elements} éléments (max {MAX_ELEMENTS})")));
    }
    Ok(export)
}

// ---- Construction du monde ----

/// Ce que donne l'import : le monde à remplir et ce qu'il faut pour écrire la seed.
#[derive(Debug, Clone)]
pub struct Imported {
    pub world: MultiWorld,
    /// Objets à placer (un élément par exemplaire), dans l'ordre de l'export.
    pub pool: Vec<ItemKindId>,
    /// Inventaire de départ, déjà ramassé.
    pub start: State,
    /// Objets de départ de chaque joueur, dans l'ordre des slots.
    pub start_items: Vec<Vec<ItemKindId>>,
    /// Objets à placer tôt (`early_items`, `local_early_items`), pour `fill`.
    pub early: Vec<fill::EarlyItem>,
    /// Datapackage de chaque jeu, `checksum` recalculé par Raftup.
    pub packages: IndexMap<String, GamePackage>,
}

/// Drapeaux `ItemClassification` → classification de `logic`. Les autres bits
/// (par exemple `deprioritized`) ne changent rien à la logique : ignorés.
fn classification(bits: u32) -> Classification {
    Classification {
        progression: bits & 0b0001 != 0,
        useful: bits & 0b0010 != 0,
        trap: bits & 0b0100 != 0,
        skip_balancing: bits & 0b1000 != 0,
    }
}

/// Tables de noms, pour passer d'un nom à un index.
struct Names {
    slots: Vec<SlotId>,
    items: BTreeMap<(u32, String), ItemKindId>,
    regions: BTreeMap<(u32, String), RegionId>,
    entrances: BTreeMap<(u32, String), EntranceId>,
    locations: BTreeMap<(u32, String), LocationId>,
    /// Compteur des groupes créés pour `has_from_list`, par joueur.
    lists: BTreeMap<u32, u32>,
}

impl Names {
    fn slot(&self, player: u32) -> Result<SlotId, ImportError> {
        player
            .checked_sub(1)
            .and_then(|i| self.slots.get(i as usize))
            .copied()
            .ok_or_else(|| invalid(format!("joueur {player} inconnu")))
    }

    fn item(&self, player: u32, name: &str) -> Result<ItemKindId, ImportError> {
        self.items
            .get(&(player, name.to_string()))
            .copied()
            .ok_or_else(|| invalid(format!("objet « {name} » du joueur {player} inconnu")))
    }

    fn region(&self, player: u32, name: &str) -> Result<RegionId, ImportError> {
        self.regions
            .get(&(player, name.to_string()))
            .copied()
            .ok_or_else(|| invalid(format!("région « {name} » du joueur {player} inconnue")))
    }
}

/// Une ligne du placement renvoyé à Archipelago :
/// `(joueur du lieu, lieu, joueur de l'objet, objet)`, joueurs numérotés à partir de 1.
pub type PlacementLine = (u32, String, u32, String);

/// Remplit un monde importé (placement puis équilibrage de progression, ADR 0027) et rend
/// le placement des lieux qui étaient vides, à renvoyer au worker (`finish_worlds`).
///
/// `seed` : le numéro de seed de la génération ; même seed, même export ⇒ même placement.
pub fn fill_imported(
    imported: &mut Imported,
    export: &Export,
    seed: u64,
) -> Result<Vec<PlacementLine>, ImportError> {
    fill_imported_with(imported, export, seed, &fill::TimeBalance::default())
        .map(|(lines, _)| lines)
}

/// Comme [`fill_imported`], avec l'équilibrage par temps de jeu (ADR 0030) après
/// l'équilibrage de progression s'il est activé ; rend aussi ce qu'il a fait (`None` s'il
/// est désactivé).
pub fn fill_imported_with(
    imported: &mut Imported,
    export: &Export,
    seed: u64,
    time_balance: &fill::TimeBalance,
) -> Result<(Vec<PlacementLine>, Option<fill::TimeBalanceReport>), ImportError> {
    let world = &mut imported.world;
    let empty: Vec<LocationId> = world
        .location_ids()
        .filter(|id| world.location(*id).is_some_and(|l| l.item.is_none()))
        .collect();
    let mut rng = crate::stream("fill", seed, 0);
    fill::fill_with_early(
        world,
        imported.pool.clone(),
        &imported.start,
        &imported.early,
        &mut rng,
    )?;
    let balancing: Vec<u8> = export
        .players
        .iter()
        .map(|p| p.progression_balancing)
        .collect();
    fill::balance(world, &balancing, &imported.start, &mut rng)?;
    let report = crate::run_time_balance(world, time_balance, &imported.start)?;

    let mut lines = Vec::with_capacity(empty.len());
    for id in empty {
        let location = world
            .location(id)
            .ok_or_else(|| invalid("lieu disparu après le placement"))?;
        let item = location
            .item
            .and_then(|i| world.item_kind(i))
            .ok_or_else(|| invalid(format!("lieu « {} » resté vide", location.name)))?;
        lines.push((
            location.slot.raw() + 1,
            location.name.clone(),
            item.slot.raw() + 1,
            item.name.clone(),
        ));
    }
    Ok((lines, report))
}

/// Construit le monde décrit par l'export.
pub fn build_world(export: &Export) -> Result<Imported, ImportError> {
    check_supported(export)?;
    let mut world = MultiWorld::new();
    let mut names = Names {
        slots: Vec::new(),
        items: BTreeMap::new(),
        regions: BTreeMap::new(),
        entrances: BTreeMap::new(),
        locations: BTreeMap::new(),
        lists: BTreeMap::new(),
    };

    // Joueurs : numérotés 1, 2, 3… dans l'ordre.
    for (index, player) in export.players.iter().enumerate() {
        if usize::try_from(player.slot).ok() != Some(index + 1) {
            return Err(invalid(format!("slot {} hors d'ordre", player.slot)));
        }
        names
            .slots
            .push(world.add_slot(player.name.clone(), player.game.clone())?);
    }

    add_item_kinds(export, &mut world, &mut names)?;
    let item_rules = add_item_rules(export, &mut world, &names)?;
    for player in &export.players {
        // `Options.Accessibility` d'Archipelago : 0 full, 1 items, 2 minimal.
        let accessibility = match player.accessibility {
            0 => Accessibility::Full,
            1 => Accessibility::Items,
            2 => Accessibility::Minimal,
            other => {
                return Err(invalid(format!(
                    "{} : accessibility {other} inconnue",
                    player.name
                )))
            }
        };
        world.set_accessibility(names.slot(player.slot)?, accessibility)?;
    }

    // Régions, puis entrées et lieux avec une règle provisoire : une règle peut citer un
    // lieu créé plus tard (`CanReachLocation`), on pose donc les règles à la fin.
    for player in &export.players {
        let slot = names.slot(player.slot)?;
        for region in &player.regions {
            let id = world.add_region(slot, region.clone())?;
            names.regions.insert((player.slot, region.clone()), id);
        }
        let origin = names.region(player.slot, &player.origin)?;
        world.set_origin(slot, origin)?;
    }
    for player in &export.players {
        for entrance in &player.entrances {
            let from = names.region(player.slot, &entrance.from)?;
            let to_name = entrance
                .to
                .as_deref()
                .ok_or_else(|| invalid(format!("entrée « {} » reliée à rien", entrance.name)))?;
            let to = names.region(player.slot, to_name)?;
            let id = world.add_entrance(from, to, entrance.name.clone(), Rule::False)?;
            names
                .entrances
                .insert((player.slot, entrance.name.clone()), id);
        }
        for location in &player.locations {
            let region = names.region(player.slot, &location.region)?;
            let id =
                world.add_location(region, location.name.clone(), location.address, Rule::False)?;
            names
                .locations
                .insert((player.slot, location.name.clone()), id);
            // `LocationProgressType` d'Archipelago : 1 normal, 2 prioritaire, 3 exclu.
            let progress = match location.progress_type {
                1 => Progress::Default,
                2 => Progress::Priority,
                3 => Progress::Excluded,
                other => {
                    return Err(invalid(format!(
                        "lieu « {} » : progress_type {other} inconnu",
                        location.name
                    )))
                }
            };
            world.set_progress(id, progress)?;
            if let Some(index) = location.item_rule {
                let rule = item_rules.get(index as usize).copied().ok_or_else(|| {
                    invalid(format!(
                        "lieu « {} » : règle d'objet n° {index} inconnue",
                        location.name
                    ))
                })?;
                world.set_item_rule(id, Some(rule))?;
            }
            if let Some(item) = &location.item {
                world.place(id, names.item(item.player, &item.name)?)?;
            }
        }
    }

    // Les règles, maintenant que tout existe.
    for player in &export.players {
        for entrance in &player.entrances {
            let id = lookup(&names.entrances, player.slot, &entrance.name)?;
            let rule = convert(&entrance.rule, &mut world, &mut names)?;
            world.set_entrance_rule(id, rule)?;
        }
        for location in &player.locations {
            let id = lookup(&names.locations, player.slot, &location.name)?;
            let rule = convert(&location.rule, &mut world, &mut names)?;
            world.set_location_rule(id, rule)?;
        }
        let goal = convert(&player.goal, &mut world, &mut names)?;
        world.set_goal(names.slot(player.slot)?, goal)?;
    }

    let pool = export
        .pool
        .iter()
        .map(|item| names.item(item.player, &item.name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut start_items = Vec::new();
    for player in &export.players {
        let items = player
            .precollected
            .iter()
            .map(|item| names.item(item.player, &item.name))
            .collect::<Result<Vec<_>, _>>()?;
        start_items.push(items);
    }
    let mut start = State::new(&world);
    for item in start_items.iter().flatten() {
        start.collect(&world, *item);
    }

    let early = early_items(export, &names)?;

    world.validate()?;
    if let Some(checks) = &export.checks {
        cross_check(export, checks, &world, &names)?;
        cross_check_items(export, checks, &world, &names)?;
    }
    let packages = packages(export)?;
    Ok(Imported {
        world,
        pool,
        start,
        start_items,
        early,
        packages,
    })
}

/// Range les règles d'objet de l'export dans le monde, dans l'ordre ; rend leurs index.
fn add_item_rules(
    export: &Export,
    world: &mut MultiWorld,
    names: &Names,
) -> Result<Vec<ItemRuleId>, ImportError> {
    let ids = |list: &[(u32, String)]| {
        list.iter()
            .map(|(player, name)| names.item(*player, name))
            .collect::<Result<Vec<_>, _>>()
    };
    let mut out = Vec::with_capacity(export.item_rules.len());
    for rule in &export.item_rules {
        let list = ids(&rule.list)?;
        let always = ids(&rule.always)?;
        let rule = if rule.allow_only {
            ItemRule::allow_only(list, always)
        } else {
            ItemRule::forbid(list, always)
        };
        out.push(world.add_item_rule(rule)?);
    }
    Ok(out)
}

/// Objets à placer tôt de chaque joueur. Un nom absent du pool est ignoré, comme dans
/// Archipelago (il n'y a aucun exemplaire à placer).
fn early_items(export: &Export, names: &Names) -> Result<Vec<fill::EarlyItem>, ImportError> {
    let mut out = Vec::new();
    for player in &export.players {
        let lists = [
            (&player.early_items, false),
            (&player.local_early_items, true),
        ];
        for (list, local) in lists {
            // `BTreeMap` : parcours trié par nom, donc reproductible.
            for (name, count) in list {
                if *count == 0 {
                    continue;
                }
                let Some(item) = names.items.get(&(player.slot, name.clone())) else {
                    continue;
                };
                out.push(fill::EarlyItem {
                    item: *item,
                    count: *count,
                    local,
                });
            }
        }
    }
    Ok(out)
}

/// Nombre maximal de paires (lieu, objet) lues pour le contrôle des règles d'objet.
pub const MAX_ITEM_CHECKS: usize = 4096;

/// Compare chaque paire (lieu, objet) contrôlée par Archipelago (`can_fill` sans l'accès)
/// avec [`MultiWorld::can_hold`] ; au moindre désaccord, le monde reste en Python.
fn cross_check_items(
    export: &Export,
    checks: &Checks,
    world: &MultiWorld,
    names: &Names,
) -> Result<(), ImportError> {
    if checks.item_checks.len() > MAX_ITEM_CHECKS {
        return Err(invalid(format!(
            "{} contrôles de règles d'objet",
            checks.item_checks.len()
        )));
    }
    for (player, location, item_player, item, result) in &checks.item_checks {
        let loc = lookup(&names.locations, *player, location)?;
        let kind = names.item(*item_player, item)?;
        let ours = world.can_hold(loc, kind);
        let agree = match result.as_str() {
            "1" => ours,
            "0" => !ours,
            _ => false,
        };
        if !agree {
            let owner = export
                .players
                .iter()
                .find(|p| p.slot == *player)
                .ok_or_else(|| invalid(format!("joueur {player} inconnu")))?;
            let what = if result == "e" {
                "la règle Python plante".to_string()
            } else {
                format!("Raftup dit {ours}, Archipelago le contraire")
            };
            return Err(ImportError::Unsupported {
                player: owner.name.clone(),
                game: owner.game.clone(),
                reason: format!(
                    "contrôle croisé de la règle d'objet du lieu « {location} » pour « {item} » : {what}"
                ),
            });
        }
    }
    Ok(())
}

/// Index d'un élément créé plus haut (toujours présent ; erreur plutôt que panique).
fn lookup<T: Copy>(
    table: &BTreeMap<(u32, String), T>,
    player: u32,
    name: &str,
) -> Result<T, ImportError> {
    table
        .get(&(player, name.to_string()))
        .copied()
        .ok_or_else(|| invalid(format!("« {name} » du joueur {player} introuvable")))
}

/// Refuse ce que le moteur Rust ne sait pas encore faire (le monde reste en Python).
fn check_supported(export: &Export) -> Result<(), ImportError> {
    for player in &export.players {
        let refuse = |reason: String| ImportError::Unsupported {
            player: player.name.clone(),
            game: player.game.clone(),
            reason,
        };
        let rules = player
            .entrances
            .iter()
            .map(|e| (&e.name, &e.rule))
            .chain(player.locations.iter().map(|l| (&l.name, &l.rule)));
        for (name, rule) in rules {
            if let Some(why) = rule.first_opaque() {
                return Err(refuse(format!(
                    "règle Python non traduite pour « {name} » ({why})"
                )));
            }
        }
        if let Some(why) = player.goal.first_opaque() {
            return Err(refuse(format!(
                "condition de victoire non traduite ({why})"
            )));
        }
        if player.custom_collect {
            return Err(refuse(
                "le monde change la façon de ramasser les objets (collect)".to_string(),
            ));
        }
        if let Some(why) = &player.item_rule_error {
            return Err(refuse(format!("règle d'objet non traduite ({why})")));
        }
    }
    Ok(())
}

/// Crée une sorte d'objet par (joueur, nom) : objets du pool, de départ, déjà posés,
/// puis ceux que citent seulement les règles (des événements jamais posés, en général).
fn add_item_kinds(
    export: &Export,
    world: &mut MultiWorld,
    names: &mut Names,
) -> Result<(), ImportError> {
    // Chaque exemplaire, dans l'ordre de l'export : l'ordre des sortes reste reproductible.
    let mut instances: Vec<&ItemExport> = export.pool.iter().collect();
    for player in &export.players {
        instances.extend(player.precollected.iter());
        instances.extend(player.locations.iter().filter_map(|l| l.item.as_ref()));
    }
    // La classification de chaque sorte : tous les exemplaires doivent être d'accord.
    let mut seen: BTreeMap<(u32, String), u32> = BTreeMap::new();
    for item in instances {
        let key = (item.player, item.name.clone());
        match seen.get(&key) {
            Some(bits) if classification(*bits) != classification(item.classification) => {
                let player = export
                    .players
                    .iter()
                    .find(|p| p.slot == item.player)
                    .ok_or_else(|| invalid(format!("joueur {} inconnu", item.player)))?;
                return Err(ImportError::Unsupported {
                    player: player.name.clone(),
                    game: player.game.clone(),
                    reason: format!("« {} » a plusieurs classifications", item.name),
                });
            }
            Some(_) => {}
            None => {
                seen.insert(key.clone(), item.classification);
                let slot = names.slot(item.player)?;
                let id = world.add_item_kind(
                    slot,
                    item.name.clone(),
                    item.code,
                    classification(item.classification),
                )?;
                names.items.insert(key, id);
            }
        }
    }
    // Objets cités par une règle sans exemplaire : progression (une règle les attend).
    let mut cited = Vec::new();
    for player in &export.players {
        for entrance in &player.entrances {
            entrance.rule.items(&mut cited);
        }
        for location in &player.locations {
            location.rule.items(&mut cited);
        }
        player.goal.items(&mut cited);
    }
    for (player, name) in cited {
        let key = (player, name.clone());
        if names.items.contains_key(&key) {
            continue;
        }
        let slot = names.slot(player)?;
        let id = world.add_item_kind(slot, name, None, Classification::PROGRESSION)?;
        names.items.insert(key, id);
    }
    Ok(())
}

/// Traduit une règle exportée en règle de `logic`.
fn convert(
    rule: &RuleExport,
    world: &mut MultiWorld,
    names: &mut Names,
) -> Result<Rule, ImportError> {
    Ok(match rule {
        RuleExport::True => Rule::True,
        RuleExport::False => Rule::False,
        RuleExport::Opaque { why } => return Err(invalid(format!("règle opaque ({why})"))),
        RuleExport::And { of } => Rule::And(convert_all(of, world, names)?),
        RuleExport::Or { of } => Rule::Or(convert_all(of, world, names)?),
        RuleExport::AtLeast { count, of } => {
            let children = convert_all(of, world, names)?;
            at_least(*count, children)?
        }
        RuleExport::Has {
            player,
            item,
            count,
        } => {
            if *count == 0 {
                Rule::True
            } else {
                Rule::has_count(names.item(*player, item)?, *count)
            }
        }
        RuleExport::HasAll { player, items } => Rule::HasAll(item_ids(*player, items, names)?),
        RuleExport::HasAny { player, items } => Rule::HasAny(item_ids(*player, items, names)?),
        RuleExport::HasAllCounts { player, items } => Rule::HasAllCounts(
            items
                .iter()
                .map(|(name, count)| Ok((names.item(*player, name)?, *count)))
                .collect::<Result<Vec<_>, ImportError>>()?,
        ),
        RuleExport::HasAnyCount { player, items } => Rule::Or(
            items
                .iter()
                .map(|(name, count)| Ok(Rule::has_count(names.item(*player, name)?, *count)))
                .collect::<Result<Vec<_>, ImportError>>()?,
        ),
        RuleExport::HasFromList {
            player,
            items,
            count,
        } => {
            if *count == 0 {
                Rule::True
            } else {
                // Somme des exemplaires = règle de groupe : on crée un groupe anonyme.
                let members = item_ids(*player, items, names)?;
                let number = names.lists.entry(*player).or_insert(0);
                *number += 1;
                let group_name = format!("raftup:liste {number}");
                let group = world.add_item_group(names.slot(*player)?, group_name, members)?;
                Rule::HasGroup {
                    group,
                    count: *count,
                }
            }
        }
        RuleExport::HasFromListUnique {
            player,
            items,
            count,
        } => {
            // Objets différents : au moins `count` des règles « a cet objet ».
            let mut distinct = item_ids(*player, items, names)?;
            distinct.sort();
            distinct.dedup();
            at_least(*count, distinct.into_iter().map(Rule::has).collect())?
        }
        RuleExport::CanReachLocation { player, location } => Rule::CanReachLocation(
            names
                .locations
                .get(&(*player, location.clone()))
                .copied()
                .ok_or_else(|| invalid(format!("lieu « {location} » inconnu")))?,
        ),
        RuleExport::CanReachRegion { player, region } => {
            Rule::CanReachRegion(names.region(*player, region)?)
        }
        RuleExport::CanReachEntrance { player, entrance } => Rule::CanReachEntrance(
            names
                .entrances
                .get(&(*player, entrance.clone()))
                .copied()
                .ok_or_else(|| invalid(format!("entrée « {entrance} » inconnue")))?,
        ),
    })
}

fn convert_all(
    rules: &[RuleExport],
    world: &mut MultiWorld,
    names: &mut Names,
) -> Result<Vec<Rule>, ImportError> {
    rules.iter().map(|r| convert(r, world, names)).collect()
}

fn item_ids(player: u32, items: &[String], names: &Names) -> Result<Vec<ItemKindId>, ImportError> {
    items.iter().map(|name| names.item(player, name)).collect()
}

/// « Au moins `count` des règles », sans opérateur dédié dans `logic` : un `Or` de `And`
/// sur toutes les combinaisons de `count` règles (plafonné à [`MAX_COMBINATIONS`]).
fn at_least(count: u32, rules: Vec<Rule>) -> Result<Rule, ImportError> {
    let n = rules.len();
    let k = count as usize;
    if k == 0 {
        return Ok(Rule::True);
    }
    if k > n {
        return Ok(Rule::False);
    }
    if k == 1 {
        return Ok(Rule::Or(rules));
    }
    if k == n {
        return Ok(Rule::And(rules));
    }
    if binomial(n, k) > MAX_COMBINATIONS {
        return Err(invalid(format!(
            "« au moins {k} parmi {n} » : trop de combinaisons"
        )));
    }
    let mut out = Vec::new();
    let mut chosen = Vec::with_capacity(k);
    combinations(&rules, k, 0, &mut chosen, &mut out);
    Ok(Rule::Or(out))
}

/// C(n, k), plafonné (on s'arrête dès qu'on dépasse le plafond).
fn binomial(n: usize, k: usize) -> usize {
    let mut value: usize = 1;
    for i in 0..k {
        value = value.saturating_mul(n - i) / (i + 1);
        if value > MAX_COMBINATIONS {
            return value;
        }
    }
    value
}

/// Toutes les façons de choisir `k` règles parmi `rules[start..]`, dans l'ordre.
fn combinations(
    rules: &[Rule],
    k: usize,
    start: usize,
    chosen: &mut Vec<usize>,
    out: &mut Vec<Rule>,
) {
    if chosen.len() == k {
        let all = chosen
            .iter()
            .filter_map(|i| rules.get(*i).cloned())
            .collect();
        out.push(Rule::And(all));
        return;
    }
    for i in start..rules.len() {
        chosen.push(i);
        combinations(rules, k, i + 1, chosen, out);
        chosen.pop();
    }
}

/// Nombre maximal d'inventaires lus pour le contrôle croisé.
pub const MAX_SAMPLES: usize = 256;

/// Recalcule chaque règle contrôlée sur chaque inventaire et compare au vrai Archipelago.
fn cross_check(
    export: &Export,
    checks: &Checks,
    world: &MultiWorld,
    names: &Names,
) -> Result<(), ImportError> {
    if checks.samples.len() > MAX_SAMPLES {
        return Err(invalid(format!(
            "{} inventaires de contrôle",
            checks.samples.len()
        )));
    }
    // Chaque règle contrôlée, retrouvée dans le monde construit.
    let mut rules = Vec::new();
    for checked in &checks.rules {
        let rule = match checked.kind.as_str() {
            "entrance" => {
                let id = lookup(&names.entrances, checked.player, &checked.name)?;
                world.entrance(id).map(|e| &e.rule)
            }
            "location" => {
                let id = lookup(&names.locations, checked.player, &checked.name)?;
                world.location(id).map(|l| &l.rule)
            }
            "goal" => world.slot(names.slot(checked.player)?).map(|s| &s.goal),
            other => return Err(invalid(format!("contrôle de genre « {other} »"))),
        };
        let rule = rule.ok_or_else(|| invalid(format!("« {} » introuvable", checked.name)))?;
        if checked.results.len() != checks.samples.len() {
            return Err(invalid(format!(
                "contrôle de « {} » : {} résultats pour {} inventaires",
                checked.name,
                checked.results.len(),
                checks.samples.len()
            )));
        }
        rules.push((checked, rule));
    }
    for (index, sample) in checks.samples.iter().enumerate() {
        let mut state = State::new(world);
        for (player, item, count) in sample {
            // Un objet qu'aucune règle ne cite n'a pas de sorte dans le monde : sans effet.
            if let Some(id) = names.items.get(&(*player, item.clone())) {
                state.add(*id, *count);
            }
        }
        let reach = Reachability::compute(world, &state);
        for (checked, rule) in &rules {
            let expected = checked.results.as_bytes().get(index).copied();
            let ours = rule.eval(world, &state, &reach);
            let agree = match expected {
                Some(b'1') => ours,
                Some(b'0') => !ours,
                _ => false,
            };
            if !agree {
                let player = export
                    .players
                    .iter()
                    .find(|p| p.slot == checked.player)
                    .ok_or_else(|| invalid(format!("joueur {} inconnu", checked.player)))?;
                let what = if expected == Some(b'e') {
                    "la règle Python plante".to_string()
                } else {
                    format!("Raftup dit {ours}, Archipelago le contraire")
                };
                return Err(ImportError::Unsupported {
                    player: player.name.clone(),
                    game: player.game.clone(),
                    reason: format!(
                        "contrôle croisé de « {} » ({}) sur l'inventaire n° {index} : {what}",
                        checked.name, checked.kind
                    ),
                });
            }
        }
    }
    Ok(())
}

/// Datapackages, `checksum` recalculé par Raftup et comparé à celui de Python.
fn packages(export: &Export) -> Result<IndexMap<String, GamePackage>, ImportError> {
    let mut out = IndexMap::new();
    for (game, package) in &export.datapackage {
        let items: Vec<(&str, i64)> = package
            .item_name_to_id
            .iter()
            .map(|(n, id)| (n.as_str(), *id))
            .collect();
        let locations: Vec<(&str, i64)> = package
            .location_name_to_id
            .iter()
            .map(|(n, id)| (n.as_str(), *id))
            .collect();
        let item_members: Vec<(&str, Vec<&str>)> = package
            .item_name_groups
            .iter()
            .filter(|(name, _)| name.as_str() != "Everything")
            .map(|(name, members)| (name.as_str(), members.iter().map(String::as_str).collect()))
            .collect();
        let location_members: Vec<(&str, Vec<&str>)> = package
            .location_name_groups
            .iter()
            .filter(|(name, _)| name.as_str() != "Everywhere")
            .map(|(name, members)| (name.as_str(), members.iter().map(String::as_str).collect()))
            .collect();
        // `PackageSource` attend des tranches de tranches : on emprunte les `Vec` ci-dessus.
        let item_groups: Vec<(&str, &[&str])> = item_members
            .iter()
            .map(|(n, m)| (*n, m.as_slice()))
            .collect();
        let location_groups: Vec<(&str, &[&str])> = location_members
            .iter()
            .map(|(n, m)| (*n, m.as_slice()))
            .collect();
        let built = game_package(&PackageSource {
            items: &items,
            locations: &locations,
            item_groups: &item_groups,
            location_groups: &location_groups,
        });
        if built.checksum != package.checksum {
            return Err(invalid(format!(
                "checksum du datapackage de « {game} » : Raftup {} ≠ Archipelago {}",
                built.checksum, package.checksum
            )));
        }
        out.insert(game.clone(), built);
    }
    Ok(out)
}
