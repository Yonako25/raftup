//! Extraction zip sécurisée : zip-slip, bombe zip, un seul point d'extraction.
//!
//! C'est le point de sécurité du launcher : un mod téléchargé est un fichier venu
//! d'Internet, donc on ne fait confiance à rien dans l'archive.
//!
//! - chaque chemin est contrôlé ([`safe_relative_path`]) : composants normaux seulement ;
//! - les liens symboliques sont refusés ;
//! - le nombre d'entrées et les octets **réellement écrits** sont bornés (la taille
//!   annoncée par l'archive peut mentir) ;
//! - un lien ou une jonction déjà présent dans le dossier de destination n'est jamais suivi.
//!
//! Aucune autre fonction du launcher ne doit extraire un zip.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::hash::Sha256;

/// Ce qui fait échouer l'extraction d'une archive.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    /// Le chemin d'une entrée sort du dossier de destination ou est interdit sous Windows.
    #[error("chemin dangereux dans l'archive : {0}")]
    UnsafePath(String),

    /// Lien symbolique dans l'archive, ou lien/jonction déjà présent sur le disque.
    #[error("lien refusé : {0}")]
    Link(String),

    /// Plus d'entrées que la limite autorisée.
    #[error("l'archive contient trop d'entrées")]
    TooManyEntries,

    /// Plus d'octets que la limite autorisée (annoncés ou réellement décompressés).
    #[error("l'archive est trop volumineuse une fois décompressée")]
    TooLarge,

    /// Archive illisible ou corrompue.
    #[error("archive zip invalide : {0}")]
    Zip(String),

    /// Archive .tar.gz illisible, tronquée ou corrompue.
    #[error("archive tar.gz invalide : {0}")]
    Tar(String),

    /// Fichier spécial (périphérique, FIFO, fichier creux…) : jamais extrait.
    #[error("fichier spécial refusé : {0}")]
    SpecialFile(String),

    /// Lecture ou écriture impossible sur ce chemin.
    #[error("accès à {path} : {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Bornes contre les bombes zip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveLimits {
    /// Nombre maximal d'entrées (fichiers et dossiers).
    pub max_entries: usize,
    /// Nombre maximal d'octets décompressés, tous fichiers confondus.
    pub max_total_bytes: u64,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        ArchiveLimits {
            max_entries: 20_000,
            max_total_bytes: 2 * 1024 * 1024 * 1024, // 2 Gio
        }
    }
}

/// Noms que Windows réserve (périphériques), quelle que soit l'extension : `CON.txt` aussi.
const RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Contrôle le nom d'une entrée et le transforme en chemin relatif sûr.
///
/// Accepte `/` et `\` comme séparateurs. Refuse : `..`, chemins absolus, lettres de
/// lecteur et `:` (donc les flux NTFS `fichier:flux`), chemins UNC, octets nuls et
/// caractères de contrôle, caractères interdits sous Windows, noms réservés (CON, NUL,
/// COM1…), composants finissant par un point ou une espace. Un `/` final (entrée de
/// dossier) et les composants `.` sont ignorés.
pub fn safe_relative_path(entry: &str) -> Result<PathBuf, ArchiveError> {
    let unsafe_path = || ArchiveError::UnsafePath(entry.to_string());

    if entry.is_empty() || entry.starts_with('/') || entry.starts_with('\\') {
        // Couvre aussi les chemins UNC (`\\serveur\partage`) et `//serveur`.
        return Err(unsafe_path());
    }

    let mut result = PathBuf::new();
    // `split` renvoie un morceau vide après un séparateur final : on le tolère une fois.
    let parts: Vec<&str> = entry.split(['/', '\\']).collect();
    let last = parts.len() - 1;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            if index == last {
                continue; // entrée de dossier « a/b/ »
            }
            return Err(unsafe_path()); // « a//b »
        }
        if *part == "." {
            continue;
        }
        if *part == ".." || !is_valid_component(part) {
            return Err(unsafe_path());
        }
        result.push(part);
    }

    if result.as_os_str().is_empty() {
        return Err(unsafe_path());
    }
    // Dernière barrière : le résultat ne doit contenir que des composants normaux.
    if result
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(unsafe_path());
    }
    Ok(result)
}

