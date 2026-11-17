//! Désinstaller les mods d'un jeu à partir du reçu (erreurs : voir `InstallError`).
//!
//! Ordre (ADR 0015, « Réversibilité ») : étapes à l'envers ; pour chaque étape, fichiers
//! créés supprimés (si leur hash n'a pas changé), originaux restaurés depuis les
//! sauvegardes, dossiers vides supprimés ; le reçu est supprimé en dernier.
//!
//! Le reçu est un fichier relu depuis le disque : il n'est donc **pas fiable**. Tous ses
//! chemins sont validés avant la moindre suppression, et on ne suit jamais un lien.
//!
//! La désinstallation peut être relancée après une interruption : un fichier déjà
//! supprimé ou déjà restauré n'est pas une erreur.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::archive::safe_relative_path;
use crate::hash::Sha256;
use crate::install::InstallError;
use crate::paths::Paths;
use crate::record::{FileRecord, InstallRecord, StepRecord};

/// Ce qu'une désinstallation a fait.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UninstallReport {
    /// Fichiers créés qui ont été supprimés.
    pub removed: usize,
    /// Originaux remis à leur place depuis les sauvegardes.
    pub restored: usize,
    /// Fichiers modifiés depuis l'installation (supprimés seulement avec `force`).
    pub conflicts: Vec<PathBuf>,
    /// Dossiers créés par l'installation mais gardés, car ils ne sont pas vides.
    pub kept_dirs: Vec<PathBuf>,
}

fn io_err(path: &Path, source: io::Error) -> InstallError {
    InstallError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Valide un chemin du reçu (relatif, sans `..`, sans lecteur…) avec la fonction
/// commune d'`archive.rs`.
fn validate_relative(path: &Path) -> Result<PathBuf, InstallError> {
    // Un chemin non UTF-8 ne peut pas venir de notre propre reçu : on le refuse.
    let text = path.to_str().unwrap_or("\u{fffd}");
    Ok(safe_relative_path(text)?)
}

/// Vérifie tout le reçu avant de toucher au disque.
fn validate_record(record: &InstallRecord) -> Result<(), InstallError> {
    for step in &record.steps {
        let root_ok = step.root.is_absolute()
            && step
                .root
                .components()
                .all(|c| !matches!(c, Component::ParentDir | Component::CurDir));
        if !root_ok {
            return Err(InstallError::UnsafeRoot(step.root.clone()));
        }
        for dir in &step.dirs {
            validate_relative(dir)?;
        }
        for file in &step.files {
            validate_relative(&file.path)?;
            if let Some(backup) = &file.backup {
                validate_relative(&backup.path)?;
            }
        }
    }
    Ok(())
}

/// Lien symbolique ou point de reparse Windows (jonction…).
pub(crate) fn is_link(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT
        const REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Regarde `root/relative` composant par composant avec `symlink_metadata` (qui ne
/// suit pas les liens). `Ok(None)` si le chemin n'existe pas ; erreur si un lien
/// se trouve en chemin.
pub(crate) fn inspect(root: &Path, relative: &Path) -> Result<Option<fs::Metadata>, InstallError> {
    let mut current = root.to_path_buf();
    let mut last = None;
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                if is_link(&meta) {
                    return Err(InstallError::Link(current));
                }
                last = Some(meta);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_err(&current, e)),
        }
    }
    Ok(last)
}

pub(crate) fn hash_file(path: &Path) -> Result<Sha256, InstallError> {
    let mut file = fs::File::open(path).map_err(|e| io_err(path, e))?;
    Sha256::of_reader(&mut file).map_err(|e| io_err(path, e))
}

/// Désinstalle tout ce que Raftup a posé pour `game_id`.
///
/// Un fichier modifié depuis l'installation est signalé dans `conflicts` et laissé en
/// place, sauf avec `force`. Tant qu'un conflit n'est pas résolu, le reçu est conservé
/// pour pouvoir relancer avec `force`.
pub fn uninstall(
    paths: &Paths,
    game_id: &str,
    force: bool,
) -> Result<UninstallReport, InstallError> {
    let record =
        InstallRecord::load(paths, game_id)?.ok_or_else(|| InstallError::NotInstalled {
            game_id: game_id.to_string(),
        })?;
    // Tout est validé AVANT de supprimer quoi que ce soit.
    validate_record(&record)?;

    let backups_root = paths.backups_dir(game_id);
    let mut report = UninstallReport::default();

    // `.rev()` : on défait dans l'ordre inverse de l'installation.
    for step in record.steps.iter().rev() {
        undo_step(step, &backups_root, force, &mut report)?;
    }

    prune_empty_dirs(&backups_root);
    if force || report.conflicts.is_empty() {
        // Le reçu part en dernier : s'il reste, on peut relancer.
        InstallRecord::remove(paths, game_id)?;
    }
    Ok(report)
}

