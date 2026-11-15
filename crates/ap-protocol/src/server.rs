//! Paquets envoyés par le serveur au client.
//!
//! Une trame WebSocket contient un tableau JSON de paquets. Chaque paquet est un
//! objet dont le champ `cmd` donne le type, par exemple
//! `[{"cmd":"RoomInfo", ...}, {"cmd":"PrintJSON", ...}]`.
//!
//! La lecture passe par [`crate::codec`] : voir là-bas pourquoi on lit `cmd` à la main.

use indexmap::IndexMap;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::codec::{parse, split_cmd, with_cmd};
use crate::common::{
    HintStatus, ItemFlags, ItemId, LocationId, NetworkItem, NetworkPlayer, NetworkSlot,
    NetworkVersion, Permission, SlotId, TeamId,
};
use crate::error::ProtocolError;

/// Un paquet envoyé par le serveur.
///
/// Chaque variante contient la structure du paquet, sans le champ `cmd`.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerPacket {
    RoomInfo(RoomInfo),
    ConnectionRefused(ConnectionRefused),
    Connected(Connected),
    ReceivedItems(ReceivedItems),
    LocationInfo(LocationInfo),
    RoomUpdate(RoomUpdate),
    PrintJson(PrintJson),
    DataPackage(DataPackage),
    Bounced(Bounced),
    InvalidPacket(InvalidPacket),
    Retrieved(Retrieved),
    SetReply(SetReply),
    /// Commande inconnue (version plus récente du protocole) : on garde le
    /// paquet tel quel, `cmd` compris, au lieu d'échouer.
    Unknown(Map<String, Value>),
}

impl ServerPacket {
    /// Nom de la commande, tel qu'il apparaît dans le champ `cmd`.
    pub fn cmd(&self) -> &str {
        match self {
            ServerPacket::RoomInfo(_) => "RoomInfo",
            ServerPacket::ConnectionRefused(_) => "ConnectionRefused",
            ServerPacket::Connected(_) => "Connected",
            ServerPacket::ReceivedItems(_) => "ReceivedItems",
            ServerPacket::LocationInfo(_) => "LocationInfo",
            ServerPacket::RoomUpdate(_) => "RoomUpdate",
            ServerPacket::PrintJson(_) => "PrintJSON",
            ServerPacket::DataPackage(_) => "DataPackage",
            ServerPacket::Bounced(_) => "Bounced",
            ServerPacket::InvalidPacket(_) => "InvalidPacket",
            ServerPacket::Retrieved(_) => "Retrieved",
            ServerPacket::SetReply(_) => "SetReply",
            // Un paquet inconnu a toujours un `cmd` texte : `from_value` l'a vérifié.
            ServerPacket::Unknown(map) => map.get("cmd").and_then(Value::as_str).unwrap_or(""),
        }
    }

    /// Construit un paquet à partir d'une valeur JSON déjà lue.
    pub fn from_value(value: Value) -> Result<Self, ProtocolError> {
        let (cmd, map) = split_cmd(value)?;
        let packet = match cmd.as_str() {
            "RoomInfo" => ServerPacket::RoomInfo(parse(&cmd, map)?),
            "ConnectionRefused" => ServerPacket::ConnectionRefused(parse(&cmd, map)?),
            "Connected" => ServerPacket::Connected(parse(&cmd, map)?),
            "ReceivedItems" => ServerPacket::ReceivedItems(parse(&cmd, map)?),
            "LocationInfo" => ServerPacket::LocationInfo(parse(&cmd, map)?),
            "RoomUpdate" => ServerPacket::RoomUpdate(parse(&cmd, map)?),
            "PrintJSON" => ServerPacket::PrintJson(parse(&cmd, map)?),
            "DataPackage" => ServerPacket::DataPackage(parse(&cmd, map)?),
            "Bounced" => ServerPacket::Bounced(parse(&cmd, map)?),
            "InvalidPacket" => ServerPacket::InvalidPacket(parse(&cmd, map)?),
            "Retrieved" => ServerPacket::Retrieved(parse(&cmd, map)?),
            "SetReply" => ServerPacket::SetReply(parse(&cmd, map)?),
            // Commande inconnue : le paquet est gardé tel quel, `cmd` compris.
            _ => ServerPacket::Unknown(map),
        };
        Ok(packet)
    }
}

