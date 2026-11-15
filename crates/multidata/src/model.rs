//! Modèle de partie : ce que contient une seed, avec des types Rust.
//!
//! Correspond au `MultiData` du serveur officiel (`NetUtils.py`), version 0.6.8.
//! Les numéros de slot, de lieu et d'objet reprennent les types d'`ap-protocol`.
//! Les tables sont des `BTreeMap` (ordre déterministe), sauf le DataPackage,
//! dont on garde l'ordre d'origine (`IndexMap`) car il est renvoyé aux clients.

use std::collections::BTreeMap;

use ap_protocol::{
    Hint, ItemFlags, ItemId, LocationId, NetworkSlot, NetworkVersion, SlotId, TeamId,
};
use indexmap::IndexMap;
use serde_json::Value;

/// Une partie générée, prête à être servie.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiData {
    /// Nom de la seed (affiché dans `RoomInfo`).
    pub seed_name: String,
    /// Version d'Archipelago qui a généré la partie.
    pub generator_version: NetworkVersion,
    /// Version minimale du serveur pour servir cette partie.
    pub minimum_server_version: NetworkVersion,
    /// Version minimale du client, par slot.
    pub minimum_client_versions: BTreeMap<SlotId, NetworkVersion>,
    pub tags: Vec<String>,
    /// Mode course : certaines informations sont cachées aux joueurs.
    pub race_mode: bool,
    /// Informations fixes de chaque slot (joueur ou groupe).
    pub slots: BTreeMap<SlotId, NetworkSlot>,
    /// Nom de connexion → (équipe, slot).
    pub connect_names: BTreeMap<String, (TeamId, SlotId)>,
    /// Données propres au jeu de chaque slot, envoyées dans `Connected`.
    /// Déjà converties en JSON, comme le ferait le serveur officiel.
    pub slot_data: BTreeMap<SlotId, Value>,
    /// Contenu de chaque lieu : slot → lieu → objet placé.
    pub locations: BTreeMap<SlotId, BTreeMap<LocationId, PlacedItem>>,
    /// Inventaire de départ de chaque slot.
    pub precollected_items: BTreeMap<SlotId, Vec<ItemId>>,
    /// Indices connus dès le départ, par slot.
    pub precollected_hints: BTreeMap<SlotId, Vec<Hint>>,
    /// Entrée à prendre pour chaque lieu (randomisation des entrées), par slot.
    pub er_hint_data: BTreeMap<SlotId, BTreeMap<LocationId, String>>,
    /// Sphères de la génération : à chaque étape, les lieux accessibles par slot.
    pub spheres: Vec<BTreeMap<SlotId, Vec<LocationId>>>,
    /// Options du serveur choisies à la génération.
    pub server_options: ServerOptions,
    /// Noms des objets et des lieux, par jeu.
    pub datapackage: IndexMap<String, GamePackage>,
}

/// L'objet placé dans un lieu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacedItem {
    pub item: ItemId,
    /// Joueur qui recevra l'objet.
    pub player: SlotId,
    pub flags: ItemFlags,
}

/// Noms et identifiants d'un jeu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GamePackage {
    pub item_name_to_id: IndexMap<String, ItemId>,
    pub location_name_to_id: IndexMap<String, LocationId>,
    /// Groupes d'objets (par exemple « Everything »), pour les indices.
    pub item_name_groups: IndexMap<String, Vec<String>>,
    pub location_name_groups: IndexMap<String, Vec<String>>,
    pub checksum: String,
}

/// Options du serveur enregistrées dans la seed.
///
/// Seules les options que le serveur officiel applique depuis la seed
/// (`simple_options` et `disable_item_cheat` dans `MultiServer.py`) sont gardées.
/// Valeurs par défaut : celles du serveur officiel (`settings.py`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerOptions {
    /// Mot de passe pour se connecter à la salle.
    pub password: Option<String>,
    /// Mot de passe des commandes d'administration.
    pub server_password: Option<String>,
    pub hint_cost: i64,
    pub location_check_points: i64,
    /// `auto`, `enabled`, `disabled`, `goal` ou `auto-enabled`.
    pub release_mode: String,
    pub collect_mode: String,
    /// `enabled`, `disabled` ou `goal`.
    pub remaining_mode: String,
    /// `auto`, `enabled` ou `disabled`.
    pub countdown_mode: String,
    /// Vrai si la commande `!getitem` est interdite.
    pub disable_item_cheat: bool,
    /// `0` : version exacte exigée ; `1` : plus souple ; `2` : le plus souple.
    pub compatibility: i64,
}

impl Default for ServerOptions {
    fn default() -> Self {
        ServerOptions {
            password: None,
            server_password: None,
            hint_cost: 10,
            location_check_points: 1,
            release_mode: "auto".to_string(),
            collect_mode: "auto".to_string(),
            remaining_mode: "goal".to_string(),
            countdown_mode: "auto".to_string(),
            disable_item_cheat: false,
            compatibility: 2,
        }
    }
}
