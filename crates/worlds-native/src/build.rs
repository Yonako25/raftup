//! Compilation d'un monde natif vers `logic`, avec les options d'un joueur.
//!
//! Les conditions `when` et les parties statiques des règles sont calculées ici ; les
//! règles qui restent ne dépendent que de l'inventaire. L'ordre de création suit celui du
//! fichier : objets, objets d'événement, groupes, régions, entrées, lieux, événements.

use logic::{
    EntranceId, ItemGroupId, ItemKindId, LocationId, MultiWorld, Progress, RegionId, Rule, SlotId,
};
use rand_core::Rng;
use serde_json::{Map, Value};

use crate::error::BuildError;
use crate::model::{
    Choice, Count, Expr, ItemRef, LocationRef, NativeWorld, OptionKind, SlotDataValue,
};
use crate::MAX_ELEMENTS;

/// Valeurs des options d'un joueur, dans l'ordre de [`NativeWorld::options`] :
/// 0/1 pour `toggle`, le nombre pour `range`, la position du choix pour `choice`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionValues {
    values: Vec<i64>,
}

impl OptionValues {
    /// Les valeurs, dans l'ordre des options.
    pub fn as_slice(&self) -> &[i64] {
        &self.values
    }
}

/// Ce que le monde d'un joueur ajoute au multiworld, en plus de ses régions et lieux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    /// Objets à placer, un élément par exemplaire.
    pub pool: Vec<ItemKindId>,
    /// Inventaire de départ.
    pub start: Vec<ItemKindId>,
    /// Le `slot_data` envoyé au client.
    pub slot_data: Value,
}

/// Régions atteignables depuis `origin` en suivant les arêtes `(de, vers)`, sans règles.
///
/// Parcours en largeur : on part de l'origine et on ajoute, tant qu'il y en a, les régions
/// où mène une entrée depuis une région déjà atteinte.
pub(crate) fn reachable_regions(
    count: usize,
    origin: u32,
    edges: impl Iterator<Item = (u32, u32)> + Clone,
) -> Vec<bool> {
    let mut reached = vec![false; count];
    let mut queue = vec![origin];
    if let Some(slot) = reached.get_mut(origin as usize) {
        *slot = true;
    }
    while let Some(region) = queue.pop() {
        for (from, to) in edges.clone() {
            if from == region {
                if let Some(slot) = reached.get_mut(to as usize) {
                    if !*slot {
                        *slot = true;
                        queue.push(to);
                    }
                }
            }
        }
    }
    reached
}

/// Calcule une expression statique avec les options du joueur.
fn eval_static(expr: &Expr, values: &[i64]) -> bool {
    let value = |option: u32| values.get(option as usize).copied().unwrap_or(0);
    match expr {
        Expr::Const(b) => *b,
        Expr::Toggle(option) => value(*option) != 0,
        Expr::Compare {
            option,
            op,
            value: right,
        } => op.apply(value(*option), *right),
        Expr::Not(inner) => !eval_static(inner, values),
        Expr::And(parts) => parts.iter().all(|p| eval_static(p, values)),
        Expr::Or(parts) => parts.iter().any(|p| eval_static(p, values)),
        // Jamais appelé sur une expression dynamique (la lecture le refuse).
        _ => false,
    }
}

/// Identifiants `logic` des éléments créés (`None` : élément absent avec ces options).
struct Ids<'a> {
    values: &'a [i64],
    items: Vec<ItemKindId>,
    event_items: Vec<ItemKindId>,
    groups: Vec<ItemGroupId>,
    regions: Vec<Option<RegionId>>,
    entrances: Vec<Option<EntranceId>>,
    locations: Vec<Option<LocationId>>,
    event_locations: Vec<Option<LocationId>>,
}

fn missing(what: &str) -> BuildError {
    BuildError::Invalid(format!("{what} introuvable (monde incohérent)"))
}

