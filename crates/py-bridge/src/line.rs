//! Lecture d'une ligne bornée en taille : un worker bavard ne doit pas saturer la mémoire.

use std::io::{self, BufRead, Read};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LineError {
    #[error("ligne trop longue : plus de {max} octets")]
    TooLong { max: usize },
    #[error("la ligne n'est pas de l'UTF-8 valide")]
    InvalidUtf8,
    #[error("erreur de lecture : {0}")]
    Io(#[from] io::Error),
}

/// Lit une ligne de `max` octets au plus (hors fin de ligne), en octets bruts.
///
/// Renvoie `Ok(None)` à la fin du flux. Le `\n` final, puis un éventuel `\r`, sont retirés.
pub fn read_bounded_bytes<R: BufRead>(
    reader: &mut R,
    max: usize,
) -> Result<Option<Vec<u8>>, LineError> {
    let mut buf: Vec<u8> = Vec::new();
    // `take` limite la lecture à max+2 octets (+2 pour pouvoir lire le `\r\n` d'une ligne de
    // exactement `max` octets). `by_ref()` prête le lecteur au lieu de le consommer.
    let limit = (max as u64).saturating_add(2);
    let n = reader.by_ref().take(limit).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    }
    // Plus de `max` octets utiles : soit la ligne est trop longue, soit la limite l'a coupée.
    if buf.len() > max {
        return Err(LineError::TooLong { max });
    }
    Ok(Some(buf))
}

/// Comme `read_bounded_bytes`, mais exige de l'UTF-8 valide.
pub fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    max: usize,
) -> Result<Option<String>, LineError> {
    match read_bounded_bytes(reader, max)? {
        None => Ok(None),
        Some(buf) => String::from_utf8(buf)
            .map(Some)
            .map_err(|_| LineError::InvalidUtf8),
    }
}
