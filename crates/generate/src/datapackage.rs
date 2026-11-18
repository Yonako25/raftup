//! Datapackage d'un jeu (noms et numéros des objets et des lieux) et son `checksum`.
//!
//! Le calcul reprend `World.get_data_package_data` et `data_package_checksum` d'Archipelago
//! 0.6.8 (`worlds/AutoWorld.py`) : les mods comparent ce `checksum` à leur cache, il doit
//! donc être identique à l'octet près.

use indexmap::IndexMap;
use multidata::GamePackage;
use serde_json::{json, Map, Value};
use sha1::{Digest, Sha1};

/// Description statique d'un jeu : objets, lieux et groupes nommés.
pub struct PackageSource<'a> {
    /// Objets `(nom, numéro)`, dans l'ordre de la table Python `item_name_to_id`.
    pub items: &'a [(&'a str, i64)],
    /// Lieux `(nom, numéro)`, dans l'ordre de `location_name_to_id`.
    pub locations: &'a [(&'a str, i64)],
    /// Groupes d'objets déclarés par le monde (sans « Everything », ajouté ici).
    pub item_groups: &'a [(&'a str, &'a [&'a str])],
    /// Groupes de lieux déclarés par le monde (sans « Everywhere », ajouté ici).
    pub location_groups: &'a [(&'a str, &'a [&'a str])],
}

/// Construit le datapackage d'un jeu, `checksum` compris.
pub fn game_package(source: &PackageSource<'_>) -> GamePackage {
    // Comme Archipelago : les numéros 0 (événements) sont retirés des tables.
    let item_name_to_id: IndexMap<String, i64> = source
        .items
        .iter()
        .filter(|(_, id)| *id != 0)
        .map(|(name, id)| (name.to_string(), *id))
        .collect();
    let location_name_to_id: IndexMap<String, i64> = source
        .locations
        .iter()
        .filter(|(_, id)| *id != 0)
        .map(|(name, id)| (name.to_string(), *id))
        .collect();

    let everything: Vec<&str> = item_name_to_id.keys().map(String::as_str).collect();
    let everywhere: Vec<&str> = location_name_to_id.keys().map(String::as_str).collect();
    let item_name_groups = sorted_groups(source.item_groups, ("Everything", &everything));
    let location_name_groups = sorted_groups(source.location_groups, ("Everywhere", &everywhere));

    let checksum = checksum(
        &item_name_groups,
        &item_name_to_id,
        &location_name_groups,
        &location_name_to_id,
    );
    GamePackage {
        item_name_to_id,
        location_name_to_id,
        item_name_groups,
        location_name_groups,
        checksum,
    }
}

/// Le datapackage du « jeu » Archipelago lui-même (objets et lieux spéciaux du serveur).
pub fn archipelago_package() -> GamePackage {
    game_package(&PackageSource {
        items: &[("Nothing", -1)],
        locations: &[("Cheat Console", -1), ("Server", -2)],
        item_groups: &[],
        location_groups: &[],
    })
}

/// Groupes triés par nom, membres triés, avec le groupe « tout » ajouté.
///
/// Python trie les chaînes par point de code ; l'ordre des `String` Rust (octets UTF-8)
/// donne le même résultat.
fn sorted_groups(
    declared: &[(&str, &[&str])],
    all: (&str, &[&str]),
) -> IndexMap<String, Vec<String>> {
    let mut groups: Vec<(String, Vec<String>)> = declared
        .iter()
        .map(|(name, members)| {
            (
                name.to_string(),
                members.iter().map(|m| m.to_string()).collect(),
            )
        })
        .collect();
    // Le groupe « tout » remplace un groupe déclaré du même nom, comme en Python.
    groups.retain(|(name, _)| name != all.0);
    groups.push((
        all.0.to_string(),
        all.1.iter().map(|m| m.to_string()).collect(),
    ));
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    groups
        .into_iter()
        .map(|(name, mut members)| {
            members.sort();
            // Python construit un `frozenset` : un nom en double n'apparaît qu'une fois.
            members.dedup();
            (name, members)
        })
        .collect()
}

/// SHA-1 du JSON compact (séparateurs `,` et `:`, caractères non ASCII gardés tels quels),
/// clés dans l'ordre alphabétique, comme `NetUtils.encode`.
fn checksum(
    item_name_groups: &IndexMap<String, Vec<String>>,
    item_name_to_id: &IndexMap<String, i64>,
    location_name_groups: &IndexMap<String, Vec<String>>,
    location_name_to_id: &IndexMap<String, i64>,
) -> String {
    // `serde_json` garde l'ordre d'insertion (option `preserve_order` du workspace).
    let mut root = Map::new();
    root.insert("item_name_groups".into(), json!(item_name_groups));
    root.insert("item_name_to_id".into(), json!(item_name_to_id));
    root.insert("location_name_groups".into(), json!(location_name_groups));
    root.insert("location_name_to_id".into(), json!(location_name_to_id));
    let text = Value::Object(root).to_string();
    let digest = Sha1::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
