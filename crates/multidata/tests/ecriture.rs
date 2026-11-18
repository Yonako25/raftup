//! Écriture d'une seed `.archipelago` (ADR 0027, étape 2).
//!
//! - aller-retour : modèle → `.archipelago` → modèle redonne le même modèle ;
//! - mêmes octets à chaque écriture (NF12) ;
//! - chaque type Python écrit est relu à l'identique par notre lecteur ;
//! - test réel (`#[ignore]`) : le vrai Python d'Archipelago relit la seed écrite
//!   par Raftup avec `restricted_loads`, et en tire le même modèle que de la seed
//!   officielle.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use multidata::pickle::{parse, Limits};
use multidata::{
    load_archipelago, pickle_writer, write_archipelago, ArchiveLimits, MultiData, Node, PyClass,
    PyValue,
};
use serde_json::Value;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

fn seed_mit() -> MultiData {
    let bytes = fs::read(data_dir().join("seed-mit.archipelago")).unwrap();
    load_archipelago(&bytes, &ArchiveLimits::default()).unwrap()
}

#[test]
fn aller_retour_seed_mit() {
    let model = seed_mit();
    let written = write_archipelago(&model).unwrap();
    assert_eq!(written[0], 3, "octet de version du format");
    let reread = load_archipelago(&written, &ArchiveLimits::default()).unwrap();
    assert_eq!(reread, model);
}

#[test]
fn memes_octets_a_chaque_ecriture() {
    let model = seed_mit();
    assert_eq!(
        write_archipelago(&model).unwrap(),
        write_archipelago(&model).unwrap()
    );
}

#[test]
fn types_relus_a_l_identique() {
    let long_text = "é".repeat(200); // 400 octets : `BINUNICODE`
    let mut big = vec![0u8; 12];
    big.push(0x10); // 2**100
    let value = PyValue::List(vec![
        PyValue::None,
        PyValue::Bool(true),
        PyValue::Bool(false),
        PyValue::Int(-5),
        PyValue::Int(70_000),
        PyValue::Int(i64::MAX),
        PyValue::BigInt(big.clone()),
        PyValue::Float(-2.5),
        PyValue::str("court"),
        PyValue::Str(long_text.clone()),
        PyValue::Tuple(vec![]),
        PyValue::Tuple(vec![PyValue::Int(1)]),
        PyValue::Tuple((0..5).map(PyValue::Int).collect()),
        PyValue::Dict(vec![(PyValue::Int(1), PyValue::str("un"))]),
        PyValue::Set(vec![PyValue::Int(7)]),
        PyValue::Object {
            class: PyClass::HintStatus,
            args: vec![PyValue::Int(30)],
        },
    ]);
    let bytes = pickle_writer::encode(&value).unwrap();
    let doc = parse(&bytes, &Limits::default()).unwrap();
    let Node::List(items) = doc.node(doc.root()) else {
        panic!("liste attendue");
    };
    let node = |i: usize| doc.node(items[i]).clone();
    assert_eq!(node(0), Node::None);
    assert_eq!(node(1), Node::Bool(true));
    assert_eq!(node(2), Node::Bool(false));
    assert_eq!(node(3), Node::Int(-5));
    assert_eq!(node(4), Node::Int(70_000));
    // Le lecteur range `LONG1` dans `Int` quand la valeur tient dans un i64.
    assert_eq!(node(5), Node::Int(i64::MAX));
    assert_eq!(node(6), Node::BigInt(big));
    assert_eq!(node(7), Node::Float(-2.5));
    assert_eq!(node(8), Node::Str("court".to_string()));
    assert_eq!(node(9), Node::Str(long_text));
    assert!(matches!(node(10), Node::Tuple(t) if t.is_empty()));
    assert!(matches!(node(11), Node::Tuple(t) if t.len() == 1));
    assert!(matches!(node(12), Node::Tuple(t) if t.len() == 5));
    assert!(matches!(node(13), Node::Dict(d) if d.len() == 1));
    assert!(matches!(node(14), Node::Set(s) if s.len() == 1));
    assert!(matches!(
        node(15),
        Node::Object { class: PyClass::HintStatus, ref args, .. } if args.len() == 1
    ));
}

/// Seed écrite par Raftup, relue par le vrai code d'Archipelago 0.6.8 :
/// le modèle obtenu doit être celui de la seed officielle (`seed-mit.modele.json`).
///
/// ```text
/// RAFTUP_PYTHON=…\python.exe RAFTUP_ARCHIPELAGO=…\generator\0.6.8\archipelago
/// RAFTUP_SITE_PACKAGES=…\generator\0.6.8\site-packages (facultatif)
///   cargo test -p multidata --test ecriture -- --ignored
/// ```
#[test]
#[ignore = "demande RAFTUP_PYTHON et RAFTUP_ARCHIPELAGO"]
fn relue_par_le_vrai_python() {
    let python = std::env::var("RAFTUP_PYTHON").expect("RAFTUP_PYTHON non défini");
    let archipelago = std::env::var("RAFTUP_ARCHIPELAGO").expect("RAFTUP_ARCHIPELAGO non défini");
    let dir = std::env::temp_dir().join("raftup-multidata-ecriture");
    fs::create_dir_all(&dir).unwrap();
    let seed = dir.join("seed-raftup.archipelago");
    let out = dir.join("seed-raftup.modele.json");
    fs::write(&seed, write_archipelago(&seed_mit()).unwrap()).unwrap();

    let mut command = Command::new(python);
    command
        .arg("-I")
        .arg(data_dir().join("scripts/gen_model_expected.py"))
        .arg(&archipelago)
        .arg(&seed)
        .arg(&out);
    // Dépendances du générateur (`yaml`…), si elles sont à part.
    if let Ok(site_packages) = std::env::var("RAFTUP_SITE_PACKAGES") {
        command.arg(site_packages);
    }
    let status = command.status().unwrap();
    assert!(status.success(), "le script Python a échoué");

    let read = |path: &Path| -> Value { serde_json::from_slice(&fs::read(path).unwrap()).unwrap() };
    assert_eq!(read(&out), read(&data_dir().join("seed-mit.modele.json")));
}
