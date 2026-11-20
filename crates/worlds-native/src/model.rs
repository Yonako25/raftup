//! Le monde natif une fois lu et vérifié : noms résolus en index, règles analysées.
//!
//! Ces types sont publics pour que les outils de la phase B (conversion apworld ↔ natif)
//! puissent parcourir un monde sans relire le TOML. Les éléments sont désignés par leur
//! position dans la liste correspondante (`u32`), comme dans `logic` (arènes).

use logic::{Classification, Progress};
use serde_json::Value;

/// Un objet désigné dans une règle : un objet déclaré, ou l'objet d'un événement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ItemRef {
    /// Position dans [`NativeWorld::items`].
    Item(u32),
    /// Position dans [`NativeWorld::events`] (son objet d'événement).
    Event(u32),
}

/// Un lieu désigné dans une règle : un lieu déclaré, ou le lieu d'un événement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LocationRef {
    /// Position dans [`NativeWorld::locations`].
    Location(u32),
    /// Position dans [`NativeWorld::events`].
    Event(u32),
}

/// Un nombre : écrit en dur, ou valeur d'une option `range`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Count {
    Fixed(u32),
    /// Position de l'option dans [`NativeWorld::options`].
    Option(u32),
}

/// Opérateur de comparaison d'une option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CompareOp {
    /// Applique la comparaison.
    pub fn apply(self, left: i64, right: i64) -> bool {
        match self {
            CompareOp::Eq => left == right,
            CompareOp::Ne => left != right,
            CompareOp::Lt => left < right,
            CompareOp::Le => left <= right,
            CompareOp::Gt => left > right,
            CompareOp::Ge => left >= right,
        }
    }

    /// Le symbole, tel qu'on l'écrit dans une règle.
    pub fn symbol(self) -> &'static str {
        match self {
            CompareOp::Eq => "==",
            CompareOp::Ne => "!=",
            CompareOp::Lt => "<",
            CompareOp::Le => "<=",
            CompareOp::Gt => ">",
            CompareOp::Ge => ">=",
        }
    }
}

/// Une expression du mini-langage, noms déjà résolus.
///
/// `Box<Expr>` pour `Not` : une enum ne peut pas se contenir elle-même directement (sa
/// taille serait infinie) ; la boîte met la sous-expression sur le tas. `Vec` le fait déjà.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// `true` ou `false`.
    Const(bool),
    Has {
        item: ItemRef,
        count: Count,
    },
    HasAll(Vec<ItemRef>),
    HasAny(Vec<ItemRef>),
    HasGroup {
        group: u32,
        count: Count,
    },
    CanReachRegion(u32),
    CanReachLocation(LocationRef),
    CanReachEntrance(u32),
    /// `option(nom)` d'une option `toggle`.
    Toggle(u32),
    /// `option(nom) <op> valeur`.
    Compare {
        option: u32,
        op: CompareOp,
        value: i64,
    },
    /// Seulement sur une expression statique.
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

impl Expr {
    /// Vrai si l'expression ne dépend que des options (pas de l'inventaire).
    pub fn is_static(&self) -> bool {
        match self {
            Expr::Const(_) | Expr::Toggle(_) | Expr::Compare { .. } => true,
            Expr::Not(inner) => inner.is_static(),
            Expr::And(parts) | Expr::Or(parts) => parts.iter().all(Expr::is_static),
            _ => false,
        }
    }
}

/// Une valeur qui peut dépendre des options : liste de cas, le premier dont la condition
/// est vraie gagne ; le dernier n'a pas de condition (`None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice<T> {
    pub cases: Vec<(Option<Expr>, T)>,
}

impl<T> Choice<T> {
    /// Une valeur qui ne dépend de rien.
    pub fn fixed(value: T) -> Choice<T> {
        Choice {
            cases: vec![(None, value)],
        }
    }
}

/// Type d'une option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionKind {
    Toggle,
    Range {
        min: i64,
        max: i64,
    },
    Choice {
        choices: Vec<String>,
        /// `(alias, position du choix)`, triés par alias.
        aliases: Vec<(String, u32)>,
    },
}

/// Une option du joueur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionDef {
    pub name: String,
    pub display: String,
    pub kind: OptionKind,
    /// Valeur par défaut (0/1, nombre, ou position du choix).
    pub default: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDef {
    pub name: String,
    pub id: i64,
    pub classification: Choice<Classification>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDef {
    pub name: String,
    pub members: Vec<ItemRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationGroupDef {
    pub name: String,
    /// Positions dans [`NativeWorld::locations`].
    pub members: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionDef {
    pub name: String,
    pub when: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntranceDef {
    pub name: String,
    pub from: u32,
    pub to: u32,
    pub rule: Expr,
    pub when: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationDef {
    pub name: String,
    pub id: i64,
    pub region: Choice<u32>,
    pub rule: Expr,
    pub progress: Progress,
    pub when: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventDef {
    pub name: String,
    pub region: Choice<u32>,
    pub rule: Expr,
    /// Nom de l'objet d'événement posé ici.
    pub item: String,
    pub when: Option<Expr>,
}

/// Une ligne du pool ou de l'inventaire de départ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amount {
    /// Position dans [`NativeWorld::items`].
    pub item: u32,
    pub count: Count,
    pub when: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filler {
    pub item: u32,
    /// Objet piège et pourcentage de chances.
    pub trap: Option<(u32, Count)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotDataValue {
    Option(u32),
    Literal(Value),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotDataDef {
    pub key: String,
    pub value: SlotDataValue,
}

/// Un monde natif lu et vérifié.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeWorld {
    pub game: String,
    /// Région de départ ; `None` : la première région présente.
    pub start_region: Option<u32>,
    pub goal: Expr,
    pub options: Vec<OptionDef>,
    pub items: Vec<ItemDef>,
    pub item_groups: Vec<GroupDef>,
    pub regions: Vec<RegionDef>,
    pub entrances: Vec<EntranceDef>,
    pub locations: Vec<LocationDef>,
    pub location_groups: Vec<LocationGroupDef>,
    pub events: Vec<EventDef>,
    pub pool: Vec<Amount>,
    pub filler: Option<Filler>,
    pub start: Vec<Amount>,
    pub slot_data: Vec<SlotDataDef>,
}
