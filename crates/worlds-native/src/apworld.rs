//! Export inverse : un monde natif devient un apworld Python (ADR 0042, § 4).
//!
//! Règle du projet : aucun monde exclusif à Raftup. Tout monde natif doit donc pouvoir
//! tourner dans Archipelago. Ce module écrit les fichiers Python d'un apworld minimal ;
//! l'appelant les met dans un zip (`<module>/<fichier>`) pour en faire un `.apworld`.
//!
//! Choix d'écriture :
//!
//! - les règles sont des **fonctions Python** ordinaires (`state.has(...)`), lisibles et
//!   comprises par toutes les versions 0.6 d'Archipelago ; les parties qui ne dépendent que
//!   des options lisent `world.options` au moment de l'appel ;
//! - `when` et listes de cas deviennent des `if` ;
//! - **aucun texte du fichier n'est recopié comme du code** : chaque nom est écrit comme
//!   une chaîne Python échappée ([`py_str`]), les noms d'options sont déjà limités à
//!   `[A-Za-z0-9_]` par la lecture du monde. Un monde hostile ne peut pas injecter de Python ;
//! - le texte est déterministe : même monde ⇒ mêmes fichiers.

use std::fmt::Write as _;

use logic::Classification;
use thiserror::Error;

use crate::model::{
    Choice, Count, Expr, ItemRef, LocationRef, NativeWorld, OptionKind, SlotDataValue,
};

/// Réglages de l'apworld produit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApworldConfig {
    /// Nom du module Python (dossier dans le zip) : `[a-z_][a-z0-9_]*`.
    pub module: String,
    /// `world_version` du manifeste `archipelago.json`.
    pub world_version: String,
    /// Auteurs, pour le manifeste.
    pub authors: Vec<String>,
}

impl ApworldConfig {
    /// Réglages par défaut : module tiré du nom du jeu, version 0.1.0.
    pub fn for_world(world: &NativeWorld) -> ApworldConfig {
        ApworldConfig {
            module: module_name(&world.game),
            world_version: "0.1.0".into(),
            authors: Vec::new(),
        }
    }
}

/// Un fichier de l'apworld : chemin relatif au dossier du module, contenu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApworldFile {
    pub name: String,
    pub contents: String,
}

/// Ce qui empêche d'écrire l'apworld.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApworldError {
    #[error("nom de module « {0} » invalide (attendu : lettres minuscules, chiffres, _)")]
    BadModule(String),
    #[error("option « {option} » : {message}")]
    BadOption { option: String, message: String },
    #[error("monde incohérent : {0}")]
    Invalid(String),
}

/// Nom de module Python tiré d'un nom de jeu : `APQuest` → `apquest`, `Slime Rancher` →
/// `slime_rancher`.
pub fn module_name(game: &str) -> String {
    let mut out = String::new();
    for c in game.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        format!("world_{out}")
    } else {
        out
    }
}

/// Écrit les fichiers de l'apworld, dans un ordre fixe.
pub fn to_apworld(
    world: &NativeWorld,
    config: &ApworldConfig,
) -> Result<Vec<ApworldFile>, ApworldError> {
    let valid_module = config
        .module
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && config
            .module
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid_module {
        return Err(ApworldError::BadModule(config.module.clone()));
    }
    let w = Writer { world };
    Ok(vec![
        file("archipelago.json", w.manifest(config)),
        file("__init__.py", w.init()),
        file("options.py", w.options()?),
        file("items.py", w.items()?),
        file("locations.py", w.locations()),
        file("regions.py", w.regions()?),
        file("rules.py", w.rules()?),
    ])
}

fn file(name: &str, contents: String) -> ApworldFile {
    ApworldFile {
        name: name.into(),
        contents,
    }
}

const HEADER: &str =
    "# Écrit par Raftup depuis un monde natif (ADR 0042). Ne pas modifier à la main :\n\
                      # modifier le world.toml et refaire l'export.\n";

struct Writer<'a> {
    world: &'a NativeWorld,
}

