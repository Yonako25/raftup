//! Installer et mettre à jour les mods d'un jeu, en écrivant le reçu avant de toucher au dossier du jeu.
//!
//! Ordre (ADR 0015, « Réversibilité ») :
//! 1. valider le profil et les arguments ;
//! 2. télécharger toutes les archives (cache par sha256, hash vérifié) : rien n'est encore
//!    écrit dans le dossier du jeu ;
//! 3. calculer le plan (archive lue, pas extraite) et sauvegarder chaque fichier existant
//!    qui va être remplacé dans `backups_dir(game_id)` ;
//! 4. écrire le reçu (état `InProgress`, atomique) ;
//! 5. extraire ou copier, puis noter le hash de chaque fichier écrit ;
//! 6. remettre en place les fichiers `[preserve]` déjà présents ;
//! 7. passer le reçu à `Complete`.
//!
//! Si l'étape 5 échoue, on défait ce qui a été fait (`uninstall` avec `force`) et on
//! renvoie l'erreur d'origine.
//!
//! Le code est **synchrone** : l'interface l'appelle depuis un thread à part.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::archive::{extract_entry, extract_zip, plan_zip, safe_relative_path, ArchiveLimits};
use crate::download::{DownloadError, Downloader};
use crate::hash::Sha256;
use crate::paths::Paths;
use crate::profile::{GameStatus, InstallAction, Profile, Step};
use crate::record::{Backup, FileRecord, InstallRecord, RecordState, StepRecord};
use crate::template::{self, TemplateError, Vars};
use crate::uninstall::{hash_file, inspect, is_link, uninstall};

/// Ce qui fait échouer une installation ou une désinstallation.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    /// Lecture ou écriture impossible sur ce chemin.
    #[error("accès à {path} : {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Aucun reçu : le jeu n'a rien d'installé par Raftup.
    #[error("rien n'est installé pour le jeu {game_id}")]
    NotInstalled { game_id: String },
    /// Le reçu est illisible ou ne s'écrit pas.
    #[error(transparent)]
    Record(#[from] crate::record::RecordError),
    /// Un chemin du reçu est dangereux (`..`, chemin absolu…) : le reçu est refusé.
    #[error(transparent)]
    UnsafePath(#[from] crate::archive::ArchiveError),
    /// La racine d'une étape dans le reçu n'est pas un chemin absolu sûr.
    #[error("racine d'étape invalide dans le reçu : {0}")]
    UnsafeRoot(PathBuf),
    /// Un lien ou une jonction se trouve sur le chemin : on ne le suit jamais.
    #[error("{0} est un lien ou une jonction : refusé")]
    Link(PathBuf),
    /// La sauvegarde d'un original a disparu : impossible de le restaurer.
    #[error("sauvegarde introuvable : {0}")]
    BackupMissing(PathBuf),
    /// La sauvegarde ne correspond plus au hash noté dans le reçu.
    #[error("sauvegarde corrompue : {0}")]
    BackupCorrupt(PathBuf),
    /// Le téléchargement d'une archive a échoué.
    #[error(transparent)]
    Download(#[from] DownloadError),
    /// Un gabarit du profil n'a pas pu être rempli.
    #[error(transparent)]
    Template(#[from] TemplateError),
    /// Le profil est marqué cassé : on n'installe pas.
    #[error("le jeu {game_id} est marqué comme cassé dans le catalogue")]
    GameBroken { game_id: String },
    /// Argument inutilisable (dossier du jeu relatif ou absent, version d'Archipelago bizarre…).
    #[error("argument invalide : {0}")]
    InvalidArgument(String),
    /// Impossible d'écrire dans le dossier du jeu (droits) : Raftup ne demande jamais l'élévation.
    #[error("impossible d'écrire dans {0} : lancez le jeu et Raftup avec les mêmes droits, ou changez les droits du dossier")]
    NotWritable(PathBuf),
    /// Une installation précédente a été interrompue : il faut la défaire (ou `force`).
    #[error("une installation précédente du jeu {game_id} a été interrompue : désinstallez d'abord (ou forcez)")]
    Interrupted { game_id: String },
    /// Une installation est déjà en cours pour ce jeu (un seul à la fois par `game_id`).
    #[error("une installation du jeu {game_id} est déjà en cours")]
    Busy { game_id: String },
    /// Des fichiers modifiés par le joueur bloquent la mise à jour (relancer avec `force`).
    #[error("{} fichier(s) modifié(s) depuis l'installation bloquent la mise à jour", .0.len())]
    Conflicts(Vec<PathBuf>),
}

/// Réglages d'une installation.
#[derive(Debug, Clone)]
pub struct InstallOptions {
    /// Écraser les fichiers modifiés par le joueur lors d'une mise à jour, et défaire
    /// une installation interrompue.
    pub force: bool,
    /// Version d'Archipelago visée : elle donne `{worlds_dir}`.
    pub ap_version: String,
}

/// Ce qu'une installation a fait.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallReport {
    /// Tout était déjà à jour : rien n'a été touché.
    pub already_up_to_date: bool,
    pub steps_installed: usize,
    pub files_written: usize,
    /// Fichiers existants remplacés, dont l'original est sauvegardé.
    pub backed_up: Vec<PathBuf>,
    /// Fichiers `[preserve]` déjà présents, laissés tels quels.
    pub kept_preserved: Vec<PathBuf>,
    /// Images du créateur rangées (section `[art]`, ADR 0051).
    pub art_installed: usize,
}

/// L'état d'un jeu vu par l'interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallStatus {
    NotInstalled,
    /// Installé, et les versions du reçu sont celles du profil.
    Installed,
    /// Reçu `InProgress` : une installation a été interrompue.
    InProgress,
    /// Installé, mais le profil demande d'autres versions.
    UpdateAvailable,
}

/// État d'installation d'un jeu, d'après son reçu et son profil.
pub fn status(paths: &Paths, profile: &Profile) -> Result<InstallStatus, InstallError> {
    let Some(record) = InstallRecord::load(paths, &profile.game.id)? else {
        return Ok(InstallStatus::NotInstalled);
    };
    if record.state == RecordState::InProgress {
        return Ok(InstallStatus::InProgress);
    }
    if same_steps(&record, profile) {
        Ok(InstallStatus::Installed)
    } else {
        Ok(InstallStatus::UpdateAvailable)
    }
}

/// Le reçu contient-il exactement les étapes du profil (id, version, hash) ?
fn same_steps(record: &InstallRecord, profile: &Profile) -> bool {
    record.steps.len() == profile.steps.len()
        && record
            .steps
            .iter()
            .zip(&profile.steps)
            .all(|(r, s)| r.id == s.id && r.version == s.version && r.sha256 == s.sha256)
}

// Un seul `install` à la fois par jeu, dans ce processus. `static` = une seule valeur
// pour tout le programme ; le `Mutex` la protège quand plusieurs threads l'utilisent.
static ACTIVE: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Réservation d'un `game_id`, libérée automatiquement par `Drop` (même en cas d'erreur).
struct LockGuard {
    game_id: String,
}

