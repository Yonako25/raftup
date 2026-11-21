//! Mondes natifs de la Bibliothèque (ADR 0046) : les lister, en installer un depuis le
//! registre signé, en désinstaller un, et convertir un apworld installé en monde natif.
//!
//! Un monde natif (ADR 0041) vit dans `mondes-natifs/` (dossier des données), soit en
//! fichier `<clé>.toml` posé à la main, soit en dossier `<clé>/` avec son `world.toml`.
//! Les dossiers posés par Raftup gardent un **reçu** (`raftup-paquet.toml`) : d'où vient
//! le monde (registre ou conversion), sa version, son empreinte, et la liste des fichiers
//! posés, la seule que la désinstallation efface.
//!
//! Rien n'est exécuté ici : un monde natif est une donnée, lue par `worlds-native`.
//! La conversion seule passe par le générateur Python, dans son bac à sable (ADR 0034).

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use serde::{Deserialize, Serialize};
use worlds_native::NativeWorld;

use crate::download::Fetcher;
use crate::generation::{
    export_worlds_json, scan_native_worlds, GenerateRequest, GenerationError, PythonBackend,
};
use crate::paths::Paths;
use crate::registry::{fetch_package, Index, Package, PackageKind, RegistryLoadError, TrustLevel};
use py_bridge::CancelToken;

/// Nom du monde dans un dossier.
pub const WORLD_FILE: &str = "world.toml";
/// Reçu d'un monde posé par Raftup.
pub const RECEIPT_FILE: &str = "raftup-paquet.toml";
/// Export JSON gardé à côté d'un monde converti (celui que lit `raftup-monde importer`).
pub const EXPORT_FILE: &str = "export.json";
/// Version du format du reçu.
pub const RECEIPT_FORMAT_VERSION: u32 = 1;
/// Nombre maximal de fichiers dans un paquet de monde natif (monde, licence, lisez-moi…).
pub const MAX_PACKAGE_FILES: usize = 32;
/// Taille maximale d'un paquet une fois décompressé (8 Mio : un monde fait au plus 4 Mio).
pub const MAX_PACKAGE_UNPACKED: u64 = 8 * 1024 * 1024;
/// Taille maximale d'un reçu.
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;
/// Taille maximale du `archipelago.json` lu dans un apworld.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// Ce qui empêche d'installer, de désinstaller ou de convertir un monde natif.
#[derive(Debug, thiserror::Error)]
pub enum NativeWorldError {
    /// Le registre ou le téléchargement a refusé le paquet (signature, sha256, taille…).
    #[error(transparent)]
    Registry(#[from] RegistryLoadError),
    /// Le générateur n'a pas pu produire l'export.
    #[error(transparent)]
    Generation(#[from] GenerationError),
    /// Aucun monde natif de ce nom dans l'index.
    #[error("monde natif « {0} » absent du registre")]
    UnknownPackage(String),
    /// Le contenu du paquet est refusé (zip dangereux, monde illisible, mauvais jeu…).
    #[error("paquet « {name} » refusé : {reason}")]
    BadPackage { name: String, reason: String },
    /// Un autre monde porte déjà ce nom dans `mondes-natifs/` : on ne l'écrase pas.
    #[error("un autre monde « {} » est déjà installé : Raftup ne l'écrase pas", path.display())]
    OtherPresent { path: PathBuf },
    /// Un monde déjà installé fournit ce jeu (deux mondes du même jeu ne servent à rien :
    /// le second serait écarté à la lecture).
    #[error("le jeu « {game} » est déjà fourni par le monde natif « {key} »")]
    GameAlreadyProvided { game: String, key: String },
    /// Nom refusé comme nom de fichier (séparateur, `..`, caractère interdit).
    #[error("nom refusé : « {0} »")]
    BadName(String),
    /// Aucun monde de ce nom dans `mondes-natifs/`.
    #[error("aucun monde natif « {0} » dans le dossier")]
    NotInstalled(String),
    /// L'export n'a pas pu devenir un monde natif (ADR 0042).
    #[error("conversion impossible : {0}")]
    Convert(String),
    /// Lecture ou écriture d'un fichier.
    #[error("{} : {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

// ---------------------------------------------------------------------------
// Reçu.
// ---------------------------------------------------------------------------

/// Reçu d'un monde posé par Raftup (`raftup-paquet.toml`, versionné).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeReceipt {
    pub format_version: u32,
    /// Fichiers posés par Raftup (noms simples), effacés à la désinstallation.
    pub files: Vec<String>,
    /// Monde venu du registre signé.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<RegistryOrigin>,
    /// Monde converti depuis un apworld installé.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversion: Option<ConversionOrigin>,
}

/// Ce que le registre disait du paquet installé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryOrigin {
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub trust: TrustLevel,
    pub license: String,
    pub authors: Vec<String>,
}

/// D'où vient un monde converti.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversionOrigin {
    /// Nom du fichier de l'apworld converti.
    pub apworld: String,
    /// Version d'Archipelago qui a fait l'export.
    pub ap_version: String,
    /// Nombre de points à vérifier laissés par le convertisseur (listés en tête du monde).
    pub notes: usize,
}

