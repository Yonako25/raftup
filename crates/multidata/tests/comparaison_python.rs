//! Le lecteur Rust doit donner exactement ce que donne Python.
//!
//! Pour chaque fichier de `tests/data/`, le résultat attendu
//! (`<fichier>.attendu.json`) a été calculé par Python : la seed avec la fonction
//! officielle `restricted_loads` du serveur, les petits pickles avec `pickle.loads`.
//! Voir `tests/data/README.md`.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use multidata::pickle::{parse, Limits};
use multidata::{read_archipelago, ArchiveLimits, Node, NodeId, PickleDocument};
use serde_json::{json, Value};

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Forme JSON « canonique », la même que celle du script Python.
fn canon(doc: &PickleDocument, id: NodeId, depth: usize) -> Value {
    assert!(depth < 1000, "imbrication trop profonde (cycle ?)");
    let all = |items: &[NodeId]| -> Vec<Value> {
        items.iter().map(|i| canon(doc, *i, depth + 1)).collect()
    };
    match doc.node(id) {
        Node::None => Value::Null,
        Node::Bool(b) => json!(b),
        Node::Int(i) => json!(i),
        Node::BigInt(bytes) => json!({ "bigint": hex(bytes) }),
        Node::Float(f) => json!({ "float": hex(&f.to_le_bytes()) }),
        Node::Str(s) => json!(s),
        Node::Bytes(bytes) => json!({ "bytes": hex(bytes) }),
        Node::List(items) => json!({ "list": all(items) }),
        Node::Tuple(items) => json!({ "tuple": all(items) }),
        Node::Dict(pairs) => {
            let pairs: Vec<Value> = pairs
                .iter()
                .map(|(k, v)| json!([canon(doc, *k, depth + 1), canon(doc, *v, depth + 1)]))
                .collect();
            json!({ "dict": pairs })
        }
        Node::Set(items) => json!({ "set": all(items) }),
        Node::FrozenSet(items) => json!({ "frozenset": all(items) }),
        Node::Global(global) => panic!("classe isolée inattendue : {global:?}"),
        Node::Object { class, args, state } => json!({
            "object": class.path(),
            "args": all(args),
            "state": state.map(|s| canon(doc, s, depth + 1)),
        }),
    }
}

/// Un ensemble n'a pas d'ordre : on trie ses éléments des deux côtés.
fn sort_sets(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(sort_sets),
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                sort_sets(inner);
                if key == "set" || key == "frozenset" {
                    if let Value::Array(items) = inner {
                        items.sort_by_key(|v| v.to_string());
                    }
                }
            }
        }
        _ => {}
    }
}

#[test]
fn tous_les_vecteurs_comme_python() {
    let mut count = 0;
    for entry in fs::read_dir(data_dir()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".json") || name.ends_with(".md") {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        let expected_text =
            fs::read_to_string(data_dir().join(format!("{name}.attendu.json"))).unwrap();
        let mut expected: Value = serde_json::from_str(&expected_text).unwrap();
        count += 1;

        // Cas où le serveur officiel refuse le fichier : Rust doit refuser aussi.
        if let Some(official_error) = expected.get("erreur") {
            let result = if name.ends_with(".archipelago") {
                read_archipelago(&bytes, &ArchiveLimits::default()).map(|_| ())
            } else {
                parse(&bytes, &Limits::default())
                    .map(|_| ())
                    .map_err(Into::into)
            };
            assert!(
                result.is_err(),
                "{name} : Python refuse ({official_error}), Rust accepte"
            );
            continue;
        }

        let doc = if name.ends_with(".archipelago") {
            read_archipelago(&bytes, &ArchiveLimits::default()).unwrap()
        } else {
            parse(&bytes, &Limits::default()).unwrap()
        };
        let mut ours = canon(&doc, doc.root(), 0);
        sort_sets(&mut ours);
        sort_sets(&mut expected);
        assert!(ours == expected, "{name} : résultat différent de Python");
    }
    assert!(count >= 10, "seulement {count} vecteurs trouvés");
}

/// La valeur partagée n'est pas copiée : les deux clés désignent le même numéro.
#[test]
fn reference_partagee_sans_copie() {
    let bytes = fs::read(data_dir().join("partage-p4.pickle")).unwrap();
    let doc = parse(&bytes, &Limits::default()).unwrap();
    let Node::Dict(pairs) = doc.node(doc.root()) else {
        panic!("dictionnaire attendu");
    };
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].1, pairs[1].1);
}

#[test]
fn seed_premier_niveau() {
    let bytes = fs::read(data_dir().join("seed-mit.archipelago")).unwrap();
    let doc = read_archipelago(&bytes, &ArchiveLimits::default()).unwrap();
    let Node::Dict(pairs) = doc.node(doc.root()) else {
        panic!("dictionnaire attendu");
    };
    let keys: Vec<&str> = pairs
        .iter()
        .filter_map(|(k, _)| match doc.node(*k) {
            Node::Str(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    for expected in [
        "slot_data",
        "connect_names",
        "locations",
        "datapackage",
        "seed_name",
    ] {
        assert!(
            keys.contains(&expected),
            "clé {expected} absente : {keys:?}"
        );
    }
}
