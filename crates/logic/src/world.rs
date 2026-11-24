//! Le modèle : un seul `MultiWorld` pour tous les joueurs, comme dans Archipelago.
//!
//! Tout est rangé dans des `Vec` (des « arènes ») : un élément est désigné par sa position,
//! jamais par une référence. On évite ainsi les durées de vie compliquées : une région
//! connaît ses voisines par leur numéro, pas en les empruntant.
//!
//! Les noms sont uniques par joueur et par sorte d'élément ; on les retrouve par des
//! `BTreeMap`, triées, pour que tout parcours reste déterministe.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::ids::{EntranceId, ItemGroupId, ItemKindId, ItemRuleId, LocationId, RegionId, SlotId};
use crate::item_rule::{Accessibility, ItemRule};
use crate::rule::Rule;

/// Erreurs de construction ou de validation d'un monde.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LogicError {
    #[error("joueur inconnu : n° {0}")]
    UnknownSlot(u32),
    #[error("région inconnue : n° {0}")]
    UnknownRegion(u32),
    #[error("entrée inconnue : n° {0}")]
    UnknownEntrance(u32),
    #[error("lieu inconnu : n° {0}")]
    UnknownLocation(u32),
    #[error("objet inconnu : n° {0}")]
    UnknownItem(u32),
    #[error("groupe d'objets inconnu : n° {0}")]
    UnknownGroup(u32),
    #[error("règle d'objet inconnue : n° {0}")]
    UnknownItemRule(u32),
    #[error("{kind} « {name} » en double pour le joueur {slot}")]
    DuplicateName {
        slot: String,
        kind: &'static str,
        name: String,
    },
    #[error("l'entrée « {name} » relie deux joueurs différents")]
    CrossSlotEntrance { name: String },
    #[error("le groupe « {group} » contient un objet d'un autre joueur")]
    CrossSlotGroup { group: String },
    #[error("le lieu « {location} » contient déjà un objet")]
    LocationFilled { location: String },
    #[error("trop d'éléments de type {0} (plus de 4 milliards)")]
    TooMany(&'static str),
}

/// Classification d'un objet, comme les drapeaux d'Archipelago (qui se combinent :
/// un objet peut être à la fois de progression et utile).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Classification {
    /// Débloque quelque chose : compte dans l'inventaire de la logique.
    pub progression: bool,
    /// Agréable à recevoir, sans rien débloquer.
    pub useful: bool,
    /// Piège.
    pub trap: bool,
    /// Progression ignorée par l'équilibrage (`progression_skip_balancing`).
    pub skip_balancing: bool,
}

impl Classification {
    /// Remplissage : ni progression, ni utile, ni piège.
    pub const FILLER: Classification = Classification {
        progression: false,
        useful: false,
        trap: false,
        skip_balancing: false,
    };
    /// Objet de progression.
    pub const PROGRESSION: Classification = Classification {
        progression: true,
        useful: false,
        trap: false,
        skip_balancing: false,
    };
    /// Objet utile.
    pub const USEFUL: Classification = Classification {
        progression: false,
        useful: true,
        trap: false,
        skip_balancing: false,
    };
    /// Piège.
    pub const TRAP: Classification = Classification {
        progression: false,
        useful: false,
        trap: true,
        skip_balancing: false,
    };
}

/// Un joueur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub name: String,
    pub game: String,
    /// Région de départ. Par défaut, la première région créée pour ce joueur
    /// (« Menu » dans Archipelago).
    pub origin: Option<RegionId>,
    /// Condition de victoire (vraie par défaut, comme dans Archipelago).
    pub goal: Rule,
    /// Ce que le placement doit garantir à ce joueur (`Full` par défaut).
    pub accessibility: Accessibility,
}

/// Une région.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub slot: SlotId,
    pub name: String,
}

/// Une entrée : passage orienté de `from` vers `to`, franchissable si `rule` est vraie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entrance {
    pub slot: SlotId,
    pub name: String,
    pub from: RegionId,
    pub to: RegionId,
    pub rule: Rule,
}