impl Writer<'_> {
    fn manifest(&self, config: &ApworldConfig) -> String {
        // `serde_json` échappe correctement les chaînes du manifeste.
        let value = serde_json::json!({
            "game": self.world.game,
            "minimum_ap_version": "0.6.7",
            "world_version": config.world_version,
            "authors": config.authors,
        });
        let mut text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into());
        text.push('\n');
        text
    }

    fn init(&self) -> String {
        let mut out = String::from(HEADER);
        out.push_str(
            "from worlds.AutoWorld import World\n\n\
             from . import items, locations, options, regions, rules\n\n\n\
             class GeneratedWorld(World):\n",
        );
        let _ = writeln!(
            out,
            "    \"\"\"Monde natif {}.\"\"\"\n",
            py_doc(&self.world.game)
        );
        out.push_str(
            "    game = items.GAME\n\
             \x20   options_dataclass = options.GameOptions\n\
             \x20   options: options.GameOptions\n\
             \x20   item_name_to_id = items.ITEM_NAME_TO_ID\n\
             \x20   location_name_to_id = locations.LOCATION_NAME_TO_ID\n\
             \x20   item_name_groups = items.ITEM_NAME_GROUPS\n\
             \x20   location_name_groups = locations.LOCATION_NAME_GROUPS\n\n\
             \x20   def create_regions(self) -> None:\n\
             \x20       regions.create_regions(self)\n\n\
             \x20   def set_rules(self) -> None:\n\
             \x20       self.multiworld.completion_condition[self.player] = (\n\
             \x20           lambda state: rules.goal(self, state)\n\
             \x20       )\n\n\
             \x20   def create_items(self) -> None:\n\
             \x20       items.create_items(self)\n\n\
             \x20   def create_item(self, name: str) -> items.GameItem:\n\
             \x20       return items.create_item(self, name)\n\n\
             \x20   def get_filler_item_name(self) -> str:\n\
             \x20       return items.FILLER\n\n\
             \x20   def fill_slot_data(self):\n\
             \x20       return items.slot_data(self)\n",
        );
        out
    }

    fn options(&self) -> Result<String, ApworldError> {
        let mut out = String::from(HEADER);
        out.push_str("from dataclasses import dataclass\n\nfrom Options import Choice, PerGameCommonOptions, Range, Toggle\n\n");
        // Les classes sont créées par `type(...)` : les noms de choix deviennent des clés
        // de dictionnaire (chaînes échappées), jamais du code.
        for option in &self.world.options {
            let class = option_class(&option.name);
            let (base, mut attrs) = match &option.kind {
                OptionKind::Toggle => ("Toggle", vec![]),
                OptionKind::Range { min, max } => (
                    "Range",
                    vec![
                        ("range_start".to_string(), min.to_string()),
                        ("range_end".to_string(), max.to_string()),
                    ],
                ),
                OptionKind::Choice { choices, aliases } => {
                    let mut attrs = Vec::new();
                    for (i, choice) in choices.iter().enumerate() {
                        if choice.eq_ignore_ascii_case("random") {
                            return Err(ApworldError::BadOption {
                                option: option.name.clone(),
                                message: "le choix « random » est réservé par Archipelago".into(),
                            });
                        }
                        attrs.push((format!("option_{}", choice.to_lowercase()), i.to_string()));
                    }
                    for (alias, index) in aliases {
                        attrs.push((format!("alias_{}", alias.to_lowercase()), index.to_string()));
                    }
                    ("Choice", attrs)
                }
            };
            attrs.push(("default".into(), option.default.to_string()));
            let _ = writeln!(out, "{class} = type({}, ({base},), {{", py_str(&class));
            let _ = writeln!(out, "    \"__doc__\": {},", py_str(&option.display));
            let _ = writeln!(out, "    \"display_name\": {},", py_str(&option.display));
            for (key, value) in attrs {
                let _ = writeln!(out, "    {}: {value},", py_str(&key));
            }
            out.push_str("})\n\n");
        }
        out.push_str("\n@dataclass\nclass GameOptions(PerGameCommonOptions):\n");
        if self.world.options.is_empty() {
            out.push_str("    pass\n");
        }
        for option in &self.world.options {
            // Noms d'options déjà limités à [A-Za-z0-9_] par la lecture du monde.
            let _ = writeln!(out, "    {}: {}", option.name, option_class(&option.name));
        }
        Ok(out)
    }

    fn items(&self) -> Result<String, ApworldError> {
        let world = self.world;
        let mut out = String::from(HEADER);
        out.push_str("import json\n\nfrom BaseClasses import Item, ItemClassification\n\nfrom . import rules\n\n");
        let _ = writeln!(out, "GAME = {}\n", py_str(&world.game));
        out.push_str("\nclass GameItem(Item):\n    game = GAME\n\n\n");

        out.push_str("ITEM_NAME_TO_ID = {\n");
        for (name, id) in world.package_items() {
            let _ = writeln!(out, "    {}: {id},", py_str(name));
        }
        out.push_str("}\n\n");

        out.push_str("ITEM_NAME_GROUPS = {\n");
        for (name, members) in world.package_item_groups() {
            let members: Vec<String> = members.iter().map(|m| py_str(m)).collect();
            let _ = writeln!(
                out,
                "    {}: {{{}}},",
                py_str(name),
                if members.is_empty() {
                    String::new()
                } else {
                    members.join(", ")
                }
            );
        }
        out.push_str("}\n\n");

        // Remplissage par défaut : celui de [filler], sinon le premier objet « filler ».
        let filler = match &world.filler {
            Some(f) => world.items.get(f.item as usize).map(|i| i.name.as_str()),
            None => world
                .items
                .iter()
                .find(|i| {
                    i.classification
                        .cases
                        .iter()
                        .all(|(_, c)| *c == Classification::FILLER)
                })
                .or(world.items.first())
                .map(|i| i.name.as_str()),
        };
        let _ = writeln!(out, "FILLER = {}\n\n", py_str(filler.unwrap_or("Nothing")));

        // Classification : fixe dans une table, ou calculée selon les options.
        out.push_str("def classification(world, name):\n");
        out.push_str("    w = world\n");
        for item in &world.items {
            if item.classification.cases.len() > 1 {
                let _ = writeln!(out, "    if name == {}:", py_str(&item.name));
                self.choice(&mut out, 8, &item.classification, |c| {
                    format!("return ItemClassification({})", bits(*c))
                })?;
            }
        }
        out.push_str("    return ItemClassification(CLASSES.get(name, 1))\n\n\n");
        out.push_str("CLASSES = {\n");
        for item in &world.items {
            if let [(None, class)] = item.classification.cases.as_slice() {
                let _ = writeln!(out, "    {}: {},", py_str(&item.name), bits(*class));
            }
        }
        // Les objets d'événement sont de progression (1, la valeur par défaut).
        out.push_str("}\n\n\n");

        out.push_str(
            "def create_item(world, name):\n\
             \x20   return GameItem(name, classification(world, name), ITEM_NAME_TO_ID.get(name), world.player)\n\n\n",
        );

        out.push_str("def create_items(world):\n    w = world\n    pool = []\n");
        for amount in &world.pool {
            let name = self.item_name(amount.item)?;
            let count = self.count(amount.count)?;
            let indent = self.when(&mut out, 4, amount.when.as_ref())?;
            let _ = writeln!(
                out,
                "{indent}pool += [create_item(w, {}) for _ in range({count})]",
                py_str(name)
            );
        }
        out.push_str(
            "    unfilled = len(w.multiworld.get_unfilled_locations(w.player)) - len(pool)\n",
        );
        match &world.filler {
            Some(f) => {
                let filler = py_str(self.item_name(f.item)?);
                match f.trap {
                    Some((trap, percent)) => {
                        let trap = py_str(self.item_name(trap)?);
                        let percent = self.count(percent)?;
                        // `randint(0, 99) < pourcentage`, comme dans le format natif.
                        let _ = writeln!(
                            out,
                            "    for _ in range(unfilled):\n        \
                             trap = w.random.randint(0, 99) < {percent}\n        \
                             pool.append(create_item(w, {trap} if trap else {filler}))"
                        );
                    }
                    None => {
                        let _ = writeln!(
                            out,
                            "    pool += [create_item(w, {filler}) for _ in range(unfilled)]"
                        );
                    }
                }
            }
            None => out.push_str(
                "    if unfilled != 0:\n        \
                 raise Exception(\"le pool ne remplit pas exactement les lieux\")\n",
            ),
        }
        out.push_str("    w.multiworld.itempool += pool\n");
        for amount in &world.start {
            let name = self.item_name(amount.item)?;
            let count = self.count(amount.count)?;
            let indent = self.when(&mut out, 4, amount.when.as_ref())?;
            let _ = writeln!(
                out,
                "{indent}for _ in range({count}):\n{indent}    \
                 w.multiworld.push_precollected(create_item(w, {}))",
                py_str(name)
            );
        }
        out.push_str("\n\ndef slot_data(world):\n    w = world\n    return {\n");
        for entry in &world.slot_data {
            let value = match &entry.value {
                SlotDataValue::Option(o) => format!("{}.value", self.option_ref(*o)?),
                SlotDataValue::Literal(v) => {
                    // Valeur JSON relue par `json.loads` : aucune syntaxe Python à écrire.
                    let text = serde_json::to_string(v).unwrap_or_else(|_| "null".into());
                    format!("json.loads({})", py_str(&text))
                }
            };
            let _ = writeln!(out, "        {}: {value},", py_str(&entry.key));
        }
        out.push_str("    }\n");
        Ok(out)
    }

    fn locations(&self) -> String {
        let world = self.world;
        let mut out = String::from(HEADER);
        out.push_str("from BaseClasses import Location\n\nfrom .items import GAME\n\n\n");
        out.push_str("class GameLocation(Location):\n    game = GAME\n\n\n");
        out.push_str("LOCATION_NAME_TO_ID = {\n");
        for (name, id) in world.package_locations() {
            let _ = writeln!(out, "    {}: {id},", py_str(name));
        }
        out.push_str("}\n\nLOCATION_NAME_GROUPS = {\n");
        for (name, members) in world.package_location_groups() {
            let members: Vec<String> = members.iter().map(|m| py_str(m)).collect();
            let _ = writeln!(out, "    {}: {{{}}},", py_str(name), members.join(", "));
        }
        out.push_str("}\n");
        out
    }

    fn regions(&self) -> Result<String, ApworldError> {
        let world = self.world;
        let mut out = String::from(HEADER);
        out.push_str(
            "from BaseClasses import Item, ItemClassification, LocationProgressType, Region\n\n\
             from . import rules\n\
             from .items import GAME\n\
             from .locations import GameLocation\n\n\n\
             class EventItem(Item):\n    game = GAME\n\n\n\
             def create_regions(world):\n\
             \x20   w = world\n\
             \x20   regions = {}\n\n\
             \x20   def add_region(name):\n\
             \x20       region = Region(name, w.player, w.multiworld)\n\
             \x20       regions[name] = region\n\
             \x20       w.multiworld.regions.append(region)\n\n",
        );
        for region in &world.regions {
            let indent = self.when(&mut out, 4, region.when.as_ref())?;
            let _ = writeln!(out, "{indent}add_region({})", py_str(&region.name));
        }
        // Région de départ : celle du fichier, sinon la première présente.
        match world.start_region {
            Some(r) => {
                let _ = writeln!(
                    out,
                    "    w.origin_region_name = {}",
                    py_str(self.region_name(r)?)
                );
            }
            None => out.push_str("    w.origin_region_name = next(iter(regions))\n"),
        }
        out.push('\n');
        for (i, entrance) in world.entrances.iter().enumerate() {
            let indent = self.when(&mut out, 4, entrance.when.as_ref())?;
            let _ = writeln!(
                out,
                "{indent}regions[{}].connect(regions[{}], {}, lambda state: rules.entrance_{i}(w, state))",
                py_str(self.region_name(entrance.from)?),
                py_str(self.region_name(entrance.to)?),
                py_str(&entrance.name),
            );
        }
        out.push('\n');
        for (i, location) in world.locations.iter().enumerate() {
            let indent = self.when(&mut out, 4, location.when.as_ref())?;
            self.region_choice(&mut out, &indent, &location.region)?;
            let _ = writeln!(
                out,
                "{indent}location = GameLocation(w.player, {}, {}, region)",
                py_str(&location.name),
                location.id
            );
            let _ = writeln!(
                out,
                "{indent}location.access_rule = lambda state: rules.location_{i}(w, state)"
            );
            match location.progress {
                logic::Progress::Default => {}
                logic::Progress::Priority => {
                    let _ = writeln!(
                        out,
                        "{indent}location.progress_type = LocationProgressType.PRIORITY"
                    );
                }
                logic::Progress::Excluded => {
                    let _ = writeln!(
                        out,
                        "{indent}location.progress_type = LocationProgressType.EXCLUDED"
                    );
                }
            }
            let _ = writeln!(out, "{indent}region.locations.append(location)");
        }
        out.push('\n');
        for (i, event) in world.events.iter().enumerate() {
            let indent = self.when(&mut out, 4, event.when.as_ref())?;
            self.region_choice(&mut out, &indent, &event.region)?;
            let _ = writeln!(
                out,
                "{indent}event = GameLocation(w.player, {}, None, region)",
                py_str(&event.name)
            );
            let _ = writeln!(
                out,
                "{indent}event.access_rule = lambda state: rules.event_{i}(w, state)"
            );
            let _ = writeln!(
                out,
                "{indent}event.place_locked_item(EventItem({}, ItemClassification.progression, None, w.player))",
                py_str(&event.item)
            );
            let _ = writeln!(out, "{indent}region.locations.append(event)");
        }
        Ok(out)
    }

    fn rules(&self) -> Result<String, ApworldError> {
        let world = self.world;
        let mut out = String::from(HEADER);
        out.push_str(
            "# Chaque règle reçoit le monde (`w`, pour ses options et son joueur) et l'inventaire.\n\n\n\
             def _region(state, w, name):\n\
             \x20   try:\n\
             \x20       return state.can_reach_region(name, w.player)\n\
             \x20   except KeyError:  # région absente avec ces options\n\
             \x20       return False\n\n\n\
             def _location(state, w, name):\n\
             \x20   try:\n\
             \x20       return state.can_reach_location(name, w.player)\n\
             \x20   except KeyError:\n\
             \x20       return False\n\n\n\
             def _entrance(state, w, name):\n\
             \x20   try:\n\
             \x20       return state.can_reach_entrance(name, w.player)\n\
             \x20   except KeyError:\n\
             \x20       return False\n\n\n",
        );
        let rule = |out: &mut String, name: &str, expr: &Expr| -> Result<(), ApworldError> {
            let _ = writeln!(
                out,
                "def {name}(w, state):\n    return {}\n\n",
                self.expr(expr)?
            );
            Ok(())
        };
        rule(&mut out, "goal", &world.goal)?;
        for (i, e) in world.entrances.iter().enumerate() {
            rule(&mut out, &format!("entrance_{i}"), &e.rule)?;
        }
        for (i, l) in world.locations.iter().enumerate() {
            rule(&mut out, &format!("location_{i}"), &l.rule)?;
        }
        for (i, e) in world.events.iter().enumerate() {
            rule(&mut out, &format!("event_{i}"), &e.rule)?;
        }
        while out.ends_with("\n\n") {
            out.pop();
        }
        Ok(out)
    }

    // ---- Morceaux ----

    /// Écrit `if <condition>:` si besoin et rend l'indentation des lignes suivantes.
    fn when(
        &self,
        out: &mut String,
        indent: usize,
        when: Option<&Expr>,
    ) -> Result<String, ApworldError> {
        let pad = " ".repeat(indent);
        match when {
            None => Ok(pad),
            Some(condition) => {
                let _ = writeln!(out, "{pad}if {}:", self.expr(condition)?);
                Ok(" ".repeat(indent + 4))
            }
        }
    }

    /// Une valeur qui dépend des options : `if … / elif … / else`.
    fn choice<T>(
        &self,
        out: &mut String,
        indent: usize,
        choice: &Choice<T>,
        line: impl Fn(&T) -> String,
    ) -> Result<(), ApworldError> {
        let pad = " ".repeat(indent);
        for (i, (condition, value)) in choice.cases.iter().enumerate() {
            match condition {
                Some(c) => {
                    let keyword = if i == 0 { "if" } else { "elif" };
                    let _ = writeln!(out, "{pad}{keyword} {}:", self.expr(c)?);
                    let _ = writeln!(out, "{pad}    {}", line(value));
                }
                None if i == 0 => {
                    let _ = writeln!(out, "{pad}{}", line(value));
                }
                None => {
                    let _ = writeln!(out, "{pad}else:\n{pad}    {}", line(value));
                }
            }
        }
        Ok(())
    }

    fn region_choice(
        &self,
        out: &mut String,
        indent: &str,
        region: &Choice<u32>,
    ) -> Result<(), ApworldError> {
        let mut names = Vec::new();
        for (_, r) in &region.cases {
            names.push(self.region_name(*r)?.to_string());
        }
        let mut cases = region.cases.iter().zip(names);
        let assign = |name: &str| format!("region = regions[{}]", py_str(name));
        if region.cases.len() == 1 {
            if let Some((_, name)) = cases.next() {
                let _ = writeln!(out, "{indent}{}", assign(&name));
            }
            return Ok(());
        }
        for (i, ((condition, _), name)) in cases.enumerate() {
            match condition {
                Some(c) => {
                    let keyword = if i == 0 { "if" } else { "elif" };
                    let _ = writeln!(out, "{indent}{keyword} {}:", self.expr(c)?);
                    let _ = writeln!(out, "{indent}    {}", assign(&name));
                }
                None => {
                    let _ = writeln!(out, "{indent}else:\n{indent}    {}", assign(&name));
                }
            }
        }
        Ok(())
    }

    /// Une expression du mini-langage en Python.
    fn expr(&self, expr: &Expr) -> Result<String, ApworldError> {
        Ok(match expr {
            Expr::Const(true) => "True".into(),
            Expr::Const(false) => "False".into(),
            Expr::Has { item, count } => format!(
                "state.has({}, w.player, {})",
                py_str(self.item_ref(*item)?),
                self.count(*count)?
            ),
            Expr::HasAll(items) => format!("state.has_all({}, w.player)", self.item_tuple(items)?),
            Expr::HasAny(items) => format!("state.has_any({}, w.player)", self.item_tuple(items)?),
            Expr::HasGroup { group, count } => {
                // Somme des exemplaires des membres, objets d'événement compris : c'est
                // le sens du format natif (le groupe Python, lui, n'a pas les événements).
                let members = &self
                    .world
                    .item_groups
                    .get(*group as usize)
                    .ok_or_else(|| ApworldError::Invalid(format!("groupe n° {group}")))?
                    .members;
                format!(
                    "state.has_from_list({}, w.player, {})",
                    self.item_tuple(members)?,
                    self.count(*count)?
                )
            }
            Expr::CanReachRegion(r) => {
                format!("_region(state, w, {})", py_str(self.region_name(*r)?))
            }
            Expr::CanReachLocation(l) => {
                format!("_location(state, w, {})", py_str(self.location_ref(*l)?))
            }
            Expr::CanReachEntrance(e) => {
                let name = &self
                    .world
                    .entrances
                    .get(*e as usize)
                    .ok_or_else(|| ApworldError::Invalid(format!("entrée n° {e}")))?
                    .name;
                format!("_entrance(state, w, {})", py_str(name))
            }
            Expr::Toggle(o) => format!("bool({}.value)", self.option_ref(*o)?),
            Expr::Compare { option, op, value } => {
                format!(
                    "({}.value {} {value})",
                    self.option_ref(*option)?,
                    op.symbol()
                )
            }
            Expr::Not(inner) => format!("(not {})", self.expr(inner)?),
            Expr::And(parts) => self.join(parts, " and ")?,
            Expr::Or(parts) => self.join(parts, " or ")?,
        })
    }

    fn join(&self, parts: &[Expr], sep: &str) -> Result<String, ApworldError> {
        let parts: Result<Vec<String>, ApworldError> = parts.iter().map(|p| self.expr(p)).collect();
        Ok(format!("({})", parts?.join(sep)))
    }

    fn item_tuple(&self, items: &[ItemRef]) -> Result<String, ApworldError> {
        let names: Result<Vec<String>, ApworldError> = items
            .iter()
            .map(|i| self.item_ref(*i).map(py_str))
            .collect();
        let names = names?;
        // `(a,)` : en Python, un tuple d'un seul élément demande une virgule.
        Ok(match names.len() {
            1 => format!("({},)", names[0]),
            _ => format!("({})", names.join(", ")),
        })
    }

    fn count(&self, count: Count) -> Result<String, ApworldError> {
        Ok(match count {
            Count::Fixed(n) => n.to_string(),
            Count::Option(o) => format!("{}.value", self.option_ref(o)?),
        })
    }

    fn option_ref(&self, index: u32) -> Result<String, ApworldError> {
        let option = self
            .world
            .options
            .get(index as usize)
            .ok_or_else(|| ApworldError::Invalid(format!("option n° {index}")))?;
        Ok(format!("w.options.{}", option.name))
    }

    fn item_name(&self, index: u32) -> Result<&str, ApworldError> {
        self.item_ref(ItemRef::Item(index))
    }

    fn item_ref(&self, item: ItemRef) -> Result<&str, ApworldError> {
        match item {
            ItemRef::Item(i) => self.world.items.get(i as usize).map(|i| i.name.as_str()),
            ItemRef::Event(e) => self.world.events.get(e as usize).map(|e| e.item.as_str()),
        }
        .ok_or_else(|| ApworldError::Invalid(format!("objet {item:?}")))
    }

    fn location_ref(&self, location: LocationRef) -> Result<&str, ApworldError> {
        match location {
            LocationRef::Location(l) => self
                .world
                .locations
                .get(l as usize)
                .map(|l| l.name.as_str()),
            LocationRef::Event(e) => self.world.events.get(e as usize).map(|e| e.name.as_str()),
        }
        .ok_or_else(|| ApworldError::Invalid(format!("lieu {location:?}")))
    }

    fn region_name(&self, index: u32) -> Result<&str, ApworldError> {
        self.world
            .regions
            .get(index as usize)
            .map(|r| r.name.as_str())
            .ok_or_else(|| ApworldError::Invalid(format!("région n° {index}")))
    }
}

