//! Paquets envoyés par le client au serveur.
//!
//! Même format que dans l'autre sens : une trame est un tableau JSON de paquets,
//! et le champ `cmd` donne le type de chaque paquet.
//! La lecture passe par [`crate::codec`] : voir là-bas pourquoi on lit `cmd` à la main.
//!
//! La forme des paquets suit le client officiel (`CommonClient.py`) et ce que
//! le serveur officiel accepte (`process_client_cmd` dans `MultiServer.py`),
//! version 0.6.8.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::codec::{parse, split_cmd, with_cmd};
use crate::common::{
    ClientStatus, HintStatus, ItemsHandling, LocationId, NetworkVersion, SlotId, TeamId,
};
use crate::error::ProtocolError;

/// Un paquet envoyé par le client.
///
/// Chaque variante contient la structure du paquet, sans le champ `cmd`.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientPacket {
    Connect(Connect),
    ConnectUpdate(ConnectUpdate),
    Sync,
    LocationChecks(LocationChecks),
    LocationScouts(LocationScouts),
    CreateHints(CreateHints),
    UpdateHint(UpdateHint),
    StatusUpdate(StatusUpdate),
    Say(Say),
    GetDataPackage(GetDataPackage),
    Bounce(Bounce),
    Get(Get),
    Set(Set),
    SetNotify(SetNotify),
    /// Commande inconnue : on garde le paquet tel quel, `cmd` compris.
    /// Le serveur officiel l'ignore sans répondre.
    Unknown(Map<String, Value>),
}

impl ClientPacket {
    /// Nom de la commande, tel qu'il apparaît dans le champ `cmd`.
    pub fn cmd(&self) -> &str {
        match self {
            ClientPacket::Connect(_) => "Connect",
            ClientPacket::ConnectUpdate(_) => "ConnectUpdate",
            ClientPacket::Sync => "Sync",
            ClientPacket::LocationChecks(_) => "LocationChecks",
            ClientPacket::LocationScouts(_) => "LocationScouts",
            ClientPacket::CreateHints(_) => "CreateHints",
            ClientPacket::UpdateHint(_) => "UpdateHint",
            ClientPacket::StatusUpdate(_) => "StatusUpdate",
            ClientPacket::Say(_) => "Say",
            ClientPacket::GetDataPackage(_) => "GetDataPackage",
            ClientPacket::Bounce(_) => "Bounce",
            ClientPacket::Get(_) => "Get",
            ClientPacket::Set(_) => "Set",
            ClientPacket::SetNotify(_) => "SetNotify",
            // Un paquet inconnu a toujours un `cmd` texte : `from_value` l'a vérifié.
            ClientPacket::Unknown(map) => map.get("cmd").and_then(Value::as_str).unwrap_or(""),
        }
    }

    /// Construit un paquet à partir d'une valeur JSON déjà lue.
    pub fn from_value(value: Value) -> Result<Self, ProtocolError> {
        let (cmd, map) = split_cmd(value)?;
        let packet = match cmd.as_str() {
            "Connect" => ClientPacket::Connect(parse(&cmd, map)?),
            "ConnectUpdate" => ClientPacket::ConnectUpdate(parse(&cmd, map)?),
            // `Sync` n'a pas de champ : on ignore ceux qui seraient présents.
            "Sync" => ClientPacket::Sync,
            "LocationChecks" => ClientPacket::LocationChecks(parse(&cmd, map)?),
            "LocationScouts" => ClientPacket::LocationScouts(parse(&cmd, map)?),
            "CreateHints" => ClientPacket::CreateHints(parse(&cmd, map)?),
            "UpdateHint" => ClientPacket::UpdateHint(parse(&cmd, map)?),
            "StatusUpdate" => ClientPacket::StatusUpdate(parse(&cmd, map)?),
            "Say" => ClientPacket::Say(parse(&cmd, map)?),
            "GetDataPackage" => ClientPacket::GetDataPackage(parse(&cmd, map)?),
            "Bounce" => ClientPacket::Bounce(parse(&cmd, map)?),
            "Get" => ClientPacket::Get(parse(&cmd, map)?),
            "Set" => ClientPacket::Set(parse(&cmd, map)?),
            "SetNotify" => ClientPacket::SetNotify(parse(&cmd, map)?),
            _ => ClientPacket::Unknown(map),
        };
        Ok(packet)
    }
}

/// Structure vide, pour écrire `Sync` sous la forme `{"cmd":"Sync"}`.
#[derive(Serialize)]
struct NoFields {}

// On écrit `Serialize` à la main pour ajouter le champ `cmd`.
impl Serialize for ClientPacket {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ClientPacket::Connect(p) => with_cmd("Connect", p, serializer),
            ClientPacket::ConnectUpdate(p) => with_cmd("ConnectUpdate", p, serializer),
            ClientPacket::Sync => with_cmd("Sync", &NoFields {}, serializer),
            ClientPacket::LocationChecks(p) => with_cmd("LocationChecks", p, serializer),
            ClientPacket::LocationScouts(p) => with_cmd("LocationScouts", p, serializer),
            ClientPacket::CreateHints(p) => with_cmd("CreateHints", p, serializer),
            ClientPacket::UpdateHint(p) => with_cmd("UpdateHint", p, serializer),
            ClientPacket::StatusUpdate(p) => with_cmd("StatusUpdate", p, serializer),
            ClientPacket::Say(p) => with_cmd("Say", p, serializer),
            ClientPacket::GetDataPackage(p) => with_cmd("GetDataPackage", p, serializer),
            ClientPacket::Bounce(p) => with_cmd("Bounce", p, serializer),
            ClientPacket::Get(p) => with_cmd("Get", p, serializer),
            ClientPacket::Set(p) => with_cmd("Set", p, serializer),
            ClientPacket::SetNotify(p) => with_cmd("SetNotify", p, serializer),
            ClientPacket::Unknown(map) => map.serialize(serializer),
        }
    }
}