/// Un composant de chemin est-il acceptable sous Windows comme sous Linux ?
fn is_valid_component(part: &str) -> bool {
    if part.chars().any(|c| {
        c.is_control() || matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*' | '/' | '\\')
    }) {
        return false;
    }
    // Windows supprime silencieusement les points et espaces finaux : « CON. » serait « CON ».
    if part.ends_with('.') || part.ends_with(' ') {
        return false;
    }
    // Le nom réservé compte avant le premier point : « nul.txt » est réservé.
    let stem = part.split('.').next().unwrap_or(part).trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    !RESERVED_NAMES.contains(&upper.as_str())
}

/// Une entrée retenue après contrôle.
struct Planned {
    /// Position dans l'archive.
    index: usize,
    /// Chemin relatif sûr, préfixe retiré.
    path: PathBuf,
    is_dir: bool,
}

/// Mode Unix d'un lien symbolique (`S_IFLNK`) dans les attributs de l'entrée.
const UNIX_TYPE_MASK: u32 = 0o170000;
const UNIX_SYMLINK: u32 = 0o120000;

/// Préfixe à retirer, déjà contrôlé comme un chemin (vide = rien à retirer).
fn parse_prefix(strip_prefix: &str) -> Result<Vec<std::ffi::OsString>, ArchiveError> {
    if strip_prefix.trim_matches(['/', '\\']).is_empty() {
        return Ok(Vec::new());
    }
    Ok(safe_relative_path(strip_prefix)?
        .components()
        .map(|c| c.as_os_str().to_os_string())
        .collect())
}

/// Ouvre l'archive et contrôle toutes les entrées sans rien écrire.
fn read_plan(
    file: &Path,
    strip_prefix: &str,
    limits: &ArchiveLimits,
) -> Result<(zip::ZipArchive<File>, Vec<Planned>), ArchiveError> {
    let handle = File::open(file).map_err(|source| ArchiveError::Io {
        path: file.to_path_buf(),
        source,
    })?;
    let mut archive = zip::ZipArchive::new(handle).map_err(|e| ArchiveError::Zip(e.to_string()))?;

    // On refuse avant de parcourir quoi que ce soit.
    if archive.len() > limits.max_entries {
        return Err(ArchiveError::TooManyEntries);
    }

    let prefix = parse_prefix(strip_prefix)?;

    let mut planned = Vec::new();
    let mut announced_total: u64 = 0;
    // Détecte les doublons (insensible à la casse, comme NTFS).
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| ArchiveError::Zip(e.to_string()))?;
        let name = entry
            .name()
            .map_err(|e| ArchiveError::Zip(e.to_string()))?
            .into_owned();

        // On contrôle d'abord le nom complet, même s'il sera écarté par le préfixe.
        let full = safe_relative_path(&name)?;

        let is_symlink = entry.is_symlink()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & UNIX_TYPE_MASK == UNIX_SYMLINK);
        if is_symlink {
            return Err(ArchiveError::Link(name));
        }

        // Retire le préfixe ; une entrée hors du préfixe est ignorée.
        let comps: Vec<std::ffi::OsString> = full
            .components()
            .map(|c| c.as_os_str().to_os_string())
            .collect();
        if comps.len() <= prefix.len() || comps[..prefix.len()] != prefix[..] {
            continue;
        }
        let relative: PathBuf = comps[prefix.len()..].iter().collect();

        let key = relative.to_string_lossy().to_lowercase();
        if !seen.insert(key) {
            return Err(ArchiveError::UnsafePath(format!("{name} (doublon)")));
        }

        let is_dir = entry.is_dir();
        if !is_dir {
            announced_total = announced_total.saturating_add(entry.size());
            if announced_total > limits.max_total_bytes {
                return Err(ArchiveError::TooLarge);
            }
        }
        planned.push(Planned {
            index,
            path: relative,
            is_dir,
        });
    }
    Ok((archive, planned))
}

/// Liste les fichiers qui seraient écrits (chemins relatifs, préfixe retiré), sans rien extraire.
///
/// Fait les mêmes contrôles que [`extract_zip`], y compris sur la taille annoncée.
pub fn plan_zip(
    file: &Path,
    strip_prefix: &str,
    limits: &ArchiveLimits,
) -> Result<Vec<PathBuf>, ArchiveError> {
    let (_, planned) = read_plan(file, strip_prefix, limits)?;
    Ok(planned
        .into_iter()
        .filter(|p| !p.is_dir)
        .map(|p| p.path)
        .collect())
}