/// Un lieu, dans une région.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub slot: SlotId,
    pub name: String,
    pub region: RegionId,
    /// Numéro réseau (celui du datapackage, que voit le mod) ; `None` pour un événement.
    pub address: Option<i64>,
    pub rule: Rule,
    /// Objet placé ici, s'il y en a un.
    pub item: Option<ItemKindId>,
    /// Objet posé d'avance par le monde (`place_locked_item` d'Archipelago, ADR 0054) :
    /// le placement et les équilibrages n'y touchent jamais. Posé par
    /// [`MultiWorld::place_locked`] ; un événement (sans numéro) est toujours verrouillé.
    pub locked: bool,
    /// Normal, prioritaire ou exclu (`LocationProgressType` d'Archipelago).
    pub progress: Progress,
    /// Règle d'objet du lieu ; `None` : tout objet est accepté (voir
    /// [`MultiWorld::can_hold`]).
    pub item_rule: Option<ItemRuleId>,
}

/// Ce que le placement a le droit de mettre dans un lieu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Progress {
    /// N'importe quel objet.
    #[default]
    Default,
    /// Reçoit de préférence un objet de progression.
    Priority,
    /// Ne reçoit ni progression ni objet utile : seulement du remplissage ou un piège.
    Excluded,
}

/// Une sorte d'objet d'un joueur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemKind {
    pub slot: SlotId,
    pub name: String,
    /// Numéro réseau ; `None` pour un événement.
    pub code: Option<i64>,
    pub classification: Classification,
}

/// Un groupe nommé de sortes d'objets d'un joueur (pour `HasGroup`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemGroup {
    pub slot: SlotId,
    pub name: String,
    pub members: Vec<ItemKindId>,
}

/// Index des noms d'un joueur, un par sorte d'élément.
#[derive(Debug, Clone, Default)]
struct SlotNames {
    regions: BTreeMap<String, RegionId>,
    entrances: BTreeMap<String, EntranceId>,
    locations: BTreeMap<String, LocationId>,
    items: BTreeMap<String, ItemKindId>,
    groups: BTreeMap<String, ItemGroupId>,
}

/// Le multiworld : tous les joueurs et tous leurs éléments.
#[derive(Debug, Clone, Default)]
pub struct MultiWorld {
    slots: Vec<Slot>,
    regions: Vec<Region>,
    entrances: Vec<Entrance>,
    locations: Vec<Location>,
    item_kinds: Vec<ItemKind>,
    item_groups: Vec<ItemGroup>,
    item_rules: Vec<ItemRule>,
    names: Vec<SlotNames>,
}

/// Prochain index d'un tableau, en `u32`.
fn next_index<T>(items: &[T], what: &'static str) -> Result<u32, LogicError> {
    u32::try_from(items.len()).map_err(|_| LogicError::TooMany(what))
}

impl MultiWorld {
    /// Monde vide.
    pub fn new() -> MultiWorld {
        MultiWorld::default()
    }

    // ---- Construction ----

    /// Ajoute un joueur.
    pub fn add_slot(
        &mut self,
        name: impl Into<String>,
        game: impl Into<String>,
    ) -> Result<SlotId, LogicError> {
        let id = SlotId(next_index(&self.slots, "joueur")?);
        self.slots.push(Slot {
            name: name.into(),
            game: game.into(),
            origin: None,
            goal: Rule::True,
            accessibility: Accessibility::Full,
        });
        self.names.push(SlotNames::default());
        Ok(id)
    }

    /// Ajoute une région au joueur. La première région d'un joueur devient son départ.
    pub fn add_region(
        &mut self,
        slot: SlotId,
        name: impl Into<String>,
    ) -> Result<RegionId, LogicError> {
        let name = name.into();
        let id = RegionId(next_index(&self.regions, "région")?);
        let (slot_name, names) = self.names_of(slot)?;
        if names.regions.contains_key(&name) {
            return Err(duplicate(slot_name, "région", name));
        }
        names.regions.insert(name.clone(), id);
        self.regions.push(Region { slot, name });
        if let Some(slot) = self.slots.get_mut(slot.index()) {
            // `get_or_insert` : ne remplace pas un départ déjà fixé.
            slot.origin.get_or_insert(id);
        }
        Ok(id)
    }

