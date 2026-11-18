//! Instance unique du launcher : verrou et « boîte aux lettres » des liens (ADR 0020, décision 5).

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Taille maximale d'un lien transmis (4 Kio).
const MAX_LINK_BYTES: usize = 4096;
/// Nombre maximal de liens lus par appel de `take_inbox`.
const MAX_PER_CALL: usize = 10;
const INBOX: &str = "inbox";
const LINK_EXT: &str = "link";

/// Résultat de `acquire` : ce processus est le premier, ou une autre instance tourne déjà.
#[derive(Debug)]
pub enum Instance {
    /// Première instance : il faut garder le `Guard` vivant pendant toute la durée du programme.
    First(Guard),
    /// Une autre instance tient déjà le verrou.
    Busy,
}

/// Garde le fichier verrouillé. Le verrou tombe quand le `Guard` est détruit (`Drop` de `File`
/// ferme le fichier) ou quand le processus meurt : pas de verrou périmé à nettoyer.
#[derive(Debug)]
pub struct Guard {
    // Le champ n'est jamais lu : il sert uniquement à garder le fichier (donc le verrou) ouvert.
    _file: File,
}

/// Tente de devenir l'instance unique en verrouillant `dir/raftup.lock`.
pub fn acquire(dir: &Path) -> io::Result<Instance> {
    fs::create_dir_all(dir)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("raftup.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Instance::First(Guard { _file: file })),
        Err(TryLockError::WouldBlock) => Ok(Instance::Busy),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// Dépose `link` dans la boîte aux lettres de l'instance déjà lancée.
pub fn forward(dir: &Path, link: &str) -> io::Result<()> {
    if link.len() > MAX_LINK_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "lien trop long (4 Kio au plus)",
        ));
    }
    let inbox = dir.join(INBOX);
    fs::create_dir_all(&inbox)?;
    let mut ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let pid = std::process::id();
    // Largeur fixe : l'ordre alphabétique des noms est alors l'ordre chronologique.
    let mut name = format!("{ms:015}-{pid}.{LINK_EXT}");
    // Deux envois dans la même milliseconde : on avance l'horodatage plutôt que d'écraser.
    while inbox.join(&name).exists() {
        ms += 1;
        name = format!("{ms:015}-{pid}.{LINK_EXT}");
    }
    let tmp = inbox.join(format!("{name}.tmp"));
    let mut file = File::create(&tmp)?;
    file.write_all(link.as_bytes())?;
    file.sync_all()?;
    drop(file);
    // Le renommage est atomique : le lecteur ne voit jamais un fichier à moitié écrit.
    fs::rename(&tmp, inbox.join(&name))
}

/// Lit puis supprime les liens en attente (10 au plus, triés par nom). Aucune erreur remontée.
pub fn take_inbox(dir: &Path) -> Vec<String> {
    let inbox = dir.join(INBOX);
    let entries = match fs::read_dir(&inbox) {
        Ok(e) => e,
        // Pas de dossier : personne n'a rien envoyé.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!("lecture de {} impossible : {e}", inbox.display());
            return Vec::new();
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some(LINK_EXT))
        .collect();
    paths.sort();
    paths.truncate(MAX_PER_CALL);

    let mut links = Vec::new();
    for path in paths {
        match read_link_file(&path) {
            Ok(Some(text)) => links.push(text),
            Ok(None) => tracing::warn!("lien ignoré (trop gros ou illisible) : {}", path.display()),
            Err(e) => tracing::warn!("lecture de {} impossible : {e}", path.display()),
        }
        if let Err(e) = fs::remove_file(&path) {
            tracing::warn!("suppression de {} impossible : {e}", path.display());
        }
    }
    links
}

/// `Ok(None)` si le fichier est trop gros ou n'est pas de l'UTF-8.
fn read_link_file(path: &Path) -> io::Result<Option<String>> {
    // On ne lit jamais plus de 4 Kio + 1 octet : un fichier géant ne remplit pas la mémoire.
    let mut buf = Vec::new();
    File::open(path)?
        .take(MAX_LINK_BYTES as u64 + 1)
        .read_to_end(&mut buf)?;
    if buf.len() > MAX_LINK_BYTES {
        return Ok(None);
    }
    Ok(String::from_utf8(buf).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_is_busy_until_guard_dropped() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("instance");
        let first = acquire(&dir)?;
        assert!(matches!(first, Instance::First(_)));
        assert!(matches!(acquire(&dir)?, Instance::Busy));
        drop(first);
        assert!(matches!(acquire(&dir)?, Instance::First(_)));
        Ok(())
    }

    #[test]
    fn forwarded_link_is_read_once() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        forward(tmp.path(), "raftup://join?x=1")?;
        assert_eq!(take_inbox(tmp.path()), vec!["raftup://join?x=1"]);
        assert!(take_inbox(tmp.path()).is_empty());
        Ok(())
    }

    #[test]
    fn links_come_back_in_order() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        for i in 0..5 {
            forward(tmp.path(), &format!("lien-{i}"))?;
        }
        let expected: Vec<String> = (0..5).map(|i| format!("lien-{i}")).collect();
        assert_eq!(take_inbox(tmp.path()), expected);
        Ok(())
    }

    #[test]
    fn at_most_ten_per_call() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        for i in 0..13 {
            forward(tmp.path(), &format!("l{i}"))?;
        }
        assert_eq!(take_inbox(tmp.path()).len(), 10);
        assert_eq!(take_inbox(tmp.path()).len(), 3);
        Ok(())
    }

    #[test]
    fn oversized_and_non_utf8_files_are_dropped() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let too_long = "a".repeat(4097);
        let err = forward(tmp.path(), &too_long).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let inbox = tmp.path().join(INBOX);
        fs::create_dir_all(&inbox)?;
        let big = inbox.join("000-1.link");
        let bad = inbox.join("001-1.link");
        fs::write(&big, vec![b'a'; 4097])?;
        fs::write(&bad, [0xff, 0xfe])?;
        forward(tmp.path(), "ok")?;
        assert_eq!(take_inbox(tmp.path()), vec!["ok"]);
        assert!(!big.exists() && !bad.exists());
        Ok(())
    }

    #[test]
    fn tmp_files_and_directories_are_left_alone() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let inbox = tmp.path().join(INBOX);
        fs::create_dir_all(inbox.join("dossier.link"))?;
        fs::write(inbox.join("x.link.tmp"), "en cours")?;
        assert!(take_inbox(tmp.path()).is_empty());
        assert!(inbox.join("x.link.tmp").exists());
        assert!(inbox.join("dossier.link").is_dir());
        Ok(())
    }
}
