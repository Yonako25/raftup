//! Conversion du pickle brut ([`PickleDocument`]) en modèle de partie ([`MultiData`]).
//!
//! Chaque erreur indique le chemin du champ fautif, par exemple
//! `locations[1][81000]`, pour savoir tout de suite ce qui ne va pas dans la seed.

use std::collections::BTreeMap;

use ap_protocol::{
    Hint, HintStatus, ItemFlags, NetworkSlot, NetworkVersion, SlotId, SlotType, TeamId,
};
use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::error::ModelError;
use crate::model::{GamePackage, MultiData, PlacedItem, ServerOptions};
use crate::pickle::{Node, NodeId, PickleDocument, PyClass};

/// Profondeur maximale d'imbrication pour `slot_data` (et protection contre
/// une valeur qui se contient elle-même).
const MAX_JSON_DEPTH: usize = 512;

type Res<T> = Result<T, ModelError>;

fn error(path: &str, problem: impl Into<String>) -> ModelError {
    ModelError {
        path: path.to_string(),
        problem: problem.into(),
    }
}

impl MultiData {
    /// Construit le modèle à partir du pickle d'une seed.
    pub fn from_document(doc: &PickleDocument) -> Result<MultiData, ModelError> {
        let r = Reader { doc };
        let root = r.dict(doc.root(), "racine")?;

        let minimum_versions = r.dict(
            r.require(root, "minimum_versions", "racine")?,
            "minimum_versions",
        )?;
        let minimum_server_version = r.version(
            r.require(minimum_versions, "server", "minimum_versions")?,
            "minimum_versions.server",
        )?;
        let minimum_client_versions = match r.field(minimum_versions, "clients") {
            Some(id) => r.slot_map(id, "minimum_versions.clients", |id, path| {
                r.version(id, path)
            })?,
            None => BTreeMap::new(),
        };

        Ok(MultiData {
            seed_name: r.string(r.require(root, "seed_name", "racine")?, "seed_name")?,
            generator_version: r.version(r.require(root, "version", "racine")?, "version")?,
            minimum_server_version,
            minimum_client_versions,
            tags: match r.field(root, "tags") {
                Some(id) => r.list(id, "tags", |id, path| r.string(id, path))?,
                None => Vec::new(),
            },
            race_mode: match r.field(root, "race_mode") {
                Some(id) => r.int(id, "race_mode")? != 0,
                None => false,
            },
            slots: r.slot_map(
                r.require(root, "slot_info", "racine")?,
                "slot_info",
                |id, path| r.network_slot(id, path),
            )?,
            connect_names: r.connect_names(r.require(root, "connect_names", "racine")?)?,
            slot_data: r.slot_map(
                r.require(root, "slot_data", "racine")?,
                "slot_data",
                |id, path| r.json(id, path, 0),
            )?,
            locations: r.slot_map(
                r.require(root, "locations", "racine")?,
                "locations",
                |id, path| r.locations(id, path),
            )?,
            precollected_items: r.optional_slot_map(root, "precollected_items", |id, path| {
                r.list(id, path, |id, path| r.int(id, path))
            })?,
            precollected_hints: r.optional_slot_map(root, "precollected_hints", |id, path| {
                r.list(id, path, |id, path| r.hint(id, path))
            })?,
            er_hint_data: r.optional_slot_map(root, "er_hint_data", |id, path| {
                r.int_map(id, path, |id, path| r.string(id, path))
            })?,
            spheres: match r.field(root, "spheres") {
                Some(id) => r.list(id, "spheres", |id, path| {
                    r.slot_map(id, path, |id, path| {
                        r.list(id, path, |id, path| r.int(id, path))
                    })
                })?,
                None => Vec::new(),
            },
            server_options: match r.field(root, "server_options") {
                Some(id) => r.server_options(id)?,
                None => ServerOptions::default(),
            },
            datapackage: match r.field(root, "datapackage") {
                Some(id) => r.datapackage(id)?,
                None => IndexMap::new(),
            },
        })
    }
}

/// Outils de lecture typée sur un document pickle.
///
/// `'a` : les références renvoyées pointent dans le document, qui vit plus
/// longtemps que le lecteur.
struct Reader<'a> {
    doc: &'a PickleDocument,
}