/// Extrait `file` dans `root`.
///
/// `on_file` est appelé après l'écriture de chaque fichier, avec son chemin **relatif à
/// `root`** et le hash de son contenu réellement écrit. En cas d'erreur, les fichiers déjà
/// écrits restent sur le disque : l'appelant (l'installation) les connaît grâce à `on_file`
/// et peut les défaire.
pub fn extract_zip(
    file: &Path,
    strip_prefix: &str,
    root: &Path,
    limits: &ArchiveLimits,
    on_file: &mut dyn FnMut(&Path, Sha256),
) -> Result<(), ArchiveError> {
    // Tous les contrôles d'abord : rien n'est écrit si une seule entrée est mauvaise.
    let (mut archive, planned) = read_plan(file, strip_prefix, limits)?;

    fs::create_dir_all(root).map_err(|source| io_error(root, source))?;

    let mut total_written: u64 = 0;
    for item in planned {
        let target = root.join(&item.path);

        if item.is_dir {
            ensure_dir_chain(root, &item.path)?;
            continue;
        }

        // Les dossiers parents : chacun est vérifié, aucun lien n'est suivi.
        if let Some(parent) = item.path.parent() {
            ensure_dir_chain(root, parent)?;
        }
        refuse_link_or_dir(&target)?;

        let mut entry = archive
            .by_index(item.index)
            .map_err(|e| ArchiveError::Zip(e.to_string()))?;
        let out = File::create(&target).map_err(|source| io_error(&target, source))?;

        // `Tee` copie ce que `of_reader` lit vers le fichier : une seule lecture sert à
        // écrire, à compter et à calculer le hash.
        let mut tee = Tee {
            input: &mut entry,
            output: BufWriter::new(out),
            written: 0,
            budget: limits.max_total_bytes.saturating_sub(total_written),
            failure: None,
        };
        let hash = match Sha256::of_reader(&mut tee) {
            Ok(hash) => hash,
            Err(error) => {
                return Err(match tee.failure.take() {
                    Some(Failure::TooLarge) => ArchiveError::TooLarge,
                    Some(Failure::Write(source)) => io_error(&target, source),
                    None => ArchiveError::Zip(error.to_string()),
                });
            }
        };
        tee.output
            .flush()
            .map_err(|source| io_error(&target, source))?;
        total_written += tee.written;

        on_file(&item.path, hash);
    }
    Ok(())
}

/// Contrôle les entrées d'un tar une par une (même règles que le zip).
///
/// Rust : un tar se lit en flux, sans index. On le parcourt donc deux fois avec deux
/// `TarPlanner` neufs : la première passe ne fait que contrôler, la seconde écrit.
struct TarPlanner {
    prefix: Vec<std::ffi::OsString>,
    max_entries: usize,
    max_total_bytes: u64,
    count: usize,
    announced_total: u64,
    seen: BTreeSet<String>,
}

impl TarPlanner {
    fn new(strip_prefix: &str, limits: &ArchiveLimits) -> Result<Self, ArchiveError> {
        Ok(TarPlanner {
            prefix: parse_prefix(strip_prefix)?,
            max_entries: limits.max_entries,
            max_total_bytes: limits.max_total_bytes,
            count: 0,
            announced_total: 0,
            seen: BTreeSet::new(),
        })
    }