/// Lit le reçu d'un dossier de monde ; `None` s'il manque ou s'il est illisible (un monde
/// posé à la main n'en a pas).
pub fn read_receipt(world_dir: &Path) -> Option<NativeReceipt> {
    let path = world_dir.join(RECEIPT_FILE);
    let file = fs::File::open(&path).ok()?;
    let mut text = String::new();
    // `take` : un reçu énorme n'est pas chargé en entier.
    file.take(MAX_RECEIPT_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    let receipt: NativeReceipt = toml::from_str(&text).ok()?;
    (receipt.format_version == RECEIPT_FORMAT_VERSION).then_some(receipt)
}

// ---------------------------------------------------------------------------
// Liste.
// ---------------------------------------------------------------------------

/// Un monde natif installé, tel que la Bibliothèque l'affiche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledNativeWorld {
    /// Nom du fichier ou du dossier dans `mondes-natifs/` : l'identifiant du monde.
    pub key: String,
    /// Le fichier `.toml` ou le dossier.
    pub path: PathBuf,
    pub game: String,
    pub format_version: i64,
    pub items: usize,
    pub locations: usize,
    pub events: usize,
    pub regions: usize,
    pub options: usize,
    /// Reçu (dossier posé par Raftup) ; `None` pour un monde posé à la main.
    pub receipt: Option<NativeReceipt>,
}

impl InstalledNativeWorld {
    /// Version annoncée par le registre, s'il y en a une (un `world.toml` n'a pas de version).
    pub fn version(&self) -> Option<&str> {
        self.receipt
            .as_ref()
            .and_then(|r| r.registry.as_ref())
            .map(|r| r.version.as_str())
    }

    /// D'où vient le monde, en quelques mots.
    pub fn origin_label(&self) -> String {
        match self.receipt.as_ref() {
            Some(NativeReceipt {
                registry: Some(r), ..
            }) => format!("Registre signé · paquet {}", r.name),
            Some(NativeReceipt {
                conversion: Some(c),
                ..
            }) => format!("Converti depuis {}", c.apworld),
            _ => "Posé à la main dans mondes-natifs".to_string(),
        }
    }
}

/// Mondes natifs du dossier, et ceux qui ont été écartés.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeLibrary {
    pub worlds: Vec<InstalledNativeWorld>,
    /// `(chemin, raison)` de chaque monde écarté (illisible, jeu en double).
    pub rejected: Vec<(PathBuf, String)>,
}

impl NativeLibrary {
    pub fn get(&self, key: &str) -> Option<&InstalledNativeWorld> {
        self.worlds.iter().find(|w| w.key == key)
    }
}