impl<'a> Reader<'a> {
    fn node(&self, id: NodeId) -> &'a Node {
        self.doc.node(id)
    }

    // --- Types simples ------------------------------------------------------

    fn dict(&self, id: NodeId, path: &str) -> Res<&'a [(NodeId, NodeId)]> {
        match self.node(id) {
            Node::Dict(pairs) => Ok(pairs),
            // `Counter` est un dictionnaire : on lit son contenu.
            Node::Object {
                class: PyClass::Counter,
                args,
                ..
            } => match args.first() {
                Some(inner) => self.dict(*inner, path),
                None => Ok(&[]),
            },
            _ => Err(error(path, "dictionnaire attendu")),
        }
    }

    /// Liste, tuple ou ensemble : la suite de ses éléments.
    fn seq(&self, id: NodeId, path: &str) -> Res<&'a [NodeId]> {
        match self.node(id) {
            Node::List(items) | Node::Tuple(items) | Node::Set(items) | Node::FrozenSet(items) => {
                Ok(items)
            }
            _ => Err(error(path, "liste attendue")),
        }
    }

    fn string(&self, id: NodeId, path: &str) -> Res<String> {
        match self.node(id) {
            Node::Str(text) => Ok(text.clone()),
            _ => Err(error(path, "texte attendu")),
        }
    }

    fn int(&self, id: NodeId, path: &str) -> Res<i64> {
        match self.node(id) {
            Node::Int(value) => Ok(*value),
            // En Python, `True` vaut 1 et `False` vaut 0.
            Node::Bool(value) => Ok(i64::from(*value)),
            // Énumération entière (`SlotType`, `HintStatus`…) : sa valeur.
            Node::Object {
                class: PyClass::SlotType | PyClass::HintStatus | PyClass::ClientStatus,
                args,
                ..
            } => match args.first() {
                Some(value) => self.int(*value, path),
                None => Err(error(path, "énumération sans valeur")),
            },
            Node::BigInt(_) => Err(error(path, "entier trop grand")),
            _ => Err(error(path, "entier attendu")),
        }
    }

    fn bool(&self, id: NodeId, path: &str) -> Res<bool> {
        match self.node(id) {
            Node::Bool(value) => Ok(*value),
            Node::Int(value) => Ok(*value != 0),
            _ => Err(error(path, "booléen attendu")),
        }
    }

    /// Entier converti dans un type plus petit (`u32`, `u8`…), sans débordement.
    fn small<T: TryFrom<i64>>(&self, id: NodeId, path: &str) -> Res<T> {
        let value = self.int(id, path)?;
        T::try_from(value).map_err(|_| error(path, format!("valeur hors limites : {value}")))
    }

    // --- Dictionnaires ------------------------------------------------------

    /// Valeur d'une clé texte, si elle existe.
    fn field(&self, pairs: &[(NodeId, NodeId)], key: &str) -> Option<NodeId> {
        pairs
            .iter()
            .find(|(k, _)| matches!(self.node(*k), Node::Str(s) if s == key))
            .map(|(_, v)| *v)
    }

    fn require(&self, pairs: &[(NodeId, NodeId)], key: &str, path: &str) -> Res<NodeId> {
        self.field(pairs, key)
            .ok_or_else(|| error(path, format!("clé « {key} » absente")))
    }

    /// Dictionnaire à clés entières (lieux, numéros…), valeurs converties par `f`.
    ///
    /// `F: Fn(NodeId, &str) -> Res<T>` : `f` reçoit la valeur et son chemin.
    fn int_map<K, T, F>(&self, id: NodeId, path: &str, f: F) -> Res<BTreeMap<K, T>>
    where
        K: TryFrom<i64> + Ord,
        F: Fn(NodeId, &str) -> Res<T>,
    {
        let mut out = BTreeMap::new();
        for (key, value) in self.dict(id, path)? {
            let child = format!("{path}[{}]", key_text(self.node(*key)));
            let key: K = self.small(*key, path)?;
            out.insert(key, f(*value, &child)?);
        }
        Ok(out)
    }

    fn slot_map<T, F>(&self, id: NodeId, path: &str, f: F) -> Res<BTreeMap<SlotId, T>>
    where
        F: Fn(NodeId, &str) -> Res<T>,
    {
        let mut out = BTreeMap::new();
        for (key, value) in self.dict(id, path)? {
            let slot: SlotId = self.small(*key, path)?;
            out.insert(slot, f(*value, &format!("{path}[{slot}]"))?);
        }
        Ok(out)
    }

    fn optional_slot_map<T, F>(
        &self,
        root: &[(NodeId, NodeId)],
        key: &str,
        f: F,
    ) -> Res<BTreeMap<SlotId, T>>
    where
        F: Fn(NodeId, &str) -> Res<T>,
    {
        match self.field(root, key) {
            Some(id) => self.slot_map(id, key, f),
            None => Ok(BTreeMap::new()),
        }
    }

    fn list<T, F>(&self, id: NodeId, path: &str, f: F) -> Res<Vec<T>>
    where
        F: Fn(NodeId, &str) -> Res<T>,
    {
        self.seq(id, path)?
            .iter()
            .enumerate()
            .map(|(i, item)| f(*item, &format!("{path}[{i}]")))
            .collect()
    }

    // --- Champs de la seed --------------------------------------------------

    /// `(major, minor, build)`.
    fn version(&self, id: NodeId, path: &str) -> Res<NetworkVersion> {
        match self.seq(id, path)? {
            [major, minor, build] => Ok(NetworkVersion {
                major: self.small(*major, path)?,
                minor: self.small(*minor, path)?,
                build: self.small(*build, path)?,
            }),
            _ => Err(error(path, "version à 3 nombres attendue")),
        }
    }

    /// Arguments d'un objet de la classe attendue.
    fn object_args(&self, id: NodeId, class: PyClass, path: &str) -> Res<&'a [NodeId]> {
        match self.node(id) {
            Node::Object { class: c, args, .. } if *c == class => Ok(args),
            _ => Err(error(path, format!("objet {} attendu", class.path()))),
        }
    }

    fn network_slot(&self, id: NodeId, path: &str) -> Res<NetworkSlot> {
        let args = self.object_args(id, PyClass::NetworkSlot, path)?;
        let [name, game, slot_type, rest @ ..] = args else {
            return Err(error(path, "NetworkSlot incomplet"));
        };
        let group_members = match rest.first() {
            Some(members) => self.list(*members, path, |id, path| self.small(id, path))?,
            None => Vec::new(),
        };
        Ok(NetworkSlot {
            name: self.string(*name, path)?,
            game: self.string(*game, path)?,
            slot_type: SlotType(self.small(*slot_type, path)?),
            group_members,
        })
    }

    fn connect_names(&self, id: NodeId) -> Res<BTreeMap<String, (TeamId, SlotId)>> {
        let mut out = BTreeMap::new();
        for (key, value) in self.dict(id, "connect_names")? {
            let name = self.string(*key, "connect_names")?;
            let path = format!("connect_names[{name}]");
            let [team, slot] = self.seq(*value, &path)? else {
                return Err(error(&path, "(équipe, slot) attendu"));
            };
            let entry = (self.small(*team, &path)?, self.small(*slot, &path)?);
            out.insert(name, entry);
        }
        Ok(out)
    }

    fn locations(&self, id: NodeId, path: &str) -> Res<BTreeMap<i64, PlacedItem>> {
        self.int_map(id, path, |value, path| {
            let [item, player, flags] = self.seq(value, path)? else {
                return Err(error(path, "(objet, joueur, drapeaux) attendu"));
            };
            Ok(PlacedItem {
                item: self.int(*item, path)?,
                player: self.small(*player, path)?,
                flags: ItemFlags(self.small(*flags, path)?),
            })
        })
    }

    /// `Hint(receiving_player, finding_player, location, item, found,
    /// entrance="", item_flags=0, status=HINT_UNSPECIFIED)`.
    fn hint(&self, id: NodeId, path: &str) -> Res<Hint> {
        let args = self.object_args(id, PyClass::Hint, path)?;
        let [receiving, finding, location, item, found, rest @ ..] = args else {
            return Err(error(path, "Hint incomplet"));
        };
        Ok(Hint {
            receiving_player: self.small(*receiving, path)?,
            finding_player: self.small(*finding, path)?,
            location: self.int(*location, path)?,
            item: self.int(*item, path)?,
            found: self.bool(*found, path)?,
            entrance: match rest.first() {
                Some(id) => self.string(*id, path)?,
                None => String::new(),
            },
            item_flags: match rest.get(1) {
                Some(id) => ItemFlags(self.small(*id, path)?),
                None => ItemFlags::NONE,
            },
            status: match rest.get(2) {
                Some(id) => HintStatus(self.small(*id, path)?),
                None => HintStatus::UNSPECIFIED,
            },
        })
    }

    fn server_options(&self, id: NodeId) -> Res<ServerOptions> {
        let pairs = self.dict(id, "server_options")?;
        let mut options = ServerOptions::default();
        // Comme le serveur officiel : une valeur illisible est ignorée.
        let text = |key: &str| match self.field(pairs, key).map(|id| self.node(id)) {
            Some(Node::Str(value)) => Some(value.clone()),
            _ => None,
        };
        let number = |key: &str| {
            self.field(pairs, key)
                .and_then(|id| self.int(id, "server_options").ok())
        };
        options.password = text("password");
        options.server_password = text("server_password");
        if let Some(value) = number("hint_cost") {
            options.hint_cost = value;
        }
        if let Some(value) = number("location_check_points") {
            options.location_check_points = value;
        }
        if let Some(value) = number("compatibility") {
            options.compatibility = value;
        }
        if let Some(value) = text("release_mode") {
            options.release_mode = value;
        }
        if let Some(value) = text("collect_mode") {
            options.collect_mode = value;
        }
        if let Some(value) = text("remaining_mode") {
            options.remaining_mode = value;
        }
        if let Some(value) = text("countdown_mode") {
            options.countdown_mode = value;
        }
        if let Some(id) = self.field(pairs, "disable_item_cheat") {
            options.disable_item_cheat = self.bool(id, "server_options.disable_item_cheat")?;
        }
        Ok(options)
    }

    fn datapackage(&self, id: NodeId) -> Res<IndexMap<String, GamePackage>> {
        let mut out = IndexMap::new();
        for (key, value) in self.dict(id, "datapackage")? {
            let game = self.string(*key, "datapackage")?;
            let path = format!("datapackage[{game}]");
            let pairs = self.dict(*value, &path)?;
            let names = |key: &str| -> Res<IndexMap<String, i64>> {
                let mut map = IndexMap::new();
                if let Some(id) = self.field(pairs, key) {
                    let path = format!("{path}.{key}");
                    for (name, number) in self.dict(id, &path)? {
                        map.insert(self.string(*name, &path)?, self.int(*number, &path)?);
                    }
                }
                Ok(map)
            };
            let groups = |key: &str| -> Res<IndexMap<String, Vec<String>>> {
                let mut map = IndexMap::new();
                if let Some(id) = self.field(pairs, key) {
                    let path = format!("{path}.{key}");
                    for (name, members) in self.dict(id, &path)? {
                        let members =
                            self.list(*members, &path, |id, path| self.string(id, path))?;
                        map.insert(self.string(*name, &path)?, members);
                    }
                }
                Ok(map)
            };
            let package = GamePackage {
                item_name_to_id: names("item_name_to_id")?,
                location_name_to_id: names("location_name_to_id")?,
                item_name_groups: groups("item_name_groups")?,
                location_name_groups: groups("location_name_groups")?,
                checksum: match self.field(pairs, "checksum") {
                    Some(id) => self.string(id, &path)?,
                    None => String::new(),
                },
            };
            out.insert(game, package);
        }
        Ok(out)
    }

    // --- Conversion en JSON (slot_data) -------------------------------------

    /// Convertit une valeur comme l'encodeur du serveur officiel
    /// (`NetUtils.encode` : `_scan_for_TypedTuples` puis `json`).
    fn json(&self, id: NodeId, path: &str, depth: usize) -> Res<Value> {
        if depth > MAX_JSON_DEPTH {
            return Err(error(path, "imbrication trop profonde"));
        }
        let inner = |id: NodeId| self.json(id, path, depth + 1);
        let value = match self.node(id) {
            Node::None => Value::Null,
            Node::Bool(b) => Value::Bool(*b),
            Node::Int(i) => Value::from(*i),
            Node::BigInt(bytes) => big_int_value(bytes, path)?,
            // `NaN` et l'infini n'existent pas en JSON : `null`.
            Node::Float(f) => serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number),
            Node::Str(s) => Value::String(s.clone()),
            Node::Bytes(_) => return Err(error(path, "octets impossibles à écrire en JSON")),
            // Listes, tuples et ensembles deviennent des listes JSON.
            Node::List(items) | Node::Tuple(items) | Node::Set(items) | Node::FrozenSet(items) => {
                Value::Array(items.iter().map(|i| inner(*i)).collect::<Res<_>>()?)
            }
            Node::Dict(pairs) => {
                let mut map = Map::new();
                for (key, value) in pairs {
                    map.insert(self.json_key(*key, path)?, inner(*value)?);
                }
                Value::Object(map)
            }
            Node::Object { class, args, .. } => match class {
                // NamedTuple : objet JSON avec ses champs et `"class"`.
                PyClass::NetworkItem | PyClass::NetworkSlot | PyClass::Hint => {
                    let mut map = Map::new();
                    for (field, arg) in named_fields(*class).iter().zip(args) {
                        map.insert((*field).to_string(), inner(*arg)?);
                    }
                    let short = class.path().rsplit('.').next().unwrap_or_default();
                    map.insert("class".to_string(), Value::String(short.to_string()));
                    Value::Object(map)
                }
                // Énumérations entières : leur valeur.
                PyClass::SlotType | PyClass::HintStatus | PyClass::ClientStatus => {
                    Value::from(self.int(id, path)?)
                }
                // `Counter(dict)` : le dictionnaire.
                PyClass::Counter => match args.first() {
                    Some(dict) => inner(*dict)?,
                    None => Value::Object(Map::new()),
                },
            },
            Node::Global(_) => return Err(error(path, "classe seule impossible à écrire en JSON")),
        };
        Ok(value)
    }

    /// Clé de dictionnaire en JSON : les clés non textuelles sont converties
    /// comme le fait le module `json` de Python.
    fn json_key(&self, id: NodeId, path: &str) -> Res<String> {
        match self.node(id) {
            Node::Str(s) => Ok(s.clone()),
            Node::Bool(true) => Ok("true".to_string()),
            Node::Bool(false) => Ok("false".to_string()),
            Node::None => Ok("null".to_string()),
            Node::Int(i) => Ok(i.to_string()),
            Node::BigInt(bytes) => Ok(big_int_decimal(bytes)),
            Node::Float(f) => Ok(f.to_string()),
            Node::Object {
                class: PyClass::SlotType | PyClass::HintStatus | PyClass::ClientStatus,
                ..
            } => Ok(self.int(id, path)?.to_string()),
            _ => Err(error(
                path,
                "clé de dictionnaire impossible à écrire en JSON",
            )),
        }
    }
}