    /// `Ok(None)` : entrée à ignorer. `Ok(Some((chemin, est_dossier)))` : à extraire.
    fn check<R: Read>(
        &mut self,
        entry: &tar::Entry<'_, R>,
    ) -> Result<Option<(PathBuf, bool)>, ArchiveError> {
        use tar::EntryType;

        let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let kind = entry.header().entry_type();

        // En-tête global PAX : métadonnées, pas un fichier.
        if kind == EntryType::XGlobalHeader {
            return Ok(None);
        }

        self.count += 1;
        if self.count > self.max_entries {
            return Err(ArchiveError::TooManyEntries);
        }

        // Liens symboliques et physiques : jamais suivis, jamais créés.
        if kind == EntryType::Symlink || kind == EntryType::Link {
            return Err(ArchiveError::Link(name));
        }
        let is_dir = kind == EntryType::Directory;
        if !(is_dir || kind == EntryType::Regular || kind == EntryType::Continuous) {
            // Périphériques, FIFO, fichiers creux GNU, types inconnus.
            return Err(ArchiveError::SpecialFile(name));
        }

        // Le dossier racine « ./ » des tar créés par `tar -C dir .` : rien à faire.
        if name == "." || name == "./" {
            return Ok(None);
        }
        // Un nom non UTF-8 deviendrait des « � » : on le refuse plutôt que de le deviner.
        if std::str::from_utf8(&entry.path_bytes()).is_err() {
            return Err(ArchiveError::UnsafePath(name));
        }
        let full = safe_relative_path(&name)?;

        let comps: Vec<std::ffi::OsString> = full
            .components()
            .map(|c| c.as_os_str().to_os_string())
            .collect();
        let prefix = &self.prefix;
        if comps.len() <= prefix.len() || comps[..prefix.len()] != prefix[..] {
            return Ok(None);
        }
        let relative: PathBuf = comps[prefix.len()..].iter().collect();

        if !self.seen.insert(relative.to_string_lossy().to_lowercase()) {
            return Err(ArchiveError::UnsafePath(format!("{name} (doublon)")));
        }
        if !is_dir {
            self.announced_total = self.announced_total.saturating_add(entry.size());
            if self.announced_total > self.max_total_bytes {
                return Err(ArchiveError::TooLarge);
            }
        }
        Ok(Some((relative, is_dir)))
    }
}

/// Ouvre un `.tar.gz` en lecture.
fn open_tar_gz(
    file: &Path,
) -> Result<tar::Archive<flate2::read::GzDecoder<BufReader<File>>>, ArchiveError> {
    let handle = File::open(file).map_err(|source| io_error(file, source))?;
    Ok(tar::Archive::new(flate2::read::GzDecoder::new(
        BufReader::new(handle),
    )))
}

/// Extrait un `.tar.gz` dans `root`, avec les mêmes garanties que [`extract_zip`].
///
/// Refuse en plus les liens physiques et les fichiers spéciaux. Une première passe
/// contrôle tout (rien n'est écrit si une entrée est mauvaise), la seconde écrit en
/// comptant les octets réellement décompressés. Sous Unix, le bit exécutable est conservé.
pub fn extract_tar_gz(
    file: &Path,
    strip_prefix: &str,
    root: &Path,
    limits: &ArchiveLimits,
    on_file: &mut dyn FnMut(&Path, Sha256),
) -> Result<(), ArchiveError> {
    let tar_error = |e: io::Error| ArchiveError::Tar(e.to_string());

    // Passe 1 : contrôle seulement.
    let mut planner = TarPlanner::new(strip_prefix, limits)?;
    let mut archive = open_tar_gz(file)?;
    for entry in archive.entries().map_err(tar_error)? {
        let entry = entry.map_err(tar_error)?;
        planner.check(&entry)?;
    }

    fs::create_dir_all(root).map_err(|source| io_error(root, source))?;

    // Passe 2 : écriture. On ne se fie pas à la passe 1 : tout est recontrôlé.
    let mut planner = TarPlanner::new(strip_prefix, limits)?;
    let mut archive = open_tar_gz(file)?;
    let mut total_written: u64 = 0;
    for entry in archive.entries().map_err(tar_error)? {
        let mut entry = entry.map_err(tar_error)?;
        let Some((relative, is_dir)) = planner.check(&entry)? else {
            continue;
        };
        let target = root.join(&relative);

        if is_dir {
            ensure_dir_chain(root, &relative)?;
            continue;
        }
        if let Some(parent) = relative.parent() {
            ensure_dir_chain(root, parent)?;
        }
        refuse_link_or_dir(&target)?;

        let mode = entry.header().mode().unwrap_or(0o644);
        let out = File::create(&target).map_err(|source| io_error(&target, source))?;
        let mut tee = Tee {
            input: &mut entry,
            output: BufWriter::new(out),
            written: 0,
            budget: limits.max_total_bytes.saturating_sub(total_written),
            failure: None,
        };
        let hash = match Sha256::of_reader(&mut tee) {
            Ok(hash) => hash,
            Err(error) => {
                return Err(match tee.failure.take() {
                    Some(Failure::TooLarge) => ArchiveError::TooLarge,
                    Some(Failure::Write(source)) => io_error(&target, source),
                    None => ArchiveError::Tar(error.to_string()),
                });
            }
        };
        tee.output
            .flush()
            .map_err(|source| io_error(&target, source))?;
        total_written += tee.written;
        drop(tee);

        // Les exécutables (python, bin/…) doivent le rester ; rien d'autre n'est copié du mode.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let wanted = if mode & 0o111 != 0 { 0o755 } else { 0o644 };
            fs::set_permissions(&target, fs::Permissions::from_mode(wanted))
                .map_err(|source| io_error(&target, source))?;
        }
        #[cfg(not(unix))]
        let _ = mode;

        on_file(&relative, hash);
    }
    Ok(())
}