// `Deserialize` passe par `from_value`, comme pour `ServerPacket`.
impl<'de> Deserialize<'de> for ClientPacket {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        ClientPacket::from_value(value).map_err(D::Error::custom)
    }
}

/// Lit une trame WebSocket venue du client : un tableau JSON de paquets.
pub fn decode_client_frame(text: &str) -> Result<Vec<ClientPacket>, ProtocolError> {
    let values: Vec<Value> = serde_json::from_str(text)?;
    values.into_iter().map(ClientPacket::from_value).collect()
}

/// Écrit une trame WebSocket à envoyer au serveur.
pub fn encode_client_frame(packets: &[ClientPacket]) -> Result<String, ProtocolError> {
    Ok(serde_json::to_string(packets)?)
}

// ---------------------------------------------------------------------------
// Structures des paquets (mêmes attributs serde que dans `server.rs`)
// ---------------------------------------------------------------------------

/// Demande de connexion à un slot, juste après avoir reçu `RoomInfo`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Connect {
    /// Mot de passe de la salle ; `null` ou vide s'il n'y en a pas.
    /// Le serveur officiel exige la présence du champ.
    pub password: Option<String>,
    /// Nom du jeu ; peut être vide pour un outil sans jeu (tag `TextOnly`, `Tracker`).
    pub game: String,
    /// Nom du slot.
    pub name: String,
    /// Identifiant unique de la machine du joueur.
    pub uuid: String,
    /// Version du protocole du client. Doit porter `"class": "Version"` :
    /// le serveur Python s'en sert pour comparer les versions.
    pub version: NetworkVersion,
    pub items_handling: ItemsHandling,
    /// Par exemple `AP`, `DeathLink`, `TextOnly`, `Tracker`.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Vrai pour recevoir `slot_data` dans `Connected`. Absent = vrai pour le serveur.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_data: Option<bool>,
}

/// Change les réglages de la connexion en cours.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items_handling: Option<ItemsHandling>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Lieux visités par le joueur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationChecks {
    pub locations: Vec<LocationId>,
}

/// Demande le contenu de lieux, sans les visiter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationScouts {
    pub locations: Vec<LocationId>,
    /// `0` : simple question ; `1` : crée des indices ; `2` : crée seulement
    /// les indices nouveaux.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_as_hint: Option<u8>,
}

/// Crée des indices sur des lieux.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateHints {
    pub locations: Vec<LocationId>,
    /// Joueur à qui sont les lieux. Absent = le client lui-même.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<SlotId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<HintStatus>,
}

/// Change la priorité d'un indice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateHint {
    /// Joueur qui trouvera l'objet.
    pub player: SlotId,
    pub location: LocationId,
    /// Absent : rien ne change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<HintStatus>,
}

/// Nouvel état du client (prêt, en jeu, objectif atteint…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusUpdate {
    pub status: ClientStatus,
}

/// Message de chat, ou commande du serveur commençant par `!`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Say {
    pub text: String,
}

/// Demande le DataPackage de certains jeux.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetDataPackage {
    /// Jeux voulus. Avec le serveur 0.6.8, absent = aucun jeu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub games: Option<Vec<String>>,
}

/// Message libre que le serveur renvoie, sous forme de `Bounced`, aux clients
/// visés (par équipe, jeu, slot ou tag). Sert par exemple au DeathLink.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Bounce {
    /// Équipes visées. Absent = l'équipe de l'expéditeur.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teams: Option<Vec<TeamId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub games: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slots: Option<Vec<SlotId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Map<String, Value>>,
    /// Autres champs (par exemple `operator` : `legacy`, `or`, `and`),
    /// renvoyés tels quels dans le `Bounced`.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Lit des valeurs du stockage de données (réponse : `Retrieved`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Get {
    pub keys: Vec<String>,
    /// Champs libres, renvoyés tels quels dans le `Retrieved`.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Modifie une valeur du stockage de données (réponse : `SetReply`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Set {
    pub key: String,
    /// Valeur de départ si la clé n'existe pas encore. Absent = `0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// Vrai pour recevoir un `SetReply` même sans `SetNotify`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub want_reply: Option<bool>,
    /// Opérations appliquées dans l'ordre.
    pub operations: Vec<DataStorageOperation>,
    /// Champs libres, renvoyés tels quels dans le `SetReply`.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Une opération sur une valeur du stockage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataStorageOperation {
    /// Par exemple `replace`, `default`, `add`, `mul`, `max`, `or`, `remove`, `update`.
    pub operation: String,
    pub value: Value,
}

/// Demande à être prévenu (par `SetReply`) quand ces clés changent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetNotify {
    pub keys: Vec<String>,
}
