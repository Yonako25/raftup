use std::io::Cursor;

use py_bridge::{read_bounded_line, LineError};

fn read(data: &[u8], max: usize) -> Result<Option<String>, LineError> {
    read_bounded_line(&mut Cursor::new(data.to_vec()), max)
}

#[test]
fn ligne_normale() {
    assert_eq!(read(b"bonjour\n", 100).unwrap(), Some("bonjour".into()));
}

#[test]
fn crlf_retire() {
    assert_eq!(read(b"bonjour\r\n", 100).unwrap(), Some("bonjour".into()));
}

#[test]
fn eof_donne_none() {
    assert_eq!(read(b"", 100).unwrap(), None);
}

#[test]
fn derniere_ligne_sans_saut() {
    assert_eq!(read(b"fin", 100).unwrap(), Some("fin".into()));
}

#[test]
fn plusieurs_lignes_a_la_suite() {
    let mut c = Cursor::new(b"a\nb\n".to_vec());
    assert_eq!(read_bounded_line(&mut c, 10).unwrap(), Some("a".into()));
    assert_eq!(read_bounded_line(&mut c, 10).unwrap(), Some("b".into()));
    assert_eq!(read_bounded_line(&mut c, 10).unwrap(), None);
}

#[test]
fn exactement_max_accepte() {
    assert_eq!(read(b"abcde\n", 5).unwrap(), Some("abcde".into()));
    assert_eq!(read(b"abcde", 5).unwrap(), Some("abcde".into()));
    assert_eq!(read(b"abcde\r\n", 5).unwrap(), Some("abcde".into()));
}

#[test]
fn max_plus_un_refuse() {
    assert!(matches!(
        read(b"abcdef\n", 5),
        Err(LineError::TooLong { max: 5 })
    ));
    assert!(matches!(
        read(b"abcdef", 5),
        Err(LineError::TooLong { max: 5 })
    ));
    assert!(matches!(
        read(b"abcdefghijkl\n", 5),
        Err(LineError::TooLong { max: 5 })
    ));
}

#[test]
fn utf8_invalide() {
    assert!(matches!(
        read(b"\xff\xfe\n", 100),
        Err(LineError::InvalidUtf8)
    ));
}

#[test]
fn utf8_multi_octets_accepte() {
    assert_eq!(read("été\n".as_bytes(), 100).unwrap(), Some("été".into()));
}