/// Lit `mondes-natifs/` comme le moteur de génération (mêmes règles, même ordre), en
/// gardant pour chaque monde sa fiche. Bloquant mais rapide (fichiers bornés à 4 Mio).
pub fn list_native_worlds(dir: &Path) -> NativeLibrary {
    let (worlds, rejected) = scan_native_worlds(dir);
    let worlds = worlds
        .into_iter()
        .map(|(path, world)| describe(path, &world))
        .collect();
    NativeLibrary { worlds, rejected }
}

fn describe(path: PathBuf, world: &NativeWorld) -> InstalledNativeWorld {
    let key = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let receipt = if path.is_dir() {
        read_receipt(&path)
    } else {
        None
    };
    InstalledNativeWorld {
        key,
        game: world.game.clone(),
        format_version: world.format_version,
        items: world.items.len(),
        locations: world.locations.len(),
        events: world.events.len(),
        regions: world.regions.len(),
        options: world.options.len(),
        receipt,
        path,
    }
}

// ---------------------------------------------------------------------------
// Contenu d'un paquet.
// ---------------------------------------------------------------------------

/// Un paquet de monde natif lu et vérifié : ses fichiers et le monde.
#[derive(Debug, Clone)]
pub struct NativePackage {
    /// `(nom simple, octets)`, dans l'ordre du paquet ; `world.toml` est toujours là.
    pub files: Vec<(String, Vec<u8>)>,
    pub world: NativeWorld,
}

/// Lit un paquet de monde natif : un `world.toml` seul (texte UTF-8), ou un zip qui le
/// contient, à la racine ou dans un dossier `<name>/`, avec au plus quelques fichiers
/// simples à côté (licence, lisez-moi). Le monde est relu par `worlds-native` ; s'il
/// annonce un jeu (`game`), ce doit être celui du monde.
///
/// Sert au launcher (installation) et à la CI du registre (`raftup-registry check`).
pub fn read_native_package(
    name: &str,
    game: Option<&str>,
    data: &[u8],
) -> Result<NativePackage, String> {
    // Un zip commence par « PK\x03\x04 » ; tout le reste est lu comme un `world.toml`.
    let files = if data.starts_with(b"PK\x03\x04") {
        read_zip(name, data)?
    } else {
        vec![(WORLD_FILE.to_string(), data.to_vec())]
    };
    let world_bytes = files
        .iter()
        .find(|(n, _)| n == WORLD_FILE)
        .map(|(_, b)| b)
        .ok_or_else(|| format!("« {WORLD_FILE} » manquant dans l'archive"))?;
    let text = std::str::from_utf8(world_bytes)
        .map_err(|_| format!("« {WORLD_FILE} » n'est pas en UTF-8"))?;
    let world = worlds_native::parse_world(text).map_err(|e| format!("monde illisible : {e}"))?;
    if let Some(expected) = game {
        if world.game != expected {
            return Err(format!(
                "le monde décrit le jeu « {} », le registre annonce « {expected} »",
                world.game
            ));
        }
    }
    Ok(NativePackage { files, world })
}

/// Lit les fichiers d'un zip de monde natif (table des matières bornée, chemins sûrs).
fn read_zip(name: &str, data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|e| format!("archive zip illisible : {e}"))?;
    if zip.len() > MAX_PACKAGE_FILES * 2 {
        return Err(format!(
            "plus de {MAX_PACKAGE_FILES} fichiers dans l'archive"
        ));
    }
    let prefix = format!("{name}/");
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| format!("entrée n° {i} illisible : {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let raw = entry
            .name()
            .map_err(|e| format!("nom de l'entrée n° {i} illisible : {e}"))?
            .replace('\\', "/");
        // À la racine, ou dans le seul dossier `<name>/` ; jamais plus profond.
        let file_name = raw.strip_prefix(&prefix).unwrap_or(&raw).to_string();
        if !is_plain_name(&file_name) || file_name == RECEIPT_FILE {
            return Err(format!("chemin « {raw} » refusé dans l'archive"));
        }
        if files.iter().any(|(n, _)| *n == file_name) {
            return Err(format!("« {file_name} » présent deux fois"));
        }
        if files.len() >= MAX_PACKAGE_FILES {
            return Err(format!(
                "plus de {MAX_PACKAGE_FILES} fichiers dans l'archive"
            ));
        }
        let mut bytes = Vec::new();
        let left = MAX_PACKAGE_UNPACKED - total;
        // `take(left + 1)` : un octet de trop suffit à savoir que l'archive est trop grosse,
        // même si sa table des matières ment sur les tailles.
        entry
            .take(left + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("« {file_name} » illisible : {e}"))?;
        total += bytes.len() as u64;
        if total > MAX_PACKAGE_UNPACKED {
            return Err(format!(
                "plus de {MAX_PACKAGE_UNPACKED} octets une fois décompressée"
            ));
        }
        files.push((file_name, bytes));
    }
    Ok(files)
}

