//! Fichiers piégés : ils doivent être refusés proprement, sans rien exécuter (NF08).

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used)]

use std::io::Write;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use multidata::pickle::{parse, Limits};
use multidata::{read_archipelago, ArchiveError, ArchiveLimits, Node, PickleErrorKind};

fn kind(data: &[u8]) -> PickleErrorKind {
    parse(data, &Limits::default()).unwrap_err().kind
}

/// Le piège classique : `os.system("…")` via l'opcode GLOBAL puis REDUCE.
#[test]
fn os_system_refuse() {
    let piege = b"cos\nsystem\n(S'echo pirate'\ntR.";
    match kind(piege) {
        PickleErrorKind::ForbiddenGlobal { module, name } => {
            assert_eq!((module.as_str(), name.as_str()), ("os", "system"));
        }
        other => panic!("erreur inattendue : {other}"),
    }
}

/// Même piège avec le protocole 4 (STACK_GLOBAL) et `builtins.eval`.
#[test]
fn builtins_eval_refuse() {
    let mut piege = vec![0x80, 4];
    piege.extend([0x8c, 8]);
    piege.extend(b"builtins");
    piege.extend([0x8c, 4]);
    piege.extend(b"eval");
    piege.extend([0x93, b'.']);
    assert!(matches!(
        kind(&piege),
        PickleErrorKind::ForbiddenGlobal { .. }
    ));
}

/// Les opcodes qui appellent du code sans passer par GLOBAL sont inconnus ici.
#[test]
fn opcodes_dangereux_refuses() {
    // PERSID, INST, OBJ, NEWOBJ_EX
    for opcode in [b'P', b'i', b'o', 0x92] {
        assert!(
            matches!(kind(&[opcode, b'.']), PickleErrorKind::UnknownOpcode(o) if o == opcode),
            "opcode {opcode:#04x}"
        );
    }
}

/// Une « bombe » : 65 Mio de zéros compressés en quelques dizaines de Ko.
#[test]
fn bombe_de_compression_refusee() {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    let zeros = vec![0u8; 1024 * 1024];
    for _ in 0..65 {
        encoder.write_all(&zeros).unwrap();
    }
    let mut file = vec![3u8];
    file.extend(encoder.finish().unwrap());
    assert!(file.len() < 200 * 1024);
    assert!(matches!(
        read_archipelago(&file, &ArchiveLimits::default()),
        Err(ArchiveError::TooLarge { .. })
    ));
}

#[test]
fn format_trop_recent_refuse() {
    assert!(matches!(
        read_archipelago(&[4, 0x78, 0x9c], &ArchiveLimits::default()),
        Err(ArchiveError::UnsupportedFormat(4))
    ));
    assert!(matches!(
        read_archipelago(&[], &ArchiveLimits::default()),
        Err(ArchiveError::Empty)
    ));
}

#[test]
fn donnees_tronquees() {
    // SHORT_BINUNICODE annonce 10 octets, il n'y en a que 3.
    assert!(matches!(
        kind(&[0x8c, 10, b'a', b'b', b'c']),
        PickleErrorKind::UnexpectedEof
    ));
    // Pas d'opcode STOP.
    assert!(matches!(kind(b"N"), PickleErrorKind::UnexpectedEof));
}

#[test]
fn pile_et_memo_invalides() {
    // APPEND sur une pile vide.
    assert!(matches!(kind(b"a."), PickleErrorKind::StackUnderflow));
    // BINGET d'une entrée jamais mémorisée.
    assert!(matches!(kind(b"h\x05."), PickleErrorKind::BadMemo(5)));
    // APPEND sur autre chose qu'une liste.
    assert!(matches!(kind(b"NNa."), PickleErrorKind::WrongType(_)));
    // Deux valeurs à la fin au lieu d'une.
    assert!(matches!(kind(b"NN."), PickleErrorKind::BadFinalStack));
    // Des octets après STOP.
    assert!(matches!(kind(b"N.N"), PickleErrorKind::TrailingData));
}

#[test]
fn trop_d_elements() {
    let limits = Limits {
        max_nodes: 10,
        ..Limits::default()
    };
    // Une liste de 20 `None`.
    let mut data = b"](".to_vec();
    data.extend([b'N'; 20]);
    data.extend(b"e.");
    let err = parse(&data, &limits).unwrap_err();
    assert!(matches!(err.kind, PickleErrorKind::TooManyNodes(10)));
}

#[test]
fn entier_geant_refuse() {
    // LONG4 annonçant 1 Mio d'octets.
    let mut data = vec![0x8b];
    data.extend(1_048_576i32.to_le_bytes());
    assert!(matches!(
        kind(&data),
        PickleErrorKind::IntegerTooLarge(1_048_576)
    ));
}

/// 100 000 listes imbriquées : le lecteur ne s'appuie pas sur la récursion,
/// il ne déborde donc pas de la pile d'appels.
#[test]
fn imbrication_profonde_sans_plantage() {
    let depth = 100_000;
    let mut data = Vec::new();
    for _ in 0..depth {
        data.extend(b"](");
    }
    data.push(b'N');
    data.extend(vec![b'e'; depth]);
    data.push(b'.');
    let doc = parse(&data, &Limits::default()).unwrap();
    assert_eq!(doc.len(), depth + 1);
}

/// Une liste qui se contient elle-même : lue sans boucle infinie.
#[test]
fn liste_cyclique() {
    // EMPTY_LIST, MEMOIZE, BINGET 0, APPEND, STOP
    let doc = parse(&[b']', 0x94, b'h', 0, b'a', b'.'], &Limits::default()).unwrap();
    assert_eq!(doc.node(doc.root()), &Node::List(vec![doc.root()]));
}