    /// Ajoute une entrée de `from` vers `to` (même joueur).
    pub fn add_entrance(
        &mut self,
        from: RegionId,
        to: RegionId,
        name: impl Into<String>,
        rule: Rule,
    ) -> Result<EntranceId, LogicError> {
        let name = name.into();
        let slot = self.region_or_err(from)?.slot;
        if self.region_or_err(to)?.slot != slot {
            return Err(LogicError::CrossSlotEntrance { name });
        }
        let id = EntranceId(next_index(&self.entrances, "entrée")?);
        let (slot_name, names) = self.names_of(slot)?;
        if names.entrances.contains_key(&name) {
            return Err(duplicate(slot_name, "entrée", name));
        }
        names.entrances.insert(name.clone(), id);
        self.entrances.push(Entrance {
            slot,
            name,
            from,
            to,
            rule,
        });
        Ok(id)
    }

    /// Ajoute un lieu dans une région ; `address` vaut `None` pour un événement.
    pub fn add_location(
        &mut self,
        region: RegionId,
        name: impl Into<String>,
        address: Option<i64>,
        rule: Rule,
    ) -> Result<LocationId, LogicError> {
        let name = name.into();
        let slot = self.region_or_err(region)?.slot;
        let id = LocationId(next_index(&self.locations, "lieu")?);
        let (slot_name, names) = self.names_of(slot)?;
        if names.locations.contains_key(&name) {
            return Err(duplicate(slot_name, "lieu", name));
        }
        names.locations.insert(name.clone(), id);
        self.locations.push(Location {
            slot,
            name,
            region,
            address,
            rule,
            item: None,
            locked: false,
            progress: Progress::Default,
            item_rule: None,
        });
        Ok(id)
    }

    /// Ajoute une sorte d'objet au joueur ; `code` vaut `None` pour un événement.
    pub fn add_item_kind(
        &mut self,
        slot: SlotId,
        name: impl Into<String>,
        code: Option<i64>,
        classification: Classification,
    ) -> Result<ItemKindId, LogicError> {
        let name = name.into();
        let id = ItemKindId(next_index(&self.item_kinds, "objet")?);
        let (slot_name, names) = self.names_of(slot)?;
        if names.items.contains_key(&name) {
            return Err(duplicate(slot_name, "objet", name));
        }
        names.items.insert(name.clone(), id);
        self.item_kinds.push(ItemKind {
            slot,
            name,
            code,
            classification,
        });
        Ok(id)
    }

    /// Ajoute un groupe d'objets ; tous les membres doivent appartenir au joueur.
    pub fn add_item_group(
        &mut self,
        slot: SlotId,
        name: impl Into<String>,
        members: Vec<ItemKindId>,
    ) -> Result<ItemGroupId, LogicError> {
        let name = name.into();
        for member in &members {
            let kind = self
                .item_kind(*member)
                .ok_or(LogicError::UnknownItem(member.raw()))?;
            if kind.slot != slot {
                return Err(LogicError::CrossSlotGroup { group: name });
            }
        }
        let id = ItemGroupId(next_index(&self.item_groups, "groupe")?);
        let (slot_name, names) = self.names_of(slot)?;
        if names.groups.contains_key(&name) {
            return Err(duplicate(slot_name, "groupe", name));
        }
        names.groups.insert(name.clone(), id);
        self.item_groups.push(ItemGroup {
            slot,
            name,
            members,
        });
        Ok(id)
    }

    /// Change la région de départ du joueur.
    pub fn set_origin(&mut self, slot: SlotId, region: RegionId) -> Result<(), LogicError> {
        self.region_or_err(region)?;
        let slot = self
            .slots
            .get_mut(slot.index())
            .ok_or(LogicError::UnknownSlot(slot.raw()))?;
        slot.origin = Some(region);
        Ok(())
    }

    /// Fixe la condition de victoire du joueur.
    pub fn set_goal(&mut self, slot: SlotId, goal: Rule) -> Result<(), LogicError> {
        let slot = self
            .slots
            .get_mut(slot.index())
            .ok_or(LogicError::UnknownSlot(slot.raw()))?;
        slot.goal = goal;
        Ok(())
    }

    /// Remplace la règle d'une entrée. Utile quand les règles se citent entre elles
    /// (`CanReachLocation` d'un lieu créé plus tard) : on crée tout, puis on pose les règles.
    pub fn set_entrance_rule(
        &mut self,
        entrance: EntranceId,
        rule: Rule,
    ) -> Result<(), LogicError> {
        let entrance = self
            .entrances
            .get_mut(entrance.index())
            .ok_or(LogicError::UnknownEntrance(entrance.raw()))?;
        entrance.rule = rule;
        Ok(())
    }