/// Un nom de fichier simple : `[A-Za-z0-9._-]`, 1 à 64 caractères, sans point au début
/// (ni `.`, ni `..`, ni fichier caché), donc sans séparateur ni lecteur.
pub fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

// ---------------------------------------------------------------------------
// Installation depuis le registre.
// ---------------------------------------------------------------------------

/// Résultat de [`install_native_world`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeInstall {
    /// Monde téléchargé, vérifié et posé dans ce dossier.
    Installed(PathBuf),
    /// Le même paquet (même sha256) était déjà là : rien n'a été téléchargé.
    AlreadyThere(PathBuf),
}

/// Télécharge le monde natif `name` de l'index (taille et sha256 contrôlés, l'index étant
/// déjà vérifié par sa signature), le relit, puis le pose dans `<dir>/<name>/` avec son
/// reçu. Les fichiers sont d'abord écrits à côté, puis le dossier est renommé : jamais de
/// monde à moitié posé dans `mondes-natifs/`.
///
/// Refusé : un autre monde du même nom (jamais écrasé), ou un monde installé qui fournit
/// déjà ce jeu.
pub fn install_native_world(
    fetcher: &dyn Fetcher,
    index: &Index,
    name: &str,
    dir: &Path,
) -> Result<NativeInstall, NativeWorldError> {
    let package = index
        .get(PackageKind::NativeWorld, name)
        .ok_or_else(|| NativeWorldError::UnknownPackage(name.to_string()))?;
    // Le nom vient d'un index signé et validé (`[a-z0-9_-]`) ; on revérifie quand même :
    // il devient un nom de dossier.
    if !is_plain_name(name) {
        return Err(NativeWorldError::BadName(name.to_string()));
    }
    let target = dir.join(name);
    if let Some(present) = already_installed(dir, name, package)? {
        return Ok(NativeInstall::AlreadyThere(present));
    }
    let data = fetch_package(fetcher, package)?;
    let content = read_native_package(name, package.game.as_deref(), &data).map_err(|reason| {
        NativeWorldError::BadPackage {
            name: name.to_string(),
            reason,
        }
    })?;
    check_game_free(dir, &content.world.game)?;
    let receipt = NativeReceipt {
        format_version: RECEIPT_FORMAT_VERSION,
        files: content.files.iter().map(|(n, _)| n.clone()).collect(),
        registry: Some(RegistryOrigin {
            name: package.name.clone(),
            version: package.version.clone(),
            sha256: package.sha256.clone(),
            trust: package.trust.level,
            license: package.license.clone(),
            authors: package.authors.clone(),
        }),
        conversion: None,
    };
    stage_and_place(dir, name, &content.files, &receipt)?;
    Ok(NativeInstall::Installed(target))
}

/// Le paquet est-il déjà posé ? `Some(dossier)` si c'est le même (même sha256 dans le
/// reçu) ; erreur si un autre monde occupe le nom.
fn already_installed(
    dir: &Path,
    name: &str,
    package: &Package,
) -> Result<Option<PathBuf>, NativeWorldError> {
    let target = dir.join(name);
    let loose = dir.join(format!("{name}.toml"));
    if loose.exists() {
        return Err(NativeWorldError::OtherPresent { path: loose });
    }
    if !target.exists() {
        return Ok(None);
    }
    let same = read_receipt(&target)
        .and_then(|r| r.registry)
        .is_some_and(|r| r.name == package.name && r.sha256 == package.sha256);
    if same {
        Ok(Some(target))
    } else {
        Err(NativeWorldError::OtherPresent { path: target })
    }
}