// On écrit `Serialize` à la main pour ajouter le champ `cmd`.
impl Serialize for ServerPacket {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ServerPacket::RoomInfo(p) => with_cmd("RoomInfo", p, serializer),
            ServerPacket::ConnectionRefused(p) => with_cmd("ConnectionRefused", p, serializer),
            ServerPacket::Connected(p) => with_cmd("Connected", p, serializer),
            ServerPacket::ReceivedItems(p) => with_cmd("ReceivedItems", p, serializer),
            ServerPacket::LocationInfo(p) => with_cmd("LocationInfo", p, serializer),
            ServerPacket::RoomUpdate(p) => with_cmd("RoomUpdate", p, serializer),
            ServerPacket::PrintJson(p) => with_cmd("PrintJSON", p, serializer),
            ServerPacket::DataPackage(p) => with_cmd("DataPackage", p, serializer),
            ServerPacket::Bounced(p) => with_cmd("Bounced", p, serializer),
            ServerPacket::InvalidPacket(p) => with_cmd("InvalidPacket", p, serializer),
            ServerPacket::Retrieved(p) => with_cmd("Retrieved", p, serializer),
            ServerPacket::SetReply(p) => with_cmd("SetReply", p, serializer),
            ServerPacket::Unknown(map) => map.serialize(serializer),
        }
    }
}

// `Deserialize` passe par `from_value`, pour la raison expliquée en tête du module.
impl<'de> Deserialize<'de> for ServerPacket {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        ServerPacket::from_value(value).map_err(D::Error::custom)
    }
}

/// Lit une trame WebSocket venue du serveur : un tableau JSON de paquets.
///
/// La taille maximale d'une trame est limitée par la couche WebSocket (étape 4) ;
/// la profondeur d'imbrication est limitée par serde_json (128 niveaux).
pub fn decode_server_frame(text: &str) -> Result<Vec<ServerPacket>, ProtocolError> {
    let values: Vec<Value> = serde_json::from_str(text)?;
    values.into_iter().map(ServerPacket::from_value).collect()
}

/// Écrit une trame WebSocket à envoyer au client.
pub fn encode_server_frame(packets: &[ServerPacket]) -> Result<String, ProtocolError> {
    Ok(serde_json::to_string(packets)?)
}

// ---------------------------------------------------------------------------
// Structures des paquets
//
// Attributs serde utilisés :
// - `#[serde(default)]` : champ absent → valeur par défaut (liste vide, None…),
//   pour tolérer les serveurs qui ne l'envoient pas ;
// - `skip_serializing_if = "Option::is_none"` : champ non écrit s'il vaut None ;
// - `rename = "type"` : `type` est un mot réservé en Rust.
// Les champs inconnus sont ignorés à la lecture (comportement par défaut de serde).
// ---------------------------------------------------------------------------

/// Premier paquet reçu, juste après l'ouverture de la connexion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomInfo {
    /// Version du protocole du serveur.
    pub version: NetworkVersion,
    /// Version d'Archipelago qui a généré la partie.
    pub generator_version: NetworkVersion,
    pub tags: Vec<String>,
    /// Vrai si un mot de passe est demandé pour se connecter.
    pub password: bool,
    /// Droits pour `release`, `collect` et `remaining`.
    pub permissions: IndexMap<String, Permission>,
    /// Prix d'un indice, en pourcentage du nombre de lieux.
    pub hint_cost: u32,
    /// Points d'indice gagnés par lieu visité.
    pub location_check_points: u32,
    pub games: Vec<String>,
    /// Empreinte du DataPackage de chaque jeu, pour savoir s'il faut le redemander.
    #[serde(default)]
    pub datapackage_checksums: IndexMap<String, String>,
    pub seed_name: String,
    /// Heure du serveur, en secondes depuis le 1er janvier 1970.
    pub time: f64,
}

/// Connexion refusée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionRefused {
    /// Par exemple `InvalidSlot`, `InvalidGame`, `IncompatibleVersion`,
    /// `InvalidPassword`, `InvalidItemsHandling`.
    #[serde(default)]
    pub errors: Vec<String>,
}

