//! Conversion du modèle de partie ([`MultiData`]) en valeur Python à écrire.
//!
//! C'est l'inverse de `convert` : on reproduit le dictionnaire que construit le
//! générateur officiel (`Main.py`, Archipelago 0.6.8), avec les mêmes clés,
//! dans le même ordre, et les mêmes types Python (tuples, ensembles, objets
//! `NetworkSlot`, `Hint`…). Le serveur officiel peut ainsi le lire tel quel.

use ap_protocol::{Hint, NetworkSlot, NetworkVersion};
use serde_json::Value;

use crate::model::{GamePackage, MultiData};
use crate::pickle::PyClass;
use crate::pickle_writer::{decimal_to_twos_complement, PyValue};

impl MultiData {
    /// Le dictionnaire Python de la seed, prêt à être écrit en pickle.
    pub fn to_py_value(&self) -> PyValue {
        // Clés dans l'ordre de `Main.py`.
        let entries = vec![
            ("slot_data", slot_dict(&self.slot_data, json_to_py)),
            ("slot_info", slot_dict(&self.slots, network_slot)),
            ("connect_names", self.connect_names_value()),
            (
                "locations",
                slot_dict(&self.locations, |locations| {
                    PyValue::Dict(
                        locations
                            .iter()
                            .map(|(location, placed)| {
                                let value = PyValue::Tuple(vec![
                                    PyValue::Int(placed.item),
                                    PyValue::Int(i64::from(placed.player)),
                                    PyValue::Int(i64::from(placed.flags.0)),
                                ]);
                                (PyValue::Int(*location), value)
                            })
                            .collect(),
                    )
                }),
            ),
            // Le modèle ne garde pas `checks_in_area` (vide en 0.6.8, inutilisé
            // par le serveur) : on écrit un dictionnaire vide, comme `Main.py`.
            ("checks_in_area", PyValue::Dict(Vec::new())),
            ("server_options", self.server_options_value()),
            (
                "er_hint_data",
                slot_dict(&self.er_hint_data, |entries| {
                    PyValue::Dict(
                        entries
                            .iter()
                            .map(|(location, entrance)| {
                                (PyValue::Int(*location), PyValue::str(entrance))
                            })
                            .collect(),
                    )
                }),
            ),
            (
                "precollected_items",
                slot_dict(&self.precollected_items, |items| {
                    PyValue::List(items.iter().map(|item| PyValue::Int(*item)).collect())
                }),
            ),
            (
                "precollected_hints",
                // Un ensemble (`set`) de `Hint` par slot, comme dans `Main.py`.
                slot_dict(&self.precollected_hints, |hints| {
                    PyValue::Set(hints.iter().map(hint).collect())
                }),
            ),
            ("version", version(&self.generator_version)),
            (
                "tags",
                PyValue::List(self.tags.iter().map(PyValue::str).collect()),
            ),
            ("minimum_versions", self.minimum_versions_value()),
            ("seed_name", PyValue::str(&self.seed_name)),
            (
                "spheres",
                PyValue::List(
                    self.spheres
                        .iter()
                        .map(|sphere| {
                            slot_dict(sphere, |locations| {
                                PyValue::Set(locations.iter().map(|l| PyValue::Int(*l)).collect())
                            })
                        })
                        .collect(),
                ),
            ),
            (
                "datapackage",
                PyValue::Dict(
                    self.datapackage
                        .iter()
                        .map(|(game, package)| (PyValue::str(game), game_package(package)))
                        .collect(),
                ),
            ),
            ("race_mode", PyValue::Int(i64::from(self.race_mode))),
        ];
        PyValue::Dict(
            entries
                .into_iter()
                .map(|(key, value)| (PyValue::str(key), value))
                .collect(),
        )
    }

    fn connect_names_value(&self) -> PyValue {
        PyValue::Dict(
            self.connect_names
                .iter()
                .map(|(name, (team, slot))| {
                    let value = PyValue::Tuple(vec![
                        PyValue::Int(i64::from(*team)),
                        PyValue::Int(i64::from(*slot)),
                    ]);
                    (PyValue::str(name), value)
                })
                .collect(),
        )
    }

    fn minimum_versions_value(&self) -> PyValue {
        PyValue::Dict(vec![
            (
                PyValue::str("server"),
                version(&self.minimum_server_version),
            ),
            (
                PyValue::str("clients"),
                slot_dict(&self.minimum_client_versions, version),
            ),
        ])
    }

    /// Les options que garde le modèle ; les autres options du serveur
    /// officiel prennent leur valeur par défaut à la lecture.
    fn server_options_value(&self) -> PyValue {
        let o = &self.server_options;
        let text = |value: &Option<String>| match value {
            Some(text) => PyValue::str(text),
            None => PyValue::None,
        };
        let entries = vec![
            ("password", text(&o.password)),
            ("server_password", text(&o.server_password)),
            ("hint_cost", PyValue::Int(o.hint_cost)),
            (
                "location_check_points",
                PyValue::Int(o.location_check_points),
            ),
            ("release_mode", PyValue::str(&o.release_mode)),
            ("collect_mode", PyValue::str(&o.collect_mode)),
            ("remaining_mode", PyValue::str(&o.remaining_mode)),
            ("countdown_mode", PyValue::str(&o.countdown_mode)),
            ("disable_item_cheat", PyValue::Bool(o.disable_item_cheat)),
            ("compatibility", PyValue::Int(o.compatibility)),
        ];
        PyValue::Dict(
            entries
                .into_iter()
                .map(|(key, value)| (PyValue::str(key), value))
                .collect(),
        )
    }
}

