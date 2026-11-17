//! Empreinte SHA-256 partagée par le téléchargement, l'extraction, les reçus et le lockfile.
//!
//! Un seul type pour tout le launcher : on ne manipule jamais un hash comme un simple
//! texte, ce qui évite de comparer par erreur des majuscules et des minuscules.

use std::fmt;
use std::io::{self, Read};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256 as Hasher};

/// Une empreinte SHA-256 (32 octets). Écrite en hexadécimal minuscule (64 caractères).
///
/// `Copy` : le type est petit (32 octets), on peut le passer sans « emprunter ».
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256([u8; 32]);

/// Le texte lu n'est pas un SHA-256 valide.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseSha256Error {
    #[error("un sha256 fait 64 caractères hexadécimaux, pas {0}")]
    Length(usize),
    #[error("caractère non hexadécimal dans le sha256 : « {0} »")]
    NotHex(char),
}

impl Sha256 {
    /// Hash d'un bloc d'octets déjà en mémoire.
    pub fn of_bytes(data: &[u8]) -> Sha256 {
        let digest = Hasher::digest(data);
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&digest);
        Sha256(bytes)
    }

    /// Hash d'un flux lu par morceaux (un gros fichier n'est jamais chargé en entier).
    pub fn of_reader(reader: &mut impl Read) -> io::Result<Sha256> {
        let mut hasher = Hasher::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&hasher.finalize());
        Ok(Sha256(bytes))
    }

    /// Les 32 octets bruts.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lit un hash écrit en hexadécimal (majuscules ou minuscules acceptées).
    pub fn from_hex(text: &str) -> Result<Sha256, ParseSha256Error> {
        let chars: Vec<char> = text.chars().collect();
        if chars.len() != 64 {
            return Err(ParseSha256Error::Length(chars.len()));
        }
        let mut bytes = [0u8; 32];
        for (index, pair) in chars.chunks(2).enumerate() {
            let high = hex_value(pair[0])?;
            let low = hex_value(pair[1])?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Sha256(bytes))
    }
}

/// Valeur d'un chiffre hexadécimal.
fn hex_value(c: char) -> Result<u8, ParseSha256Error> {
    c.to_digit(16)
        .and_then(|v| u8::try_from(v).ok())
        .ok_or(ParseSha256Error::NotHex(c))
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// `Debug` affiche l'hexadécimal, plus lisible dans les messages de test.
impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256({self})")
    }
}

impl FromStr for Sha256 {
    type Err = ParseSha256Error;

    fn from_str(text: &str) -> Result<Sha256, ParseSha256Error> {
        Sha256::from_hex(text)
    }
}

// Dans les fichiers TOML et JSON, un hash est une simple chaîne hexadécimale.
impl Serialize for Sha256 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Sha256 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Sha256, D::Error> {
        let text = String::deserialize(deserializer)?;
        Sha256::from_hex(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn hash_connu() {
        assert_eq!(Sha256::of_bytes(b"abc").to_string(), ABC);
    }

    #[test]
    fn flux_et_octets_donnent_le_meme_hash() {
        let mut flux: &[u8] = b"abc";
        assert_eq!(
            Sha256::of_reader(&mut flux).unwrap(),
            Sha256::of_bytes(b"abc")
        );
    }

    #[test]
    fn aller_retour_hexadecimal() {
        let hash = Sha256::from_hex(ABC).unwrap();
        assert_eq!(hash.to_string(), ABC);
        assert_eq!(Sha256::from_hex(&ABC.to_uppercase()).unwrap(), hash);
    }

    #[test]
    fn refuse_un_hash_mal_forme() {
        assert_eq!(Sha256::from_hex("abc"), Err(ParseSha256Error::Length(3)));
        let faux = format!("g{}", &ABC[1..]);
        assert_eq!(Sha256::from_hex(&faux), Err(ParseSha256Error::NotHex('g')));
    }

    #[test]
    fn serde_en_json() {
        let hash = Sha256::from_hex(ABC).unwrap();
        let json = serde_json::to_string(&hash).unwrap();
        assert_eq!(json, format!("\"{ABC}\""));
        assert_eq!(serde_json::from_str::<Sha256>(&json).unwrap(), hash);
    }
}
