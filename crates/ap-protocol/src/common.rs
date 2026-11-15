//! Types partagés par plusieurs paquets : versions, objets, joueurs, slots.
//!
//! Le serveur Python ajoute à certains objets un champ `"class"` (par exemple
//! `"class": "NetworkItem"`). Les clients Python s'en servent pour reconstruire
//! leurs objets : le serveur Raftup doit donc l'écrire aussi. L'attribut
//! `#[serde(tag = "class")]` s'en charge : serde l'ajoute toujours à l'écriture,
//! et tolère son absence à la lecture (testé dans `tests/pieges.rs`).

use serde::{Deserialize, Serialize};

/// Identifiant d'un objet (« item ») dans le DataPackage d'un jeu.
pub type ItemId = i64;
/// Identifiant d'un lieu (« location »). Négatif pour les lieux spéciaux :
/// `-1` = objet donné par le serveur (commande), `-2` = inventaire de départ.
pub type LocationId = i64;
/// Numéro de slot d'un joueur. `0` désigne le serveur lui-même.
pub type SlotId = u32;
/// Numéro d'équipe, à partir de `0`.
pub type TeamId = u32;

/// Version du protocole, d'Archipelago ou du générateur (`"class": "Version"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "class", rename = "Version")]
pub struct NetworkVersion {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
}

/// Drapeaux d'un objet : une suite de bits, comme en Python.
///
/// `#[serde(transparent)]` : en JSON, c'est juste le nombre (par exemple `1`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemFlags(pub u8);

impl ItemFlags {
    /// Objet ordinaire (« filler »).
    pub const NONE: ItemFlags = ItemFlags(0);
    /// Objet nécessaire pour progresser.
    pub const PROGRESSION: ItemFlags = ItemFlags(0b001);
    /// Objet utile, sans être indispensable.
    pub const USEFUL: ItemFlags = ItemFlags(0b010);
    /// Piège.
    pub const TRAP: ItemFlags = ItemFlags(0b100);

    /// Vrai si tous les bits de `other` sont présents.
    pub fn contains(self, other: ItemFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Un objet placé dans un lieu (`"class": "NetworkItem"`).
///
/// Le sens de `player` dépend du paquet : dans `ReceivedItems`, c'est le joueur
/// qui a trouvé l'objet ; dans `LocationInfo`, c'est celui qui le recevra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "class", rename = "NetworkItem")]
pub struct NetworkItem {
    pub item: ItemId,
    pub location: LocationId,
    pub player: SlotId,
    pub flags: ItemFlags,
}

/// Un joueur de la partie (`"class": "NetworkPlayer"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "class", rename = "NetworkPlayer")]
pub struct NetworkPlayer {
    pub team: TeamId,
    pub slot: SlotId,
    /// Nom affiché ; le joueur peut le changer avec `!alias`.
    pub alias: String,
    /// Nom du slot, fixé à la génération.
    pub name: String,
}

/// Type d'un slot : un nombre en JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SlotType(pub u8);

impl SlotType {
    /// Spectateur.
    pub const SPECTATOR: SlotType = SlotType(0);
    /// Joueur.
    pub const PLAYER: SlotType = SlotType(1);
    /// Groupe (par exemple un « item link »).
    pub const GROUP: SlotType = SlotType(2);
}

/// Informations fixes sur un slot (`"class": "NetworkSlot"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "class", rename = "NetworkSlot")]
pub struct NetworkSlot {
    pub name: String,
    pub game: String,
    #[serde(rename = "type")]
    pub slot_type: SlotType,
    /// Membres du groupe ; vide pour un joueur.
    #[serde(default)]
    pub group_members: Vec<SlotId>,
}

/// Droit d'utiliser `!release`, `!collect` ou `!remaining` : un nombre en JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Permission(pub u8);

impl Permission {
    /// Commande interdite.
    pub const DISABLED: Permission = Permission(0);
    /// Commande autorisée à tout moment.
    pub const ENABLED: Permission = Permission(1);
    /// Commande autorisée une fois l'objectif atteint.
    pub const GOAL: Permission = Permission(2);
    /// Exécutée automatiquement à l'objectif.
    pub const AUTO: Permission = Permission(6);
    /// Automatique à l'objectif, et autorisée avant.
    pub const AUTO_ENABLED: Permission = Permission(7);
}

/// Priorité d'un indice : un nombre en JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HintStatus(pub u8);

impl HintStatus {
    /// Priorité non précisée.
    pub const UNSPECIFIED: HintStatus = HintStatus(0);
    /// Pas prioritaire.
    pub const NO_PRIORITY: HintStatus = HintStatus(10);
    /// À éviter (par exemple un piège).
    pub const AVOID: HintStatus = HintStatus(20);
    /// Prioritaire.
    pub const PRIORITY: HintStatus = HintStatus(30);
    /// Objet déjà trouvé. Un client ne peut pas choisir ce statut lui-même.
    pub const FOUND: HintStatus = HintStatus(40);
}

/// État du client, envoyé avec `StatusUpdate` : un nombre en JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClientStatus(pub u8);

impl ClientStatus {
    pub const UNKNOWN: ClientStatus = ClientStatus(0);
    pub const CONNECTED: ClientStatus = ClientStatus(5);
    pub const READY: ClientStatus = ClientStatus(10);
    pub const PLAYING: ClientStatus = ClientStatus(20);
    /// Objectif atteint : définitif, le serveur ne revient pas en arrière.
    pub const GOAL: ClientStatus = ClientStatus(30);
}

/// Objets que le client veut recevoir du serveur : une suite de bits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemsHandling(pub u8);

impl ItemsHandling {
    /// Aucun objet : le jeu gère tout lui-même (rare).
    pub const NONE: ItemsHandling = ItemsHandling(0);
    /// Objets venant des autres mondes.
    pub const OTHER_WORLDS: ItemsHandling = ItemsHandling(0b001);
    /// Objets de son propre monde (à combiner avec `OTHER_WORLDS`).
    pub const OWN_WORLD: ItemsHandling = ItemsHandling(0b010);
    /// Inventaire de départ (à combiner avec `OTHER_WORLDS`).
    pub const STARTING_INVENTORY: ItemsHandling = ItemsHandling(0b100);
    /// Tous les objets : valeur la plus courante pour un mod.
    pub const ALL: ItemsHandling = ItemsHandling(0b111);
}