fn io_error(path: &Path, source: io::Error) -> ArchiveError {
    ArchiveError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Pourquoi la copie s'est arrêtée (autrement qu'à cause du zip lui-même).
enum Failure {
    TooLarge,
    Write(io::Error),
}

/// Lecteur qui recopie tout ce qu'il lit dans un fichier, en comptant les octets.
///
/// Rust : `impl Read` pour notre propre type permet de le passer à `Sha256::of_reader`.
/// Le `'a` dit que `input` emprunte l'entrée du zip pendant la durée de la copie.
struct Tee<'a, R: Read> {
    input: &'a mut R,
    output: BufWriter<File>,
    written: u64,
    /// Octets qu'il reste le droit d'écrire pour toute l'archive.
    budget: u64,
    failure: Option<Failure>,
}

impl<R: Read> Read for Tee<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.input.read(buffer)?;
        let after = self.written.saturating_add(count as u64);
        if after > self.budget {
            self.failure = Some(Failure::TooLarge);
            return Err(io::Error::other("limite d'octets dépassée"));
        }
        if let Err(error) = self.output.write_all(&buffer[..count]) {
            // On garde l'erreur d'origine pour la remonter avec le bon chemin.
            self.failure = Some(Failure::Write(io::Error::new(
                error.kind(),
                error.to_string(),
            )));
            return Err(error);
        }
        self.written = after;
        Ok(count)
    }
}

/// Crée (si besoin) chaque dossier de `relative` sous `root`, en refusant tout lien.
///
/// On regarde chaque étape avec `symlink_metadata`, qui ne suit pas les liens :
/// un dossier déjà présent qui est un lien symbolique ou une jonction fait échouer.
fn ensure_dir_chain(root: &Path, relative: &Path) -> Result<(), ArchiveError> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                if is_link(&meta) {
                    return Err(ArchiveError::Link(current.display().to_string()));
                }
                if !meta.is_dir() {
                    return Err(io_error(
                        &current,
                        io::Error::new(io::ErrorKind::AlreadyExists, "un fichier occupe la place"),
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|source| io_error(&current, source))?;
            }
            Err(source) => return Err(io_error(&current, source)),
        }
    }
    Ok(())
}

/// Avant d'écrire un fichier : la cible ne doit pas être un lien (on écrirait ailleurs)
/// ni un dossier.
fn refuse_link_or_dir(target: &Path) -> Result<(), ArchiveError> {
    match fs::symlink_metadata(target) {
        Ok(meta) => {
            if is_link(&meta) {
                return Err(ArchiveError::Link(target.display().to_string()));
            }
            if meta.is_dir() {
                return Err(io_error(
                    target,
                    io::Error::new(io::ErrorKind::AlreadyExists, "un dossier occupe la place"),
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(target, source)),
    }
}

/// Lien symbolique, ou point de reparse Windows (jonction, montage…).
fn is_link(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT : couvre les jonctions et autres points de reparse.
        const REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chemin_normal_accepte() {
        assert_eq!(
            safe_relative_path("a/b\\c.txt").unwrap(),
            PathBuf::from("a").join("b").join("c.txt")
        );
        assert_eq!(
            safe_relative_path("dossier/").unwrap(),
            PathBuf::from("dossier")
        );
    }

    #[test]
    fn chemins_dangereux_refuses() {
        for bad in [
            "", "..", "../x", "/abs", "C:\\x", "a:b", "a/CON", "nul.txt", "a/b.",
        ] {
            assert!(
                safe_relative_path(bad).is_err(),
                "{bad} aurait dû être refusé"
            );
        }
    }
}