    /// Remplace la règle d'un lieu (voir [`MultiWorld::set_entrance_rule`]).
    pub fn set_location_rule(
        &mut self,
        location: LocationId,
        rule: Rule,
    ) -> Result<(), LogicError> {
        let location = self
            .locations
            .get_mut(location.index())
            .ok_or(LogicError::UnknownLocation(location.raw()))?;
        location.rule = rule;
        Ok(())
    }

    /// Rend un lieu prioritaire ou exclu (voir [`Progress`]).
    pub fn set_progress(
        &mut self,
        location: LocationId,
        progress: Progress,
    ) -> Result<(), LogicError> {
        let location = self
            .locations
            .get_mut(location.index())
            .ok_or(LogicError::UnknownLocation(location.raw()))?;
        location.progress = progress;
        Ok(())
    }

    /// Fixe l'accessibilité du joueur (voir [`Accessibility`]).
    pub fn set_accessibility(
        &mut self,
        slot: SlotId,
        accessibility: Accessibility,
    ) -> Result<(), LogicError> {
        let slot = self
            .slots
            .get_mut(slot.index())
            .ok_or(LogicError::UnknownSlot(slot.raw()))?;
        slot.accessibility = accessibility;
        Ok(())
    }

    /// Range une règle d'objet ; plusieurs lieux peuvent ensuite la partager.
    pub fn add_item_rule(&mut self, rule: ItemRule) -> Result<ItemRuleId, LogicError> {
        for item in rule.items() {
            self.item_or_err(item)?;
        }
        let id = ItemRuleId(next_index(&self.item_rules, "règle d'objet")?);
        self.item_rules.push(rule);
        Ok(id)
    }

    /// Donne une règle d'objet à un lieu (`None` : tout objet est accepté).
    pub fn set_item_rule(
        &mut self,
        location: LocationId,
        rule: Option<ItemRuleId>,
    ) -> Result<(), LogicError> {
        if let Some(rule) = rule {
            if self.item_rule(rule).is_none() {
                return Err(LogicError::UnknownItemRule(rule.raw()));
            }
        }
        let location = self
            .locations
            .get_mut(location.index())
            .ok_or(LogicError::UnknownLocation(location.raw()))?;
        location.item_rule = rule;
        Ok(())
    }

    /// Le lieu a-t-il le droit de recevoir cet objet ? Comme `Location.can_fill`
    /// d'Archipelago, sans regarder si le lieu est atteignable :
    ///
    /// - un objet de `always_allow` passe toujours (même dans un lieu exclu) ;
    /// - sinon, un lieu exclu refuse la progression et les objets utiles ;
    /// - sinon, la règle d'objet du lieu décide (tout passe s'il n'en a pas).
    ///
    /// Un lieu ou un objet inconnu donne `false`.
    pub fn can_hold(&self, location: LocationId, item: ItemKindId) -> bool {
        let (Some(location), Some(kind)) = (self.location(location), self.item_kind(item)) else {
            return false;
        };
        let rule = location.item_rule.and_then(|id| self.item_rule(id));
        if rule.is_some_and(|r| r.always_allows(item)) {
            return true;
        }
        let important = kind.classification.progression || kind.classification.useful;
        if location.progress == Progress::Excluded && important {
            return false;
        }
        rule.is_none_or(|r| r.allows(item))
    }

    /// Le lieu doit-il être atteignable à la fin du placement ? Comme
    /// `MultiWorld.fulfills_accessibility` d'Archipelago :
    ///
    /// - oui si son joueur est en accessibilité `Full` ;
    /// - sinon, seulement s'il garde un objet de progression d'un joueur qui n'est pas en
    ///   `Minimal` (un objet nécessaire à quelqu'un qui veut tout atteindre).
    pub fn must_reach(&self, location: LocationId) -> bool {
        let Some(location) = self.location(location) else {
            return false;
        };
        let accessibility = |slot: SlotId| self.slot(slot).map(|s| s.accessibility);
        if accessibility(location.slot) == Some(Accessibility::Full) {
            return true;
        }
        location
            .item
            .and_then(|item| self.item_kind(item))
            .is_some_and(|kind| {
                kind.classification.progression
                    && accessibility(kind.slot) != Some(Accessibility::Minimal)
            })
    }

    /// Le monde a au moins une règle d'objet.
    pub fn has_item_rules(&self) -> bool {
        !self.item_rules.is_empty()
    }