impl Ids<'_> {
    fn item(&self, item: ItemRef) -> Result<ItemKindId, BuildError> {
        let found = match item {
            ItemRef::Item(i) => self.items.get(i as usize),
            ItemRef::Event(e) => self.event_items.get(e as usize),
        };
        found.copied().ok_or_else(|| missing("objet"))
    }

    fn count(&self, count: Count) -> Result<u32, BuildError> {
        match count {
            Count::Fixed(n) => Ok(n),
            Count::Option(option) => {
                let value = self.values.get(option as usize).copied().unwrap_or(0);
                u32::try_from(value).map_err(|_| {
                    BuildError::Invalid(format!("nombre négatif ({value}) tiré d'une option"))
                })
            }
        }
    }

    /// Traduit une expression en règle `logic`, en simplifiant les constantes.
    fn lower(&self, expr: &Expr) -> Result<Rule, BuildError> {
        Ok(match expr {
            Expr::Const(_) | Expr::Toggle(_) | Expr::Compare { .. } | Expr::Not(_) => {
                if eval_static(expr, self.values) {
                    Rule::True
                } else {
                    Rule::False
                }
            }
            Expr::Has { item, count } => match self.count(*count)? {
                0 => Rule::True,
                count => Rule::Has {
                    item: self.item(*item)?,
                    count,
                },
            },
            Expr::HasAll(items) => Rule::HasAll(self.items_of(items)?),
            Expr::HasAny(items) => Rule::HasAny(self.items_of(items)?),
            Expr::HasGroup { group, count } => match self.count(*count)? {
                0 => Rule::True,
                count => Rule::HasGroup {
                    group: self
                        .groups
                        .get(*group as usize)
                        .copied()
                        .ok_or_else(|| missing("groupe"))?,
                    count,
                },
            },
            Expr::CanReachRegion(r) => match self.regions.get(*r as usize).copied().flatten() {
                Some(region) => Rule::CanReachRegion(region),
                None => Rule::False,
            },
            Expr::CanReachLocation(l) => {
                let found = match l {
                    LocationRef::Location(i) => self.locations.get(*i as usize),
                    LocationRef::Event(e) => self.event_locations.get(*e as usize),
                };
                match found.copied().flatten() {
                    Some(location) => Rule::CanReachLocation(location),
                    None => Rule::False,
                }
            }
            Expr::CanReachEntrance(e) => match self.entrances.get(*e as usize).copied().flatten() {
                Some(entrance) => Rule::CanReachEntrance(entrance),
                None => Rule::False,
            },
            Expr::And(parts) => {
                let mut kept = Vec::new();
                for part in parts {
                    match self.lower(part)? {
                        Rule::True => {}
                        Rule::False => return Ok(Rule::False),
                        rule => kept.push(rule),
                    }
                }
                simplify(kept, Rule::True, Rule::And)
            }
            Expr::Or(parts) => {
                let mut kept = Vec::new();
                for part in parts {
                    match self.lower(part)? {
                        Rule::False => {}
                        Rule::True => return Ok(Rule::True),
                        rule => kept.push(rule),
                    }
                }
                simplify(kept, Rule::False, Rule::Or)
            }
        })
    }

    fn items_of(&self, items: &[ItemRef]) -> Result<Vec<ItemKindId>, BuildError> {
        items.iter().map(|item| self.item(*item)).collect()
    }
}

/// Liste vide → `empty`, un seul élément → lui-même, sinon `wrap(liste)`.
///
/// `wrap` est le constructeur de la variante (`Rule::And` ou `Rule::Or`) : en Rust, une
/// variante d'enum à un champ s'utilise comme une fonction.
fn simplify(mut kept: Vec<Rule>, empty: Rule, wrap: fn(Vec<Rule>) -> Rule) -> Rule {
    match kept.len() {
        0 => empty,
        1 => kept.remove(0),
        _ => wrap(kept),
    }
}

impl NativeWorld {
    /// Les valeurs par défaut de toutes les options.
    pub fn default_values(&self) -> OptionValues {
        OptionValues {
            values: self.options.iter().map(|o| o.default).collect(),
        }
    }

    /// Valeur d'une option, par son nom.
    pub fn option_value(&self, values: &OptionValues, name: &str) -> Option<i64> {
        let index = self.options.iter().position(|o| o.name == name)?;
        values.values.get(index).copied()
    }

