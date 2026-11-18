//! Écrivain de pickle Python (protocole 4), pour produire des seeds `.archipelago`.
//!
//! C'est le pendant du lecteur ([`crate::pickle`]) : on décrit la valeur à
//! écrire avec [`PyValue`], puis [`encode`] la transforme en octets.
//!
//! Sécurité : les seuls objets qu'on sait écrire sont ceux de la liste blanche
//! du lecteur ([`PyClass`]). Le type [`PyValue`] ne permet pas d'en écrire
//! d'autres : un fichier écrit par Raftup est donc toujours relisible par le
//! serveur officiel (`RestrictedUnpickler`) et par notre propre lecteur.
//!
//! On écrit les mêmes opcodes que Python 3.12 (`pickle.dumps`, protocole 4),
//! sans les blocs `FRAME` (facultatifs) ni le mémo (rien n'est partagé).

use crate::error::WriteError;
use crate::pickle::PyClass;

/// Profondeur maximale d'imbrication écrite (comme le lecteur JSON de `convert`).
pub const MAX_DEPTH: usize = 512;

/// Une valeur Python à écrire.
#[derive(Debug, Clone, PartialEq)]
pub enum PyValue {
    None,
    Bool(bool),
    Int(i64),
    /// Entier qui ne tient pas dans un `i64` : octets en petit-boutiste,
    /// complément à deux, sans octet inutile (même forme que [`crate::Node::BigInt`]).
    BigInt(Vec<u8>),
    Float(f64),
    Str(String),
    List(Vec<PyValue>),
    Tuple(Vec<PyValue>),
    /// Paires clé → valeur, écrites dans cet ordre.
    Dict(Vec<(PyValue, PyValue)>),
    Set(Vec<PyValue>),
    /// Objet d'une classe de la liste blanche, avec ses arguments de construction.
    Object {
        class: PyClass,
        args: Vec<PyValue>,
    },
}

impl PyValue {
    /// Raccourci pour un texte.
    pub fn str(text: impl Into<String>) -> PyValue {
        PyValue::Str(text.into())
    }
}

// Opcodes utilisés (noms de `pickletools`).
const PROTO: u8 = 0x80;
const STOP: u8 = b'.';
const MARK: u8 = b'(';
const NONE: u8 = b'N';
const NEWTRUE: u8 = 0x88;
const NEWFALSE: u8 = 0x89;
const BININT1: u8 = b'K';
const BININT2: u8 = b'M';
const BININT: u8 = b'J';
const LONG1: u8 = 0x8a;
const LONG4: u8 = 0x8b;
const BINFLOAT: u8 = b'G';
const SHORT_BINUNICODE: u8 = 0x8c;
const BINUNICODE: u8 = b'X';
const EMPTY_LIST: u8 = b']';
const APPENDS: u8 = b'e';
const EMPTY_TUPLE: u8 = b')';
const TUPLE: u8 = b't';
const TUPLE1: u8 = 0x85;
const EMPTY_DICT: u8 = b'}';
const SETITEMS: u8 = b'u';
const EMPTY_SET: u8 = 0x8f;
const ADDITEMS: u8 = 0x90;
const STACK_GLOBAL: u8 = 0x93;
const REDUCE: u8 = b'R';
const NEWOBJ: u8 = 0x81;

/// Écrit une valeur en pickle (protocole 4).
pub fn encode(value: &PyValue) -> Result<Vec<u8>, WriteError> {
    let mut out = vec![PROTO, 4];
    write_value(&mut out, value, 0)?;
    out.push(STOP);
    Ok(out)
}