    /// Place un objet (de n'importe quel joueur) dans un lieu vide.
    pub fn place(&mut self, location: LocationId, item: ItemKindId) -> Result<(), LogicError> {
        if self.item_kind(item).is_none() {
            return Err(LogicError::UnknownItem(item.raw()));
        }
        let location = self
            .locations
            .get_mut(location.index())
            .ok_or(LogicError::UnknownLocation(location.raw()))?;
        if location.item.is_some() {
            return Err(LogicError::LocationFilled {
                location: location.name.clone(),
            });
        }
        location.item = Some(item);
        Ok(())
    }

    /// Pose un objet d'avance et verrouille le lieu (ADR 0054) : comme [`MultiWorld::place`],
    /// puis plus aucun équilibrage ne déplace cet objet.
    pub fn place_locked(
        &mut self,
        location: LocationId,
        item: ItemKindId,
    ) -> Result<(), LogicError> {
        self.place(location, item)?;
        if let Some(location) = self.locations.get_mut(location.index()) {
            location.locked = true;
        }
        Ok(())
    }

    /// Le lieu est-il verrouillé ? Oui s'il a été posé par [`MultiWorld::place_locked`],
    /// ou s'il n'a pas de numéro (un événement ne bouge jamais). Un lieu inconnu aussi.
    pub fn is_locked(&self, location: LocationId) -> bool {
        self.location(location)
            .is_none_or(|l| l.locked || l.address.is_none())
    }

    /// Retire l'objet d'un lieu et le renvoie (`None` si le lieu était vide).
    pub fn take(&mut self, location: LocationId) -> Result<Option<ItemKindId>, LogicError> {
        let location = self
            .locations
            .get_mut(location.index())
            .ok_or(LogicError::UnknownLocation(location.raw()))?;
        // `take` sur une `Option` : renvoie la valeur et laisse `None` à sa place.
        // Le verrou reste : il dit ce qu'est le lieu, pas ce qu'il contient (les
        // équilibrages vident puis reposent tous les lieux pour revenir en arrière).
        Ok(location.item.take())
    }