impl LockGuard {
    fn acquire(game_id: &str) -> Result<LockGuard, InstallError> {
        // Un mutex « empoisonné » (un thread a paniqué) reste utilisable : `into_inner`.
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if !active.insert(game_id.to_string()) {
            return Err(InstallError::Busy {
                game_id: game_id.to_string(),
            });
        }
        Ok(LockGuard {
            game_id: game_id.to_string(),
        })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        active.remove(&self.game_id);
    }
}

fn io_err(path: &Path, source: io::Error) -> InstallError {
    InstallError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Une étape prête à être appliquée.
struct PlannedStep<'a> {
    step: &'a Step,
    /// Archive téléchargée (dans le cache).
    archive: PathBuf,
    /// Dossier de base (`game_dir` ou `worlds_dir`) : racine du reçu pour cette étape.
    base: PathBuf,
    /// Dossier de destination, relatif à `base` (vide = la base elle-même).
    into_rel: PathBuf,
    /// Fichiers qui seront écrits, relatifs à `base`.
    files: Vec<PathBuf>,
}

/// Sépare `{game_dir}/Mods` en (dossier de base, chemin relatif `Mods`).
fn resolve_target(
    template: &str,
    game_dir: &Path,
    worlds_dir: &Path,
) -> Result<(PathBuf, PathBuf), InstallError> {
    let (base, rest) = if let Some(rest) = template.strip_prefix("{game_dir}") {
        (game_dir, rest)
    } else if let Some(rest) = template.strip_prefix("{worlds_dir}") {
        (worlds_dir, rest)
    } else {
        return Err(InstallError::InvalidArgument(format!(
            "cible « {template} » hors de {{game_dir}} et {{worlds_dir}}"
        )));
    };
    let rest = rest.trim_matches(['/', '\\']);
    let rel = if rest.is_empty() {
        PathBuf::new()
    } else {
        // Même contrôle que pour les entrées d'archive : jamais de `..`, de lecteur…
        safe_relative_path(rest)?
    };
    Ok((base.to_path_buf(), rel))
}