/// Refuse un jeu déjà fourni par un monde installé.
fn check_game_free(dir: &Path, game: &str) -> Result<(), NativeWorldError> {
    let installed = list_native_worlds(dir);
    match installed.worlds.iter().find(|w| w.game == game) {
        Some(w) => Err(NativeWorldError::GameAlreadyProvided {
            game: game.to_string(),
            key: w.key.clone(),
        }),
        None => Ok(()),
    }
}

/// Dossier de préparation, à côté de `mondes-natifs/` (et pas dedans : un monde à moitié
/// écrit n'y est jamais lu).
fn staging_root(dir: &Path) -> PathBuf {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mondes-natifs".to_string());
    dir.with_file_name(format!("{name}.partiel"))
}

/// Écrit les fichiers et le reçu dans le dossier de préparation, puis le renomme en
/// `<dir>/<key>`.
fn stage_and_place(
    dir: &Path,
    key: &str,
    files: &[(String, Vec<u8>)],
    receipt: &NativeReceipt,
) -> Result<(), NativeWorldError> {
    let staging = staging_root(dir).join(key);
    // Reste d'une installation interrompue : on retire seulement ses fichiers simples.
    if staging.exists() {
        remove_flat_dir(&staging, None)?;
    }
    fs::create_dir_all(&staging).map_err(io(&staging))?;
    for (name, bytes) in files {
        if !is_plain_name(name) || name == RECEIPT_FILE {
            return Err(NativeWorldError::BadName(name.clone()));
        }
        let path = staging.join(name);
        fs::write(&path, bytes).map_err(io(&path))?;
    }
    let text = toml::to_string(receipt).map_err(|e| NativeWorldError::Convert(e.to_string()))?;
    let receipt_path = staging.join(RECEIPT_FILE);
    fs::write(&receipt_path, text).map_err(io(&receipt_path))?;
    fs::create_dir_all(dir).map_err(io(dir))?;
    let target = dir.join(key);
    fs::rename(&staging, &target).map_err(io(&target))?;
    // Le dossier de préparation vide disparaît (meilleur effort : il ne gêne pas).
    let _ = fs::remove_dir(staging_root(dir));
    Ok(())
}

// ---------------------------------------------------------------------------
// Désinstallation.
// ---------------------------------------------------------------------------

/// Ce que [`uninstall_native_world`] a fait.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeUninstall {
    /// Fichiers effacés.
    pub removed: usize,
    /// Fichiers laissés en place (ajoutés à la main dans le dossier du monde) : le dossier
    /// reste alors, avec eux.
    pub left: Vec<String>,
}

/// Retire le monde `key` de `dir`. Un fichier `<key>.toml` est effacé. Pour un dossier,
/// seuls `world.toml`, le reçu et les fichiers que le reçu liste sont effacés, puis le
/// dossier s'il est vide ; un fichier ajouté à la main reste (et le dit). Un lien n'est
/// jamais suivi.
pub fn uninstall_native_world(dir: &Path, key: &str) -> Result<NativeUninstall, NativeWorldError> {
    if !is_plain_name(key) {
        return Err(NativeWorldError::BadName(key.to_string()));
    }
    let path = dir.join(key);
    // `symlink_metadata` : renseigne sur le lien lui-même, sans le suivre.
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(NativeWorldError::NotInstalled(key.to_string()))
        }
        Err(e) => return Err(io(&path)(e)),
    };
    if meta.is_file() {
        let is_toml = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));
        if !is_toml {
            return Err(NativeWorldError::NotInstalled(key.to_string()));
        }
        fs::remove_file(&path).map_err(io(&path))?;
        return Ok(NativeUninstall {
            removed: 1,
            left: Vec::new(),
        });
    }
    if !meta.is_dir() {
        // Lien ou autre : on n'y touche pas.
        return Err(NativeWorldError::NotInstalled(key.to_string()));
    }
    let mut known = vec![WORLD_FILE.to_string(), RECEIPT_FILE.to_string()];
    if let Some(receipt) = read_receipt(&path) {
        known.extend(receipt.files);
    }
    remove_flat_dir(&path, Some(&known))
}