fn write_value(out: &mut Vec<u8>, value: &PyValue, depth: usize) -> Result<(), WriteError> {
    if depth > MAX_DEPTH {
        return Err(WriteError::TooDeep(MAX_DEPTH));
    }
    match value {
        PyValue::None => out.push(NONE),
        PyValue::Bool(true) => out.push(NEWTRUE),
        PyValue::Bool(false) => out.push(NEWFALSE),
        PyValue::Int(i) => write_int(out, *i),
        PyValue::BigInt(bytes) => write_long(out, bytes)?,
        PyValue::Float(f) => {
            // Python écrit les nombres à virgule en gros-boutiste.
            out.push(BINFLOAT);
            out.extend_from_slice(&f.to_be_bytes());
        }
        PyValue::Str(text) => write_str(out, text)?,
        PyValue::List(items) => {
            out.push(EMPTY_LIST);
            write_marked(out, items, APPENDS, depth)?;
        }
        PyValue::Tuple(items) => write_tuple(out, items, depth)?,
        PyValue::Dict(pairs) => {
            out.push(EMPTY_DICT);
            if !pairs.is_empty() {
                out.push(MARK);
                for (key, value) in pairs {
                    write_value(out, key, depth + 1)?;
                    write_value(out, value, depth + 1)?;
                }
                out.push(SETITEMS);
            }
        }
        PyValue::Set(items) => {
            out.push(EMPTY_SET);
            write_marked(out, items, ADDITEMS, depth)?;
        }
        PyValue::Object { class, args } => {
            write_global(out, *class)?;
            write_tuple(out, args, depth)?;
            // Un `NamedTuple` est recréé par `cls.__new__(cls, *args)` (NEWOBJ) ;
            // une énumération ou un `Counter` par un appel `cls(*args)` (REDUCE).
            // C'est ce que fait Python, et ce qu'accepte le lecteur.
            out.push(match class {
                PyClass::NetworkItem | PyClass::NetworkSlot | PyClass::Hint => NEWOBJ,
                PyClass::SlotType
                | PyClass::ClientStatus
                | PyClass::HintStatus
                | PyClass::Counter => REDUCE,
            });
        }
    }
    Ok(())
}

/// `MARK`, les éléments, puis l'opcode qui les ajoute au conteneur ; rien si vide.
fn write_marked(
    out: &mut Vec<u8>,
    items: &[PyValue],
    opcode: u8,
    depth: usize,
) -> Result<(), WriteError> {
    if items.is_empty() {
        return Ok(());
    }
    out.push(MARK);
    for item in items {
        write_value(out, item, depth + 1)?;
    }
    out.push(opcode);
    Ok(())
}

fn write_tuple(out: &mut Vec<u8>, items: &[PyValue], depth: usize) -> Result<(), WriteError> {
    match items.len() {
        0 => out.push(EMPTY_TUPLE),
        // 1 à 3 éléments : opcodes `TUPLE1`, `TUPLE2`, `TUPLE3` (0x85 à 0x87).
        n @ 1..=3 => {
            for item in items {
                write_value(out, item, depth + 1)?;
            }
            // `n` vaut au plus 3 : la conversion en `u8` ne déborde pas.
            out.push(TUPLE1 + (n as u8 - 1));
        }
        _ => {
            out.push(MARK);
            for item in items {
                write_value(out, item, depth + 1)?;
            }
            out.push(TUPLE);
        }
    }
    Ok(())
}

/// Entier, avec l'opcode le plus court, comme `save_long` de Python.
fn write_int(out: &mut Vec<u8>, value: i64) {
    if (0..=0xff).contains(&value) {
        out.push(BININT1);
        out.push(value as u8);
    } else if (0..=0xffff).contains(&value) {
        out.push(BININT2);
        out.extend_from_slice(&(value as u16).to_le_bytes());
    } else if let Ok(small) = i32::try_from(value) {
        out.push(BININT);
        out.extend_from_slice(&small.to_le_bytes());
    } else {
        let mut bytes = value.to_le_bytes().to_vec();
        trim_twos_complement(&mut bytes);
        out.push(LONG1);
        // Au plus 8 octets : tient dans un `u8`.
        out.push(bytes.len() as u8);
        out.extend_from_slice(&bytes);
    }
}

fn write_long(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), WriteError> {
    match u8::try_from(bytes.len()) {
        Ok(len) => {
            out.push(LONG1);
            out.push(len);
        }
        Err(_) => {
            let len = u32::try_from(bytes.len()).map_err(|_| WriteError::TooLarge)?;
            out.push(LONG4);
            out.extend_from_slice(&len.to_le_bytes());
        }
    }
    out.extend_from_slice(bytes);
    Ok(())
}

fn write_str(out: &mut Vec<u8>, text: &str) -> Result<(), WriteError> {
    let bytes = text.as_bytes();
    match u8::try_from(bytes.len()) {
        Ok(len) => {
            out.push(SHORT_BINUNICODE);
            out.push(len);
        }
        Err(_) => {
            let len = u32::try_from(bytes.len()).map_err(|_| WriteError::TooLarge)?;
            out.push(BINUNICODE);
            out.extend_from_slice(&len.to_le_bytes());
        }
    }
    out.extend_from_slice(bytes);
    Ok(())
}