/// Forme comparable d'un chemin : séparateurs unifiés, minuscules (NTFS ignore la casse).
fn normalize(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

/// Crée (si besoin) chaque dossier de `relative` sous `base`, en refusant tout lien.
fn ensure_dirs(base: &Path, relative: &Path) -> Result<(), InstallError> {
    let mut current = base.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                if is_link(&meta) {
                    return Err(InstallError::Link(current));
                }
                if !meta.is_dir() {
                    return Err(io_err(
                        &current,
                        io::Error::new(io::ErrorKind::AlreadyExists, "un fichier occupe la place"),
                    ));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|e| io_err(&current, e))?;
            }
            Err(e) => return Err(io_err(&current, e)),
        }
    }
    Ok(())
}

/// Vérifie qu'on peut écrire dans le dossier (sans jamais demander d'élévation).
fn check_writable(dir: &Path) -> Result<(), InstallError> {
    let probe = dir.join(".raftup-write-test");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            Err(InstallError::NotWritable(dir.to_path_buf()))
        }
        Err(e) => Err(io_err(dir, e)),
    }
}

/// Installe (ou met à jour) les mods d'un jeu. Voir l'en-tête du module pour l'ordre.
///
/// `progress` reçoit (id de l'étape, octets téléchargés).
pub fn install(
    paths: &Paths,
    profile: &Profile,
    game_dir: &Path,
    downloader: &Downloader,
    options: &InstallOptions,
    progress: &mut dyn FnMut(&str, u64),
) -> Result<InstallReport, InstallError> {
    let game_id = profile.game.id.as_str();
    if profile.game.status == GameStatus::Broken {
        return Err(InstallError::GameBroken {
            game_id: game_id.to_string(),
        });
    }
    if !game_dir.is_absolute() || !game_dir.is_dir() {
        return Err(InstallError::InvalidArgument(format!(
            "dossier du jeu : {} (un dossier existant, en chemin absolu, est attendu)",
            game_dir.display()
        )));
    }
    let ap_ok = !options.ap_version.is_empty()
        && options
            .ap_version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if !ap_ok {
        return Err(InstallError::InvalidArgument(format!(
            "version d'Archipelago : {}",
            options.ap_version
        )));
    }

    // Réservé jusqu'à la fin de la fonction : `_guard` est libéré par `Drop`.
    let _guard = LockGuard::acquire(game_id)?;
    check_writable(game_dir)?;

    let worlds_dir = paths.worlds_dir(&options.ap_version);
    let vars = Vars::new()
        .with("game_dir", &game_dir.to_string_lossy())
        .with("worlds_dir", &worlds_dir.to_string_lossy());

    // Fichiers à ne jamais écraser (F07), en chemins absolus normalisés.
    let mut preserved: BTreeSet<String> = BTreeSet::new();
    for pattern in &profile.preserve.paths {
        preserved.insert(normalize(Path::new(&template::expand(pattern, &vars)?)));
    }

    // --- Reçu existant : à jour, interrompu, ou à mettre à jour ---
    match InstallRecord::load(paths, game_id)? {
        Some(record) if record.state == RecordState::InProgress => {
            if !options.force {
                return Err(InstallError::Interrupted {
                    game_id: game_id.to_string(),
                });
            }
            uninstall(paths, game_id, true)?;
        }
        Some(mut record) => {
            if same_steps(&record, profile) {
                // Les images peuvent manquer (profil complété après l'installation).
                return Ok(InstallReport {
                    already_up_to_date: true,
                    art_installed: art_best_effort(paths, profile, downloader),
                    ..InstallReport::default()
                });
            }
            // Mise à jour = on défait l'ancienne installation puis on installe la nouvelle.
            // Les fichiers `[preserve]` sortent du reçu : la désinstallation n'y touche pas.
            for step in &mut record.steps {
                let root = step.root.clone();
                step.files
                    .retain(|f| !preserved.contains(&normalize(&root.join(&f.path))));
            }
            record.save(paths)?;
            let report = uninstall(paths, game_id, options.force)?;
            if !report.conflicts.is_empty() && !options.force {
                return Err(InstallError::Conflicts(report.conflicts));
            }
        }
        None => {
            // Pas de reçu : d'éventuelles sauvegardes sont des restes sans valeur.
            remove_dir_all_if_exists(&paths.backups_dir(game_id))?;
        }
    }

    // --- 2. Téléchargements (rien n'est encore écrit dans le dossier du jeu) ---
    let mut archives = Vec::new();
    for step in &profile.steps {
        let url = step.source.resolve(&step.version);
        let id = step.id.clone();
        let file = downloader.get(
            &url,
            &step.sha256,
            step.max_bytes_or_default(),
            &mut |bytes| progress(&id, bytes),
        )?;
        archives.push(file);
    }

    // --- 3. Plan de chaque étape ---
    let limits = ArchiveLimits::default();
    let mut planned = Vec::new();
    for (step, archive) in profile.steps.iter().zip(archives) {
        let (base, into_rel, files, archive) = match &step.install {
            InstallAction::Extract {
                into,
                strip_prefix,
                inner,
            } => {
                let (base, into_rel) = resolve_target(into, game_dir, &worlds_dir)?;
                // Archive dans l'archive (ADR 0050) : on sort l'archive intérieure à côté
                // du fichier téléchargé, puis on la traite comme l'archive de l'étape.
                let archive = match inner {
                    Some(name) => {
                        let mut inner_name = archive.as_os_str().to_os_string();
                        inner_name.push(".inner.zip");
                        let inner_path = PathBuf::from(inner_name);
                        extract_entry(&archive, name, &inner_path, &limits)?;
                        inner_path
                    }
                    None => archive,
                };
                let files = plan_zip(&archive, strip_prefix, &limits)?
                    .into_iter()
                    .map(|f| into_rel.join(f))
                    .collect();
                (base, into_rel, files, archive)
            }
            InstallAction::CopyTo(target) => {
                let (base, rel) = resolve_target(target, game_dir, &worlds_dir)?;
                let parent = rel.parent().map(Path::to_path_buf).unwrap_or_default();
                (base, parent, vec![rel], archive)
            }
        };
        planned.push(PlannedStep {
            step,
            archive,
            base,
            into_rel,
            files,
        });
    }

    // --- 3 bis. Sauvegardes et reçu initial ---
    let mut report = InstallReport::default();
    let mut record = InstallRecord::new(game_id, game_dir.to_path_buf());
    let prepared = prepare_records(paths, game_id, &planned, &mut record, &mut report);
    if let Err(e) = prepared {
        // Rien n'a encore touché le jeu : on enlève seulement les sauvegardes.
        let _ = remove_dir_all_if_exists(&paths.backups_dir(game_id));
        return Err(e);
    }

    // --- 4. Le reçu AVANT de toucher au dossier du jeu ---
    if let Err(e) = record.save(paths) {
        let _ = remove_dir_all_if_exists(&paths.backups_dir(game_id));
        return Err(e.into());
    }

    // --- 5 et 6. Écriture ; en cas d'échec, on défait tout ---
    let applied = apply(
        paths,
        &planned,
        &mut record,
        &preserved,
        &limits,
        &mut report,
    );
    if let Err(e) = applied {
        // Meilleur effort : l'erreur d'origine est celle qu'on rapporte.
        let _ = uninstall(paths, game_id, true);
        return Err(e);
    }

    // --- 7. Reçu final ---
    record.state = RecordState::Complete;
    record.save(paths)?;
    report.steps_installed = planned.len();
    report.art_installed = art_best_effort(paths, profile, downloader);
    Ok(report)
}