    /// Change la valeur d'une option, après l'avoir vérifiée.
    pub fn set_option(
        &self,
        values: &mut OptionValues,
        name: &str,
        value: i64,
    ) -> Result<(), BuildError> {
        let Some(index) = self.options.iter().position(|o| o.name == name) else {
            return Err(BuildError::BadOption {
                option: name.to_string(),
                message: "option inconnue".into(),
            });
        };
        self.check_value(index, value)?;
        match values.values.get_mut(index) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => Err(BuildError::BadOption {
                option: name.to_string(),
                message: "valeurs d'un autre monde".into(),
            }),
        }
    }

    fn check_value(&self, index: usize, value: i64) -> Result<(), BuildError> {
        let Some(option) = self.options.get(index) else {
            return Err(BuildError::Invalid("option inconnue".into()));
        };
        let message = match &option.kind {
            OptionKind::Toggle if !(0..=1).contains(&value) => Some("0 ou 1 attendu".to_string()),
            OptionKind::Range { min, max } if !(*min..=*max).contains(&value) => {
                Some(format!("doit être entre {min} et {max}"))
            }
            OptionKind::Choice { choices, .. } if value < 0 || value as usize >= choices.len() => {
                Some(format!("choix n° {value} inexistant"))
            }
            _ => None,
        };
        match message {
            Some(message) => Err(BuildError::BadOption {
                option: option.name.clone(),
                message,
            }),
            None => Ok(()),
        }
    }

    /// Vérifie toutes les valeurs (nombre et bornes).
    pub fn check_values(&self, values: &OptionValues) -> Result<(), BuildError> {
        if values.values.len() != self.options.len() {
            return Err(BuildError::Invalid(
                "valeurs d'options d'un autre monde".into(),
            ));
        }
        for (index, value) in values.values.iter().enumerate() {
            self.check_value(index, *value)?;
        }
        Ok(())
    }

    /// Options lisibles pour le spoiler : (nom affiché, valeur), dans l'ordre du fichier.
    pub fn spoiler_options(&self, values: &OptionValues) -> Vec<(String, String)> {
        self.options
            .iter()
            .zip(&values.values)
            .map(|(option, value)| {
                let shown = match &option.kind {
                    OptionKind::Toggle => if *value != 0 { "Yes" } else { "No" }.to_string(),
                    OptionKind::Range { .. } => value.to_string(),
                    OptionKind::Choice { choices, .. } => usize::try_from(*value)
                        .ok()
                        .and_then(|i| choices.get(i))
                        .map(|c| crate::load::title_case(c))
                        .unwrap_or_else(|| value.to_string()),
                };
                (option.display.clone(), shown)
            })
            .collect()
    }

    /// Objets du datapackage `(nom, numéro)`, par numéro croissant (toutes options).
    pub fn package_items(&self) -> Vec<(&str, i64)> {
        let mut items: Vec<(&str, i64)> =
            self.items.iter().map(|i| (i.name.as_str(), i.id)).collect();
        items.sort_by_key(|(_, id)| *id);
        items
    }

    /// Lieux du datapackage `(nom, numéro)`, par numéro croissant (toutes options).
    pub fn package_locations(&self) -> Vec<(&str, i64)> {
        let mut locations: Vec<(&str, i64)> = self
            .locations
            .iter()
            .map(|l| (l.name.as_str(), l.id))
            .collect();
        locations.sort_by_key(|(_, id)| *id);
        locations
    }

    /// Groupes d'objets du datapackage, dans l'ordre du fichier.
    pub fn package_item_groups(&self) -> Vec<(&str, Vec<&str>)> {
        self.item_groups
            .iter()
            .map(|g| {
                let members = g
                    .members
                    .iter()
                    .filter_map(|m| match m {
                        ItemRef::Item(i) => self.items.get(*i as usize).map(|i| i.name.as_str()),
                        ItemRef::Event(_) => None,
                    })
                    .collect();
                (g.name.as_str(), members)
            })
            .collect()
    }

    /// Groupes de lieux du datapackage, dans l'ordre du fichier.
    pub fn package_location_groups(&self) -> Vec<(&str, Vec<&str>)> {
        self.location_groups
            .iter()
            .map(|g| {
                let members = g
                    .members
                    .iter()
                    .filter_map(|l| self.locations.get(*l as usize).map(|l| l.name.as_str()))
                    .collect();
                (g.name.as_str(), members)
            })
            .collect()
    }

    /// Construit le monde d'un joueur dans le multiworld.
    ///
    /// `rng` sert seulement au remplissage (pièges tirés au hasard).
    pub fn build<R: Rng + ?Sized>(
        &self,
        values: &OptionValues,
        w: &mut MultiWorld,
        slot: SlotId,
        rng: &mut R,
    ) -> Result<Built, BuildError> {
        self.check_values(values)?;
        let v = values.as_slice();
        // Un élément sans `when` est toujours présent.
        let present = |when: &Option<Expr>| when.as_ref().is_none_or(|w| eval_static(w, v));
        // Premier cas qui convient ; le dernier n'a pas de condition, donc il y en a un.
        fn pick<'c, T>(choice: &'c Choice<T>, v: &[i64]) -> Result<&'c T, BuildError> {
            choice
                .cases
                .iter()
                .find(|(when, _)| when.as_ref().is_none_or(|w| eval_static(w, v)))
                .map(|(_, value)| value)
                .ok_or_else(|| BuildError::Invalid("aucun cas ne convient".into()))
        }

        // Objets, puis objets d'événement (tous, quelles que soient les options).
        let mut items = Vec::new();
        for item in &self.items {
            let classification = *pick(&item.classification, v)?;
            items.push(w.add_item_kind(slot, item.name.clone(), Some(item.id), classification)?);
        }
        // Un objet d'événement donné par plusieurs événements (format 2) n'est créé
        // qu'une fois : chaque événement en pose un exemplaire.
        let mut event_items = Vec::new();
        let mut created: std::collections::BTreeMap<&str, ItemKindId> = Default::default();
        // Format 3 : un événement peut donner un objet déclaré (à numéro) ; on retrouve
        // ces objets par leur nom.
        let declared_items: std::collections::BTreeMap<&str, ItemKindId> = self
            .items
            .iter()
            .map(|i| i.name.as_str())
            .zip(items.iter().copied())
            .collect();
        for event in &self.events {
            let declared = declared_items.get(event.item.as_str()).copied();
            let id = match declared.or_else(|| created.get(event.item.as_str()).copied()) {
                Some(id) => id,
                None => {
                    let id = w.add_item_kind(
                        slot,
                        event.item.clone(),
                        None,
                        logic::Classification::PROGRESSION,
                    )?;
                    created.insert(event.item.as_str(), id);
                    id
                }
            };
            event_items.push(id);
        }
        let mut groups = Vec::new();
        for group in &self.item_groups {
            let mut members = Vec::new();
            for member in &group.members {
                if let ItemRef::Item(i) = member {
                    members.push(
                        items
                            .get(*i as usize)
                            .copied()
                            .ok_or_else(|| missing("objet"))?,
                    );
                }
            }
            groups.push(w.add_item_group(slot, group.name.clone(), members)?);
        }

        // Régions présentes.
        let mut regions = Vec::new();
        for region in &self.regions {
            regions.push(if present(&region.when) {
                Some(w.add_region(slot, region.name.clone())?)
            } else {
                None
            });
        }
        let origin = match self.start_region {
            Some(r) => r,
            None => regions
                .iter()
                .position(Option::is_some)
                .map(|i| i as u32)
                .ok_or_else(|| {
                    BuildError::Invalid("aucune région présente avec ces options".into())
                })?,
        };
        let Some(origin_id) = regions.get(origin as usize).copied().flatten() else {
            return Err(BuildError::Invalid(
                "la région de départ est absente avec ces options".into(),
            ));
        };
        w.set_origin(slot, origin_id)?;
        let region_of = |r: u32, who: &str| -> Result<RegionId, BuildError> {
            regions.get(r as usize).copied().flatten().ok_or_else(|| {
                let name = self
                    .regions
                    .get(r as usize)
                    .map(|r| r.name.as_str())
                    .unwrap_or("?");
                BuildError::Invalid(format!(
                    "{who} touche la région « {name} », absente avec ces options"
                ))
            })
        };

        // Entrées présentes (règles posées plus bas, une fois tous les éléments créés).
        let mut entrances = Vec::new();
        for entrance in &self.entrances {
            entrances.push(if present(&entrance.when) {
                let who = format!("l'entrée « {} »", entrance.name);
                let from = region_of(entrance.from, &who)?;
                let to = region_of(entrance.to, &who)?;
                Some(w.add_entrance(from, to, entrance.name.clone(), Rule::True)?)
            } else {
                None
            });
        }

        // Avec ces options, toute région présente doit rester atteignable.
        let edges: Vec<(u32, u32)> = self
            .entrances
            .iter()
            .zip(&entrances)
            .filter(|(_, id)| id.is_some())
            .map(|(e, _)| (e.from, e.to))
            .collect();
        let reached = reachable_regions(self.regions.len(), origin, edges.iter().copied());
        for (i, region) in self.regions.iter().enumerate() {
            let is_present = regions.get(i).copied().flatten().is_some();
            if is_present && !reached.get(i).copied().unwrap_or(false) {
                return Err(BuildError::Invalid(format!(
                    "région « {} » inaccessible avec ces options",
                    region.name
                )));
            }
        }

        // Lieux, puis événements (posés d'office).
        let mut locations = Vec::new();
        for location in &self.locations {
            locations.push(if present(&location.when) {
                let who = format!("le lieu « {} »", location.name);
                let region = region_of(*pick(&location.region, v)?, &who)?;
                let id =
                    w.add_location(region, location.name.clone(), Some(location.id), Rule::True)?;
                if location.progress != Progress::Default {
                    w.set_progress(id, location.progress)?;
                }
                // Objet posé d'avance (format 3) : posé et verrouillé avant de compter les
                // lieux à remplir, comme `place_locked_item` d'Archipelago.
                if let Some(choice) = &location.item {
                    let item = items
                        .get(*pick(choice, v)? as usize)
                        .copied()
                        .ok_or_else(|| missing("objet"))?;
                    w.place_locked(id, item)?;
                }
                Some(id)
            } else {
                None
            });
        }
        let mut event_locations = Vec::new();
        for (event, item) in self.events.iter().zip(&event_items) {
            event_locations.push(if present(&event.when) {
                let who = format!("l'événement « {} »", event.name);
                let region = region_of(*pick(&event.region, v)?, &who)?;
                let id = w.add_location(region, event.name.clone(), None, Rule::True)?;
                w.place_locked(id, *item)?;
                Some(id)
            } else {
                None
            });
        }

        let ids = Ids {
            values: v,
            items,
            event_items,
            groups,
            regions,
            entrances,
            locations,
            event_locations,
        };

        // Règles et objectif.
        for (entrance, id) in self.entrances.iter().zip(&ids.entrances) {
            if let Some(id) = id {
                w.set_entrance_rule(*id, ids.lower(&entrance.rule)?)?;
            }
        }
        for (location, id) in self.locations.iter().zip(&ids.locations) {
            if let Some(id) = id {
                w.set_location_rule(*id, ids.lower(&location.rule)?)?;
            }
        }
        for (event, id) in self.events.iter().zip(&ids.event_locations) {
            if let Some(id) = id {
                w.set_location_rule(*id, ids.lower(&event.rule)?)?;
            }
        }
        w.set_goal(slot, ids.lower(&self.goal)?)?;

        // Pool, borné par le nombre de lieux à remplir.
        let unfilled = w
            .locations()
            .iter()
            .filter(|l| l.slot == slot && l.item.is_none())
            .count();
        let mut pool = Vec::new();
        for amount in self.pool.iter().filter(|a| present(&a.when)) {
            let item = ids.item(ItemRef::Item(amount.item))?;
            let count = ids.count(amount.count)? as usize;
            if pool.len().saturating_add(count) > unfilled {
                return Err(BuildError::Invalid(format!(
                    "plus d'objets dans le pool que de lieux ({unfilled})"
                )));
            }
            pool.extend(std::iter::repeat_n(item, count));
        }
        match &self.filler {
            Some(filler) => {
                let item = ids.item(ItemRef::Item(filler.item))?;
                let trap = match filler.trap {
                    Some((trap, percent)) => {
                        let percent = ids.count(percent)?;
                        if percent > 100 {
                            return Err(BuildError::Invalid(format!(
                                "pourcentage de pièges {percent} (au plus 100)"
                            )));
                        }
                        Some((ids.item(ItemRef::Item(trap))?, u64::from(percent)))
                    }
                    None => None,
                };
                while pool.len() < unfilled {
                    let chosen = match trap {
                        // `randint(0, 99) < trap_chance` en Python, comme le portage.
                        Some((trap, percent)) if fill::below(rng, 100) < percent => trap,
                        _ => item,
                    };
                    pool.push(chosen);
                }
            }
            None if pool.len() < unfilled => {
                return Err(BuildError::Invalid(format!(
                    "{} objets pour {unfilled} lieux, et pas de [filler]",
                    pool.len()
                )))
            }
            None => {}
        }

        // Inventaire de départ, plafonné.
        let mut start = Vec::new();
        for amount in self.start.iter().filter(|a| present(&a.when)) {
            let item = ids.item(ItemRef::Item(amount.item))?;
            let count = ids.count(amount.count)? as usize;
            if start.len().saturating_add(count) > MAX_ELEMENTS {
                return Err(BuildError::Invalid(
                    "inventaire de départ trop grand".into(),
                ));
            }
            start.extend(std::iter::repeat_n(item, count));
        }

        // slot_data, dans l'ordre du fichier.
        let mut slot_data = Map::new();
        for entry in &self.slot_data {
            let value = match &entry.value {
                SlotDataValue::Option(o) => Value::from(v.get(*o as usize).copied().unwrap_or(0)),
                SlotDataValue::Literal(value) => value.clone(),
            };
            slot_data.insert(entry.key.clone(), value);
        }

        Ok(Built {
            pool,
            start,
            slot_data: Value::Object(slot_data),
        })
    }
}
