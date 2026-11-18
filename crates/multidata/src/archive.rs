//! Lecture d'un fichier `.archipelago` produit par le générateur officiel.
//!
//! Format (`MultiServer.py`, fonction `decompress`) :
//! - 1 octet : version du format (3 au maximum) ;
//! - le reste : un pickle Python compressé avec zlib.

use std::io::{Read, Write};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::error::{ArchiveError, WriteError};
use crate::model::MultiData;
use crate::pickle::{self, Limits, PickleDocument};
use crate::pickle_writer;

/// Version de format la plus récente acceptée (comme le serveur officiel 0.6.8).
pub const MAX_FORMAT_VERSION: u8 = 3;

/// Limites de sécurité pour lire un fichier `.archipelago`.
#[derive(Debug, Clone)]
pub struct ArchiveLimits {
    /// Taille maximale une fois décompressé : 64 Mio (NF08). Protège contre une
    /// « bombe » de compression (quelques Ko qui deviennent des Go).
    pub max_decompressed_bytes: usize,
    pub pickle: Limits,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        ArchiveLimits {
            max_decompressed_bytes: 64 * 1024 * 1024,
            pickle: Limits::default(),
        }
    }
}

/// Lit le contenu d'un fichier `.archipelago`.
pub fn read_archipelago(
    bytes: &[u8],
    limits: &ArchiveLimits,
) -> Result<PickleDocument, ArchiveError> {
    let (&format, compressed) = bytes.split_first().ok_or(ArchiveError::Empty)?;
    if format > MAX_FORMAT_VERSION {
        return Err(ArchiveError::UnsupportedFormat(format));
    }
    let raw = decompress(compressed, limits.max_decompressed_bytes)?;
    Ok(pickle::parse(&raw, &limits.pickle)?)
}

/// Version de format écrite par Raftup : la même que le générateur officiel 0.6.8.
pub const WRITTEN_FORMAT_VERSION: u8 = 3;

/// Écrit une seed en fichier `.archipelago` (comme `Main.py`) : l'octet de
/// version, puis le pickle compressé avec zlib au niveau 9.
///
/// Même modèle ⇒ mêmes octets : le pickle suit l'ordre du modèle et la
/// compression est déterministe.
pub fn write_archipelago(data: &MultiData) -> Result<Vec<u8>, WriteError> {
    let pickle = pickle_writer::encode(&data.to_py_value())?;
    let mut encoder = ZlibEncoder::new(vec![WRITTEN_FORMAT_VERSION], Compression::best());
    encoder.write_all(&pickle)?;
    Ok(encoder.finish()?)
}

/// Décompresse en s'arrêtant dès que la limite est dépassée.
fn decompress(compressed: &[u8], limit: usize) -> Result<Vec<u8>, ArchiveError> {
    let mut out = Vec::new();
    // `take(limit + 1)` : on lit au plus un octet de trop, juste assez pour
    // savoir que la limite est dépassée, sans jamais tout décompresser.
    let max = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    ZlibDecoder::new(compressed)
        .take(max)
        .read_to_end(&mut out)?;
    if out.len() > limit {
        return Err(ArchiveError::TooLarge { limit });
    }
    Ok(out)
}
