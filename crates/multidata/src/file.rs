//! Lecture d'un fichier avec une taille maximale.
//!
//! `std::fs::read` charge tout le fichier, quelle que soit sa taille : un
//! fichier de plusieurs Go choisi par erreur remplirait la mémoire. Ici, la
//! taille est vérifiée avant de lire, puis pendant la lecture (un fichier peut
//! grandir entre les deux).

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

/// Taille maximale d'un fichier `.archipelago` : la même que la taille
/// décompressée acceptée par défaut (`ArchiveLimits`, NF08).
pub const MAX_SEED_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Lit tout le fichier, s'il ne dépasse pas `max` octets.
///
/// Trop gros : erreur de genre `InvalidData` (`FileTooLarge` demande Rust
/// 1.83, le projet vise 1.80), avec le message « fichier trop gros … ».
pub fn read_file_bounded(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    if size > max {
        return Err(too_large(size, max));
    }
    // `take(max + 1)` : on lit au plus un octet de trop, juste assez pour
    // savoir que le fichier a grandi au-delà de la limite.
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(too_large(bytes.len() as u64, max));
    }
    Ok(bytes)
}

fn too_large(size: u64, max: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("fichier trop gros ({size} octets, au plus {max})"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, len: usize) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("raftup-file-{name}"));
        std::fs::write(&path, vec![b'x'; len]).expect("écriture du fichier de test");
        path
    }

    #[test]
    fn reads_a_small_file() {
        let path = temp_file("petit", 10);
        assert_eq!(read_file_bounded(&path, 10).expect("lecture").len(), 10);
    }

    #[test]
    fn refuses_a_file_too_large() {
        let path = temp_file("gros", 11);
        let error = read_file_bounded(&path, 10).expect_err("doit refuser");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("trop gros"));
    }
}