/// Efface les fichiers simples d'un dossier (`only` : seulement ceux-là), puis le dossier
/// s'il est vide. Ne descend jamais dans un sous-dossier et ne suit aucun lien.
fn remove_flat_dir(
    path: &Path,
    only: Option<&[String]>,
) -> Result<NativeUninstall, NativeWorldError> {
    let mut report = NativeUninstall::default();
    let entries = fs::read_dir(path).map_err(io(path))?;
    let mut names: Vec<(String, bool)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io(path))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // `file_type` ne suit pas les liens.
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        names.push((name, is_file));
    }
    // Ordre stable pour le rapport.
    names.sort();
    for (name, is_file) in names {
        let allowed = only.is_none_or(|list| list.contains(&name));
        if is_file && allowed && is_plain_name(&name) {
            let file = path.join(&name);
            fs::remove_file(&file).map_err(io(&file))?;
            report.removed += 1;
        } else {
            report.left.push(name);
        }
    }
    if report.left.is_empty() {
        fs::remove_dir(path).map_err(io(path))?;
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// Conversion d'un apworld installé (ADR 0042, depuis le launcher).
// ---------------------------------------------------------------------------

/// Un apworld converti et posé dans `mondes-natifs/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertedWorld {
    /// Dossier du monde (`<dir>/<clé>`).
    pub path: PathBuf,
    pub game: String,
    /// Points à vérifier laissés par le convertisseur.
    pub notes: Vec<String>,
}

