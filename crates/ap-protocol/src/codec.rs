//! Outils communs aux deux sens : lire le champ `cmd`, puis la structure du paquet.
//!
//! # Pourquoi on lit `cmd` à la main
//!
//! serde sait gérer seul ce genre d'enum (`#[serde(tag = "cmd")]`), mais avec la
//! fonctionnalité `arbitrary_precision` de serde_json, il n'arrive plus à lire les
//! nombres à virgule à l'intérieur (testé : « invalid type: map, expected f64 »).
//! On lit donc d'abord le paquet comme une valeur JSON brute, on regarde `cmd`,
//! puis on demande à serde la structure qui correspond.

use serde::de::DeserializeOwned;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

use crate::error::ProtocolError;

/// Sépare un paquet en deux : le nom de la commande et ses champs.
///
/// Les champs gardent `cmd` : on ne le retire que pour une commande connue
/// (voir [`parse`]), afin de garder intact un paquet inconnu.
pub(crate) fn split_cmd(value: Value) -> Result<(String, Map<String, Value>), ProtocolError> {
    let Value::Object(map) = value else {
        return Err(ProtocolError::NotAnObject);
    };
    match map.get("cmd") {
        Some(Value::String(cmd)) => Ok((cmd.clone(), map)),
        _ => Err(ProtocolError::MissingCmd),
    }
}

/// Lit la structure d'un paquet connu à partir de ses champs.
///
/// `T: DeserializeOwned` : n'importe quel type que serde sait lire et qui ne
/// garde pas de référence vers le texte d'origine.
pub(crate) fn parse<T: DeserializeOwned>(
    cmd: &str,
    mut map: Map<String, Value>,
) -> Result<T, ProtocolError> {
    // On retire `cmd` : les structures ne le contiennent pas.
    // `shift_remove` garde l'ordre des autres clés.
    map.shift_remove("cmd");
    serde_json::from_value(Value::Object(map)).map_err(|source| ProtocolError::InvalidPacket {
        cmd: cmd.to_string(),
        source,
    })
}

/// Paquet à écrire : le champ `cmd` suivi des champs de la structure.
///
/// `#[serde(flatten)]` recopie les champs de `packet` au même niveau que `cmd`.
/// `'a` : la structure emprunte le paquet le temps de l'écriture, sans le copier.
#[derive(Serialize)]
struct WithCmd<'a, T> {
    cmd: &'static str,
    #[serde(flatten)]
    packet: &'a T,
}

/// Écrit `packet` en ajoutant le champ `cmd` en tête.
pub(crate) fn with_cmd<T: Serialize, S: Serializer>(
    cmd: &'static str,
    packet: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    WithCmd { cmd, packet }.serialize(serializer)
}