/// Texte d'une clé pour les chemins d'erreur.
fn key_text(node: &Node) -> String {
    match node {
        Node::Int(i) => i.to_string(),
        Node::Str(s) => s.clone(),
        _ => "?".to_string(),
    }
}

/// Noms des champs des NamedTuple autorisés, dans l'ordre (`NetUtils.py`).
fn named_fields(class: PyClass) -> &'static [&'static str] {
    match class {
        PyClass::NetworkItem => &["item", "location", "player", "flags"],
        PyClass::NetworkSlot => &["name", "game", "type", "group_members"],
        PyClass::Hint => &[
            "receiving_player",
            "finding_player",
            "location",
            "item",
            "found",
            "entrance",
            "item_flags",
            "status",
        ],
        _ => &[],
    }
}

/// Grand entier en nombre JSON exact (possible grâce à `arbitrary_precision`).
fn big_int_value(bytes: &[u8], path: &str) -> Res<Value> {
    serde_json::from_str(&big_int_decimal(bytes)).map_err(|e| error(path, e.to_string()))
}

/// Écrit en base 10 un entier en complément à deux, petit-boutiste.
fn big_int_decimal(bytes: &[u8]) -> String {
    let negative = bytes.last().is_some_and(|b| b & 0x80 != 0);
    // Valeur absolue, en gros-boutiste (chiffre le plus fort en premier).
    let mut magnitude: Vec<u8> = bytes.iter().rev().copied().collect();
    if negative {
        // Complément à deux : on inverse les bits puis on ajoute 1.
        for byte in magnitude.iter_mut() {
            *byte = !*byte;
        }
        for byte in magnitude.iter_mut().rev() {
            let (sum, overflow) = byte.overflowing_add(1);
            *byte = sum;
            if !overflow {
                break;
            }
        }
    }
    // Divisions successives par 10 ; chaque reste donne un chiffre.
    let mut digits = Vec::new();
    while magnitude.iter().any(|b| *b != 0) {
        let mut remainder: u32 = 0;
        for byte in magnitude.iter_mut() {
            let current = (remainder << 8) | u32::from(*byte);
            *byte = (current / 10) as u8;
            remainder = current % 10;
        }
        digits.push(char::from(b'0' + remainder as u8));
    }
    if digits.is_empty() {
        digits.push('0');
    }
    if negative {
        digits.push('-');
    }
    digits.iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::big_int_decimal;

    #[test]
    fn grands_entiers_en_decimal() {
        // 2**100 et -2**100, tels que Python les écrit (`pickle.encode_long`).
        let mut plus = vec![0u8; 12];
        plus.push(0x10);
        assert_eq!(big_int_decimal(&plus), "1267650600228229401496703205376");
        let mut minus = vec![0u8; 12];
        minus.push(0xf0);
        assert_eq!(big_int_decimal(&minus), "-1267650600228229401496703205376");
        assert_eq!(big_int_decimal(&[0xff]), "-1");
        assert_eq!(big_int_decimal(&[]), "0");
    }
}