/// Nom de la classe Python d'une option (le nom est déjà limité à `[A-Za-z0-9_]`).
fn option_class(name: &str) -> String {
    format!("Option_{name}")
}

/// Drapeaux `ItemClassification` d'Archipelago.
fn bits(c: Classification) -> u32 {
    u32::from(c.progression)
        | u32::from(c.useful) << 1
        | u32::from(c.trap) << 2
        | u32::from(c.skip_balancing) << 3
}

/// Une chaîne Python entre guillemets, échappée : seuls l'ASCII imprimable passe tel quel,
/// tout le reste devient `\xNN`, `\uNNNN` ou `\UNNNNNNNN`.
pub fn py_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            ' '..='~' => out.push(c),
            c if (c as u32) < 0x100 => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c if (c as u32) < 0x10000 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => {
                let _ = write!(out, "\\U{:08x}", c as u32);
            }
        }
    }
    out.push('"');
    out
}

/// Texte d'une docstring : sans guillemets triples ni barre oblique inverse.
fn py_doc(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '"' | '\\' => '\'',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaines_python_echappees() {
        assert_eq!(py_str("Sword"), "\"Sword\"");
        assert_eq!(py_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(py_str("é\n"), "\"\\xe9\\x0a\"");
        assert_eq!(py_str("€"), "\"\\u20ac\"");
        assert_eq!(py_str("🦆"), "\"\\U0001f986\"");
    }

    #[test]
    fn noms_de_module() {
        assert_eq!(module_name("APQuest"), "apquest");
        assert_eq!(module_name("Slime Rancher"), "slime_rancher");
        assert_eq!(module_name("  "), "world_");
        assert_eq!(module_name("2048"), "world_2048");
    }
}