/// Référence à une classe : module et nom, puis `STACK_GLOBAL`.
fn write_global(out: &mut Vec<u8>, class: PyClass) -> Result<(), WriteError> {
    // `path()` vaut toujours « module.Nom » : on coupe au dernier point.
    let (module, name) = class.path().rsplit_once('.').unwrap_or(("", class.path()));
    write_str(out, module)?;
    write_str(out, name)?;
    out.push(STACK_GLOBAL);
    Ok(())
}

/// Retire les octets de poids fort inutiles d'un entier en complément à deux,
/// petit-boutiste (comme `pickle.encode_long`).
///
/// Un octet `0x00` (ou `0xff`) en tête est inutile si l'octet suivant porte déjà
/// le bon signe (bit de poids fort à 0, ou à 1).
pub(crate) fn trim_twos_complement(bytes: &mut Vec<u8>) {
    while bytes.len() > 1 {
        let last = bytes[bytes.len() - 1];
        let sign_of_next = bytes[bytes.len() - 2] & 0x80 != 0;
        let useless = (last == 0x00 && !sign_of_next) || (last == 0xff && sign_of_next);
        if !useless {
            break;
        }
        bytes.pop();
    }
}

/// Écrit un entier décimal (« 123 », « -456 ») en complément à deux,
/// petit-boutiste, sans octet inutile. `None` si le texte n'est pas un entier.
pub(crate) fn decimal_to_twos_complement(text: &str) -> Option<Vec<u8>> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Valeur absolue en petit-boutiste : pour chaque chiffre, on multiplie
    // par 10 et on ajoute le chiffre.
    let mut magnitude: Vec<u8> = vec![0];
    for digit in digits.bytes() {
        let mut carry = u32::from(digit - b'0');
        for byte in magnitude.iter_mut() {
            let current = u32::from(*byte) * 10 + carry;
            *byte = (current & 0xff) as u8;
            carry = current >> 8;
        }
        while carry > 0 {
            magnitude.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    // Un octet de signe à 0, pour que le nombre reste positif.
    magnitude.push(0);
    if negative {
        // Complément à deux : on inverse les bits puis on ajoute 1.
        for byte in magnitude.iter_mut() {
            *byte = !*byte;
        }
        for byte in magnitude.iter_mut() {
            let (sum, overflow) = byte.overflowing_add(1);
            *byte = sum;
            if !overflow {
                break;
            }
        }
    }
    trim_twos_complement(&mut magnitude);
    Some(magnitude)
}

#[cfg(test)]
mod tests {
    // Tests : un unwrap() qui échoue fait simplement échouer le test.
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn entiers_comme_python() {
        // Octets de `pickle.dumps(x, 4)` sans l'en-tête ni le `.` final.
        let body = |v: i64| {
            let bytes = encode(&PyValue::Int(v)).unwrap();
            bytes[2..bytes.len() - 1].to_vec()
        };
        assert_eq!(body(0), b"K\x00");
        assert_eq!(body(255), b"K\xff");
        assert_eq!(body(256), b"M\x00\x01");
        assert_eq!(body(-1), b"J\xff\xff\xff\xff");
        assert_eq!(body(1 << 31), b"\x8a\x05\x00\x00\x00\x80\x00");
        assert_eq!(body(-(1 << 31) - 1), b"\x8a\x05\xff\xff\xff\x7f\xff");
        assert_eq!(body(i64::MIN), b"\x8a\x08\x00\x00\x00\x00\x00\x00\x00\x80");
    }

    #[test]
    fn decimal_en_complement_a_deux() {
        // Valeurs de `pickle.encode_long`.
        assert_eq!(decimal_to_twos_complement("127").unwrap(), [0x7f]);
        assert_eq!(decimal_to_twos_complement("128").unwrap(), [0x80, 0x00]);
        assert_eq!(decimal_to_twos_complement("-128").unwrap(), [0x80]);
        assert_eq!(decimal_to_twos_complement("-256").unwrap(), [0x00, 0xff]);
        let mut two_100 = vec![0u8; 12];
        two_100.push(0x10);
        assert_eq!(
            decimal_to_twos_complement("1267650600228229401496703205376").unwrap(),
            two_100
        );
        assert_eq!(decimal_to_twos_complement("1.5"), None);
        assert_eq!(decimal_to_twos_complement("-"), None);
    }

    #[test]
    fn imbrication_trop_profonde_refusee() {
        let mut value = PyValue::None;
        for _ in 0..=MAX_DEPTH + 1 {
            value = PyValue::List(vec![value]);
        }
        assert!(matches!(encode(&value), Err(WriteError::TooDeep(_))));
    }
}