/// Connexion acceptée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Connected {
    pub team: TeamId,
    pub slot: SlotId,
    pub players: Vec<NetworkPlayer>,
    pub missing_locations: Vec<LocationId>,
    pub checked_locations: Vec<LocationId>,
    /// Données propres au jeu, envoyées si le client les a demandées.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_data: Option<Value>,
    /// Informations sur chaque slot. En JSON, les clés sont des textes (`"1"`) ;
    /// serde les convertit en nombres.
    pub slot_info: IndexMap<SlotId, NetworkSlot>,
    pub hint_points: i64,
}

/// Objets reçus par le joueur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceivedItems {
    /// Position du premier objet de la liste dans l'historique du joueur.
    /// `0` signifie « liste complète » : le client repart de zéro.
    pub index: u32,
    pub items: Vec<NetworkItem>,
}

/// Réponse à `LocationScouts` : le contenu des lieux demandés.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationInfo {
    pub locations: Vec<NetworkItem>,
}

/// Changements de l'état de la salle : seuls les champs modifiés sont présents.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoomUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint_points: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub players: Option<Vec<NetworkPlayer>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_locations: Option<Vec<LocationId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_locations: Option<Vec<LocationId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<IndexMap<String, Permission>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint_cost: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_check_points: Option<u32>,
}

/// Message à afficher, découpé en morceaux typés (joueur, objet, lieu…).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintJson {
    pub data: Vec<JsonMessagePart>,
    /// Genre du message : `ItemSend`, `Chat`, `Join`, `Goal`… Absent pour
    /// certains messages du serveur.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Joueur qui reçoit l'objet (`ItemSend`, `ItemCheat`, `Hint`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiving: Option<SlotId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<NetworkItem>,
    /// Pour un indice : vrai si l'objet a déjà été trouvé.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<SlotId>,
    /// Texte d'origine d'un message de chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Secondes restantes d'un compte à rebours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub countdown: Option<i64>,
}

/// Morceau d'un message `PrintJSON`.
///
/// Pour `item_id`, `location_id` ou `player_id`, `text` contient un numéro que
/// le client remplace par un nom grâce au DataPackage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonMessagePart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Par exemple `player_id`, `item_id`, `location_id`, `color`. Absent = texte simple.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<ItemFlags>,
    /// Joueur à qui appartient l'objet ou le lieu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<SlotId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint_status: Option<HintStatus>,
}

/// Réponse à `GetDataPackage` : les noms des objets et des lieux de chaque jeu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataPackage {
    pub data: DataPackageData,
}

/// Contenu d'un `DataPackage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataPackageData {
    pub games: IndexMap<String, GameData>,
}

/// Noms et identifiants d'un jeu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameData {
    pub item_name_to_id: IndexMap<String, ItemId>,
    pub location_name_to_id: IndexMap<String, LocationId>,
    pub checksum: String,
}

/// Paquet `Bounce` relayé par le serveur (par exemple un DeathLink).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bounced {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teams: Option<Vec<TeamId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub games: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slots: Option<Vec<SlotId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Contenu libre, choisi par le client qui a envoyé le `Bounce`.
    /// Absent pour un simple « ping » (le client officiel en envoie).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Map<String, Value>>,
    /// Autres champs du `Bounce` (par exemple `operator`), renvoyés tels quels :
    /// le serveur officiel renvoie le paquet entier en changeant seulement `cmd`.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Le serveur n'a pas compris un paquet du client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidPacket {
    /// `cmd` (commande inconnue) ou `arguments` (champs incorrects).
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_cmd: Option<String>,
    pub text: String,
}

/// Réponse à `Get` : les valeurs demandées dans le stockage de données.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Retrieved {
    pub keys: Map<String, Value>,
    /// Autres champs du `Get`, renvoyés tels quels par le serveur.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Une valeur du stockage de données a changé (après un `Set`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetReply {
    pub key: String,
    pub value: Value,
    pub original_value: Value,
    /// Slot qui a fait le `Set`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<SlotId>,
    /// Autres champs du `Set` (`default`, `want_reply`, `operations`, champs
    /// libres), renvoyés tels quels par le serveur.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