/// Le jeu d'un apworld, lu dans son `archipelago.json` (à la racine ou dans le dossier du
/// monde) ; `None` s'il n'y en a pas ou s'il ne le dit pas.
pub fn apworld_game(apworld: &Path) -> Option<String> {
    let file = fs::File::open(apworld).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let index = (0..zip.len()).find(|i| {
        zip.by_index(*i).ok().and_then(|e| {
            let name = e.name().ok()?.replace('\\', "/");
            Some(
                name == "archipelago.json"
                    || (name.ends_with("/archipelago.json") && name.matches('/').count() == 1),
            )
        }) == Some(true)
    })?;
    let entry = zip.by_index(index).ok()?;
    let mut text = String::new();
    entry
        .take(MAX_MANIFEST_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("game")?.as_str().map(str::to_string)
}

/// Clé du monde converti : le nom du fichier de l'apworld sans `.apworld`.
pub fn conversion_key(apworld: &Path) -> Result<String, NativeWorldError> {
    let stem = apworld
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if is_plain_name(&stem) {
        Ok(stem)
    } else {
        Err(NativeWorldError::BadName(stem))
    }
}

/// Le YAML d'un joueur seul, options par défaut. Le nom du jeu est écrit comme une chaîne
/// JSON entre guillemets, que YAML lit telle quelle : aucun caractère du nom ne peut
/// casser le fichier.
pub fn default_player_yaml(game: &str) -> String {
    let quoted = serde_json::Value::from(game).to_string();
    format!("name: Raftup\ngame: {quoted}\n{quoted}: {{}}\n")
}

/// Produit l'export JSON de l'apworld installé (générateur officiel, options par défaut,
/// un joueur), puis le convertit et le pose dans `<native_dir>/<clé>/`, avec l'export à
/// côté (`export.json`, celui que lit `raftup-monde importer`). Bloquant : à lancer dans
/// un thread. Le générateur doit être installé (`runtime_status`).
pub fn convert_installed_apworld(
    paths: &Paths,
    backend: &PythonBackend,
    apworld: &Path,
    game: &str,
    cancel: &CancelToken,
    log: Sender<String>,
) -> Result<ConvertedWorld, NativeWorldError> {
    let native_dir = paths.native_worlds_dir();
    let key = conversion_key(apworld)?;
    // Refus avant de lancer Python : un nom pris ou un jeu déjà fourni.
    check_key_free(&native_dir, &key)?;
    check_game_free(&native_dir, game)?;
    // Le fichier joueur est écrit dans le dossier de préparation, puis copié par la
    // génération dans son propre dossier.
    let staging = staging_root(&native_dir);
    fs::create_dir_all(&staging).map_err(io(&staging))?;
    let yaml = staging.join(format!("{key}.yaml"));
    fs::write(&yaml, default_player_yaml(game)).map_err(io(&yaml))?;
    let request = GenerateRequest {
        player_files: vec![yaml.clone()],
        apworlds: vec![apworld.to_path_buf()],
        seed: Some(1),
        ..GenerateRequest::default()
    };
    let exported = export_worlds_json(paths, backend, &request, cancel, log);
    let _ = fs::remove_file(&yaml);
    let bytes = exported?;
    let apworld_name = apworld
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    install_converted_world(
        &native_dir,
        &key,
        &apworld_name,
        &backend.runtime.ap_version,
        &bytes,
    )
}

/// La partie sans Python de [`convert_installed_apworld`] : l'export JSON (joueur 1)
/// devient un monde natif posé dans `<dir>/<key>/`, avec l'export et un reçu.
pub fn install_converted_world(
    dir: &Path,
    key: &str,
    apworld_name: &str,
    ap_version: &str,
    export_bytes: &[u8],
) -> Result<ConvertedWorld, NativeWorldError> {
    if !is_plain_name(key) {
        return Err(NativeWorldError::BadName(key.to_string()));
    }
    check_key_free(dir, key)?;
    let export = generate::imported::read_export(export_bytes)
        .map_err(|e| NativeWorldError::Convert(e.to_string()))?;
    let player = export
        .players
        .first()
        .map(|p| p.slot)
        .ok_or_else(|| NativeWorldError::Convert("aucun joueur dans l'export".to_string()))?;
    let conversion = generate::convert::convert(&export, player, None)
        .map_err(|e| NativeWorldError::Convert(e.to_string()))?;
    // Le convertisseur relit déjà son texte ; on le relit pour avoir le nom du jeu.
    let world = worlds_native::parse_world(&conversion.toml)
        .map_err(|e| NativeWorldError::Convert(e.to_string()))?;
    check_game_free(dir, &world.game)?;
    let files = vec![
        (WORLD_FILE.to_string(), conversion.toml.into_bytes()),
        (EXPORT_FILE.to_string(), export_bytes.to_vec()),
    ];
    let receipt = NativeReceipt {
        format_version: RECEIPT_FORMAT_VERSION,
        files: files.iter().map(|(n, _)| n.clone()).collect(),
        registry: None,
        conversion: Some(ConversionOrigin {
            apworld: apworld_name.to_string(),
            ap_version: ap_version.to_string(),
            notes: conversion.notes.len(),
        }),
    };
    stage_and_place(dir, key, &files, &receipt)?;
    Ok(ConvertedWorld {
        path: dir.join(key),
        game: world.game,
        notes: conversion.notes,
    })
}

/// Refuse une clé déjà prise (dossier ou fichier `.toml`).
fn check_key_free(dir: &Path, key: &str) -> Result<(), NativeWorldError> {
    for path in [dir.join(key), dir.join(format!("{key}.toml"))] {
        if path.exists() {
            return Err(NativeWorldError::OtherPresent { path });
        }
    }
    Ok(())
}

/// Fabrique la conversion d'une erreur d'entrée-sortie en `NativeWorldError::Io`.
/// `impl Fn` : on rend une fermeture qui garde sa propre copie du chemin.
fn io(path: &Path) -> impl Fn(std::io::Error) -> NativeWorldError {
    let path = path.to_path_buf();
    move |source| NativeWorldError::Io {
        path: path.clone(),
        source,
    }
}