fn undo_step(
    step: &StepRecord,
    backups_root: &Path,
    force: bool,
    report: &mut UninstallReport,
) -> Result<(), InstallError> {
    for file in step.files.iter().rev() {
        undo_file(&step.root, file, backups_root, force, report)?;
    }

    // Dossiers les plus profonds d'abord, pour qu'un parent devienne vide.
    let mut dirs: Vec<&PathBuf> = step.dirs.iter().collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for dir in dirs {
        let Some(meta) = inspect(&step.root, dir)? else {
            continue; // déjà supprimé
        };
        let full = step.root.join(dir);
        if !meta.is_dir() {
            continue;
        }
        let is_empty = fs::read_dir(&full)
            .map_err(|e| io_err(&full, e))?
            .next()
            .is_none();
        if is_empty {
            fs::remove_dir(&full).map_err(|e| io_err(&full, e))?;
        } else {
            report.kept_dirs.push(full);
        }
    }
    Ok(())
}

fn undo_file(
    root: &Path,
    file: &FileRecord,
    backups_root: &Path,
    force: bool,
    report: &mut UninstallReport,
) -> Result<(), InstallError> {
    let target = root.join(&file.path);
    let current = match inspect(root, &file.path)? {
        None => None,
        Some(meta) if meta.is_dir() => {
            return Err(io_err(
                &target,
                io::Error::new(io::ErrorKind::InvalidData, "un dossier occupe la place"),
            ));
        }
        Some(_) => Some(hash_file(&target)?),
    };

    if let Some(backup) = &file.backup {
        let backup_file = backups_root.join(&backup.path);
        // Relance : l'original est déjà revenu, il ne reste qu'à nettoyer la sauvegarde.
        if current == Some(backup.original_sha256) {
            remove_if_exists(&backup_file)?;
            return Ok(());
        }
        // Fichier modifié depuis l'installation ? (`sha256` inconnu = installation
        // interrompue : on le considère comme le nôtre.)
        let modified = matches!((current, file.sha256), (Some(now), Some(wrote)) if now != wrote);
        if modified {
            report.conflicts.push(target.clone());
            if !force {
                return Ok(());
            }
        }
        restore(&backup_file, backup.original_sha256, &target)?;
        remove_if_exists(&backup_file)?;
        report.restored += 1;
        return Ok(());
    }

    // Pas de sauvegarde : le fichier a été créé par l'installation.
    let Some(now) = current else {
        return Ok(()); // déjà supprimé
    };
    if let Some(wrote) = file.sha256 {
        if now != wrote {
            report.conflicts.push(target.clone());
            if !force {
                return Ok(());
            }
            fs::remove_file(&target).map_err(|e| io_err(&target, e))?;
            report.removed += 1;
            return Ok(());
        }
    }
    fs::remove_file(&target).map_err(|e| io_err(&target, e))?;
    report.removed += 1;
    Ok(())
}

/// Remet la sauvegarde à sa place : copie vers un `.tmp` voisin, vérifie le hash,
/// puis renomme par-dessus la cible (un plantage ne laisse jamais un original à moitié copié).
fn restore(backup_file: &Path, expected: Sha256, target: &Path) -> Result<(), InstallError> {
    match fs::symlink_metadata(backup_file) {
        Ok(meta) if !is_link(&meta) && meta.is_file() => {}
        Ok(_) => return Err(InstallError::Link(backup_file.to_path_buf())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(InstallError::BackupMissing(backup_file.to_path_buf()));
        }
        Err(e) => return Err(io_err(backup_file, e)),
    }
    if hash_file(backup_file)? != expected {
        return Err(InstallError::BackupCorrupt(backup_file.to_path_buf()));
    }
    let mut tmp_name = target.as_os_str().to_os_string();
    tmp_name.push(".raftup-restore.tmp");
    let tmp = PathBuf::from(tmp_name);
    let result = fs::copy(backup_file, &tmp)
        .map_err(|e| io_err(&tmp, e))
        .and_then(|_| fs::rename(&tmp, target).map_err(|e| io_err(target, e)));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn remove_if_exists(path: &Path) -> Result<(), InstallError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err(path, e)),
    }
}

/// Supprime les dossiers vides sous `dir` (et `dir` lui-même s'il devient vide).
/// Au mieux : une erreur est ignorée, ce n'est que du rangement.
fn prune_empty_dirs(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if meta.is_dir() && !is_link(&meta) {
                prune_empty_dirs(&path);
            }
        }
    }
    let _ = fs::remove_dir(dir); // échoue si non vide : voulu
}