/// Range les images du créateur. Une image qui ne vient pas ne fait jamais échouer
/// l'installation : le jeu marche sans, et l'interface garde sa jaquette dessinée.
fn art_best_effort(paths: &Paths, profile: &Profile, downloader: &Downloader) -> usize {
    match crate::art::install_art(paths, profile, downloader) {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!(%error, game = %profile.game.id, "images du jeu non installées");
            0
        }
    }
}

/// Sauvegarde les fichiers qui vont être remplacés et remplit le reçu (hash encore inconnus).
fn prepare_records(
    paths: &Paths,
    game_id: &str,
    planned: &[PlannedStep<'_>],
    record: &mut InstallRecord,
    report: &mut InstallReport,
) -> Result<(), InstallError> {
    let backups_dir = paths.backups_dir(game_id);
    for (index, item) in planned.iter().enumerate() {
        let mut files = Vec::new();
        let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
        for rel in &item.files {
            // Dossiers parents qui n'existent pas encore : c'est nous qui les créons.
            for ancestor in rel.ancestors().skip(1) {
                if ancestor.as_os_str().is_empty() {
                    break;
                }
                if inspect(&item.base, ancestor)?.is_none() {
                    dirs.insert(ancestor.to_path_buf());
                }
            }

            let mut backup = None;
            if let Some(meta) = inspect(&item.base, rel)? {
                if meta.is_dir() {
                    return Err(io_err(
                        &item.base.join(rel),
                        io::Error::new(io::ErrorKind::InvalidData, "un dossier occupe la place"),
                    ));
                }
                let source = item.base.join(rel);
                let saved_rel = PathBuf::from(index.to_string()).join(rel);
                let saved = backups_dir.join(&saved_rel);
                if let Some(parent) = saved.parent() {
                    fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
                }
                fs::copy(&source, &saved).map_err(|e| io_err(&saved, e))?;
                backup = Some(Backup {
                    path: saved_rel,
                    original_sha256: hash_file(&saved)?,
                });
                report.backed_up.push(source);
            }
            files.push(FileRecord {
                path: rel.clone(),
                sha256: None,
                backup,
            });
        }
        record.steps.push(StepRecord {
            id: item.step.id.clone(),
            version: item.step.version.clone(),
            root: item.base.clone(),
            sha256: item.step.sha256,
            files,
            dirs: dirs.into_iter().collect(),
        });
    }
    Ok(())
}

/// Écrit les fichiers de chaque étape, note leurs hash, remet les fichiers préservés.
fn apply(
    paths: &Paths,
    planned: &[PlannedStep<'_>],
    record: &mut InstallRecord,
    preserved: &BTreeSet<String>,
    limits: &ArchiveLimits,
    report: &mut InstallReport,
) -> Result<(), InstallError> {
    let backups_dir = paths.backups_dir(&record.game_id);
    for (index, item) in planned.iter().enumerate() {
        // `worlds_dir` n'existe pas forcément encore (le dossier du jeu, lui, existe).
        fs::create_dir_all(&item.base).map_err(|e| io_err(&item.base, e))?;
        ensure_dirs(&item.base, &item.into_rel)?;
        // Hash des fichiers écrits, par chemin relatif à `base`.
        let mut written: BTreeMap<PathBuf, Sha256> = BTreeMap::new();

        match &item.step.install {
            InstallAction::Extract { strip_prefix, .. } => {
                let root = item.base.join(&item.into_rel);
                // `&mut |rel, hash| …` : une fermeture qui emprunte `written` en écriture.
                extract_zip(
                    &item.archive,
                    strip_prefix,
                    &root,
                    limits,
                    &mut |rel, hash| {
                        written.insert(item.into_rel.join(rel), hash);
                    },
                )?;
            }
            InstallAction::CopyTo(_) => {
                for rel in &item.files {
                    let target = item.base.join(rel);
                    let mut tmp_name = target.as_os_str().to_os_string();
                    tmp_name.push(".raftup.tmp");
                    let tmp = PathBuf::from(tmp_name);
                    // Copie vers un fichier voisin puis renommage : jamais de fichier à moitié écrit.
                    let result = fs::copy(&item.archive, &tmp)
                        .map_err(|e| io_err(&tmp, e))
                        .and_then(|_| fs::rename(&tmp, &target).map_err(|e| io_err(&target, e)));
                    if let Err(e) = result {
                        let _ = fs::remove_file(&tmp);
                        return Err(e);
                    }
                    written.insert(rel.clone(), hash_file(&target)?);
                }
            }
        }

        // Hash dans le reçu (en mémoire ; sauvegardé avec l'état final).
        let step_record = &mut record.steps[index];
        for file in &mut step_record.files {
            file.sha256 = written.get(&file.path).copied();
        }
        report.files_written += written.len();

        // Fichiers `[preserve]` qui existaient déjà : on remet l'original du joueur,
        // et on les retire du reçu (ils ne sont pas à nous).
        let root = step_record.root.clone();
        let mut kept = Vec::new();
        step_record.files.retain(|file| {
            let absolute = root.join(&file.path);
            let keep_user_file = file.backup.is_some() && preserved.contains(&normalize(&absolute));
            if keep_user_file {
                kept.push((absolute, file.backup.clone()));
            }
            !keep_user_file
        });
        for (absolute, backup) in kept {
            if let Some(backup) = backup {
                let saved = backups_dir.join(&backup.path);
                fs::copy(&saved, &absolute).map_err(|e| io_err(&absolute, e))?;
                let _ = fs::remove_file(&saved);
            }
            report.kept_preserved.push(absolute);
        }
    }
    Ok(())
}

fn remove_dir_all_if_exists(dir: &Path) -> Result<(), InstallError> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err(dir, e)),
    }
}