/// Dictionnaire à clés de slot (`int`), valeurs converties par `f`.
///
/// `impl IntoIterator<Item = (&'a u32, &'a T)>` : accepte une `BTreeMap` par
/// référence, qu'on parcourt dans l'ordre croissant des slots.
fn slot_dict<'a, T: 'a>(
    map: impl IntoIterator<Item = (&'a u32, &'a T)>,
    f: impl Fn(&'a T) -> PyValue,
) -> PyValue {
    PyValue::Dict(
        map.into_iter()
            .map(|(slot, value)| (PyValue::Int(i64::from(*slot)), f(value)))
            .collect(),
    )
}

/// `(major, minor, build)`.
fn version(v: &NetworkVersion) -> PyValue {
    PyValue::Tuple(vec![
        PyValue::Int(i64::from(v.major)),
        PyValue::Int(i64::from(v.minor)),
        PyValue::Int(i64::from(v.build)),
    ])
}

/// Valeur d'une énumération entière (`SlotType(1)`, `HintStatus(30)`…).
fn enum_value(class: PyClass, value: u8) -> PyValue {
    PyValue::Object {
        class,
        args: vec![PyValue::Int(i64::from(value))],
    }
}

/// `NetworkSlot(name, game, type, group_members)`.
fn network_slot(slot: &NetworkSlot) -> PyValue {
    // Comme `Main.py` : `()` par défaut pour un joueur, une liste triée pour un groupe.
    let members = if slot.group_members.is_empty() {
        PyValue::Tuple(Vec::new())
    } else {
        PyValue::List(
            slot.group_members
                .iter()
                .map(|m| PyValue::Int(i64::from(*m)))
                .collect(),
        )
    };
    PyValue::Object {
        class: PyClass::NetworkSlot,
        args: vec![
            PyValue::str(&slot.name),
            PyValue::str(&slot.game),
            enum_value(PyClass::SlotType, slot.slot_type.0),
            members,
        ],
    }
}

/// `Hint(receiving_player, finding_player, location, item, found, entrance,
/// item_flags, status)`.
fn hint(h: &Hint) -> PyValue {
    PyValue::Object {
        class: PyClass::Hint,
        args: vec![
            PyValue::Int(i64::from(h.receiving_player)),
            PyValue::Int(i64::from(h.finding_player)),
            PyValue::Int(h.location),
            PyValue::Int(h.item),
            PyValue::Bool(h.found),
            PyValue::str(&h.entrance),
            PyValue::Int(i64::from(h.item_flags.0)),
            enum_value(PyClass::HintStatus, h.status.0),
        ],
    }
}

/// Paquet d'un jeu, clés dans l'ordre de `get_data_package_data`.
fn game_package(p: &GamePackage) -> PyValue {
    let names = |map: &indexmap::IndexMap<String, i64>| {
        PyValue::Dict(
            map.iter()
                .map(|(name, id)| (PyValue::str(name), PyValue::Int(*id)))
                .collect(),
        )
    };
    let groups = |map: &indexmap::IndexMap<String, Vec<String>>| {
        PyValue::Dict(
            map.iter()
                .map(|(name, members)| {
                    let members = PyValue::List(members.iter().map(PyValue::str).collect());
                    (PyValue::str(name), members)
                })
                .collect(),
        )
    };
    PyValue::Dict(vec![
        (
            PyValue::str("item_name_groups"),
            groups(&p.item_name_groups),
        ),
        (PyValue::str("item_name_to_id"), names(&p.item_name_to_id)),
        (
            PyValue::str("location_name_groups"),
            groups(&p.location_name_groups),
        ),
        (
            PyValue::str("location_name_to_id"),
            names(&p.location_name_to_id),
        ),
        (PyValue::str("checksum"), PyValue::str(&p.checksum)),
    ])
}

/// Valeur JSON (`slot_data`) en valeur Python de base, comme le fait
/// `convert_to_base_types` : objet → `dict`, tableau → `list`.
///
/// Relu par le serveur officiel puis renvoyé en JSON, le résultat redonne
/// exactement la même valeur JSON.
pub(crate) fn json_to_py(value: &Value) -> PyValue {
    match value {
        Value::Null => PyValue::None,
        Value::Bool(b) => PyValue::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                PyValue::Int(i)
            } else if let Some(bytes) = decimal_to_twos_complement(&n.to_string()) {
                // Entier plus grand qu'un `i64` (gardé exact grâce à
                // `arbitrary_precision`).
                PyValue::BigInt(bytes)
            } else {
                PyValue::Float(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        Value::String(s) => PyValue::str(s),
        Value::Array(items) => PyValue::List(items.iter().map(json_to_py).collect()),
        Value::Object(map) => PyValue::Dict(
            map.iter()
                .map(|(key, value)| (PyValue::str(key), json_to_py(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    // Tests : un unwrap() qui échoue fait simplement échouer le test.
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn nombres_json() {
        let value: Value =
            serde_json::from_str("[1, -2, 1.5, 18446744073709551616, -9223372036854775809]")
                .unwrap();
        let PyValue::List(items) = json_to_py(&value) else {
            panic!("liste attendue");
        };
        assert_eq!(items[0], PyValue::Int(1));
        assert_eq!(items[1], PyValue::Int(-2));
        assert_eq!(items[2], PyValue::Float(1.5));
        // 2**64 : 8 octets à 0, puis 1, puis l'octet de signe.
        assert_eq!(items[3], PyValue::BigInt(vec![0, 0, 0, 0, 0, 0, 0, 0, 1]));
        assert_eq!(
            items[4],
            PyValue::BigInt(vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0xff])
        );
    }
}