    // ---- Lecture ----

    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }
    pub fn entrances(&self) -> &[Entrance] {
        &self.entrances
    }
    pub fn locations(&self) -> &[Location] {
        &self.locations
    }
    pub fn item_kinds(&self) -> &[ItemKind] {
        &self.item_kinds
    }
    pub fn item_groups(&self) -> &[ItemGroup] {
        &self.item_groups
    }

    pub fn slot(&self, id: SlotId) -> Option<&Slot> {
        self.slots.get(id.index())
    }
    pub fn region(&self, id: RegionId) -> Option<&Region> {
        self.regions.get(id.index())
    }
    pub fn entrance(&self, id: EntranceId) -> Option<&Entrance> {
        self.entrances.get(id.index())
    }
    pub fn location(&self, id: LocationId) -> Option<&Location> {
        self.locations.get(id.index())
    }
    pub fn item_kind(&self, id: ItemKindId) -> Option<&ItemKind> {
        self.item_kinds.get(id.index())
    }
    pub fn item_group(&self, id: ItemGroupId) -> Option<&ItemGroup> {
        self.item_groups.get(id.index())
    }
    pub fn item_rule(&self, id: ItemRuleId) -> Option<&ItemRule> {
        self.item_rules.get(id.index())
    }

    /// Identifiants des joueurs, dans l'ordre.
    pub fn slot_ids(&self) -> impl Iterator<Item = SlotId> + '_ {
        // Les index tiennent dans un `u32` : `add_slot` refuse d'en créer davantage.
        (0..self.slots.len() as u32).map(SlotId)
    }

    /// Identifiants des lieux, dans l'ordre.
    pub fn location_ids(&self) -> impl Iterator<Item = LocationId> + '_ {
        (0..self.locations.len() as u32).map(LocationId)
    }

    /// Région d'un joueur, par son nom.
    pub fn region_named(&self, slot: SlotId, name: &str) -> Option<RegionId> {
        self.names.get(slot.index())?.regions.get(name).copied()
    }
    /// Entrée d'un joueur, par son nom.
    pub fn entrance_named(&self, slot: SlotId, name: &str) -> Option<EntranceId> {
        self.names.get(slot.index())?.entrances.get(name).copied()
    }
    /// Lieu d'un joueur, par son nom.
    pub fn location_named(&self, slot: SlotId, name: &str) -> Option<LocationId> {
        self.names.get(slot.index())?.locations.get(name).copied()
    }
    /// Sorte d'objet d'un joueur, par son nom.
    pub fn item_named(&self, slot: SlotId, name: &str) -> Option<ItemKindId> {
        self.names.get(slot.index())?.items.get(name).copied()
    }
    /// Groupe d'objets d'un joueur, par son nom.
    pub fn group_named(&self, slot: SlotId, name: &str) -> Option<ItemGroupId> {
        self.names.get(slot.index())?.groups.get(name).copied()
    }

    // ---- Validation ----

    /// Vérifie que chaque identifiant cité (règles, départs, objets placés) existe.
    ///
    /// Les règles peuvent citer des éléments créés plus tard (un lieu qui demande une
    /// région pas encore ajoutée) : on ne peut donc les vérifier qu'une fois le monde
    /// complet. Renvoie la première erreur, dans l'ordre de création.
    pub fn validate(&self) -> Result<(), LogicError> {
        for slot in &self.slots {
            if let Some(origin) = slot.origin {
                self.region_or_err(origin)?;
            }
            self.check_rule(&slot.goal)?;
        }
        for entrance in &self.entrances {
            self.check_rule(&entrance.rule)?;
        }
        for location in &self.locations {
            self.check_rule(&location.rule)?;
            if let Some(item) = location.item {
                self.item_or_err(item)?;
            }
            if let Some(rule) = location.item_rule {
                self.item_rule(rule)
                    .ok_or(LogicError::UnknownItemRule(rule.raw()))?;
            }
        }
        Ok(())
    }

    fn check_rule(&self, rule: &Rule) -> Result<(), LogicError> {
        let mut result = Ok(());
        rule.walk(&mut |node| {
            if result.is_ok() {
                result = self.check_node(node);
            }
        });
        result
    }

    /// Vérifie les identifiants d'un seul nœud (pas ses enfants).
    fn check_node(&self, node: &Rule) -> Result<(), LogicError> {
        match node {
            Rule::Has { item, .. } => self.item_or_err(*item).map(|_| ()),
            Rule::HasAll(items) | Rule::HasAny(items) => items
                .iter()
                .try_for_each(|item| self.item_or_err(*item).map(|_| ())),
            Rule::HasAllCounts(pairs) => pairs
                .iter()
                .try_for_each(|(item, _)| self.item_or_err(*item).map(|_| ())),
            Rule::Sum { terms, .. } => terms
                .iter()
                .try_for_each(|term| self.item_or_err(term.item).map(|_| ())),
            Rule::HasGroup { group, .. } => self
                .item_group(*group)
                .map(|_| ())
                .ok_or(LogicError::UnknownGroup(group.raw())),
            Rule::CanReachRegion(region) => self.region_or_err(*region).map(|_| ()),
            Rule::CanReachLocation(location) => self
                .location(*location)
                .map(|_| ())
                .ok_or(LogicError::UnknownLocation(location.raw())),
            Rule::CanReachEntrance(entrance) => self
                .entrance(*entrance)
                .map(|_| ())
                .ok_or(LogicError::UnknownEntrance(entrance.raw())),
            Rule::True | Rule::False | Rule::And(_) | Rule::Or(_) => Ok(()),
        }
    }

    // ---- Aides internes ----

    fn region_or_err(&self, id: RegionId) -> Result<&Region, LogicError> {
        self.region(id).ok_or(LogicError::UnknownRegion(id.raw()))
    }

    fn item_or_err(&self, id: ItemKindId) -> Result<&ItemKind, LogicError> {
        self.item_kind(id).ok_or(LogicError::UnknownItem(id.raw()))
    }

    /// Nom du joueur et son index de noms, modifiable.
    fn names_of(&mut self, slot: SlotId) -> Result<(String, &mut SlotNames), LogicError> {
        let slot_name = self
            .slots
            .get(slot.index())
            .ok_or(LogicError::UnknownSlot(slot.raw()))?
            .name
            .clone();
        let names = self
            .names
            .get_mut(slot.index())
            .ok_or(LogicError::UnknownSlot(slot.raw()))?;
        Ok((slot_name, names))
    }
}

fn duplicate(slot: String, kind: &'static str, name: String) -> LogicError {
    LogicError::DuplicateName { slot, kind, name }
}
