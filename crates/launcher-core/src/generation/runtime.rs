//! Installation du runtime de génération : Python autonome, code d'Archipelago et wheels
//! (ADR 0017). Tout vient de `generator-pins.toml`, rien n'est redistribué.
//!
//! Disposition sous le dossier de données de Raftup :
//!
//! ```text
//! python/<version>/python.exe
//! generator/<version d'Archipelago>/archipelago/
//! generator/<version d'Archipelago>/site-packages/
//! generator/<version d'Archipelago>/runtime.json   <- écrit EN DERNIER
//! ```
//!
//! `runtime.json` contient les sha256 des pins utilisés : s'il existe et correspond aux pins
//! actuels, l'installation est complète. Chaque dossier est d'abord préparé dans
//! `<dossier>.partial` puis renommé, donc une coupure ne laisse jamais un dossier à moitié rempli
//! sous son vrai nom.
//!
//! **Windows seulement pour l'instant (NF11).** Les pins visent Python pour
//! `x86_64-pc-windows-msvc` et la disposition ci-dessus (`python.exe` à la racine) est celle
//! de cette archive ; sous Linux et macOS, python-build-standalone range l'exécutable dans
//! `bin/python3`. Le code compile et ses tests passent partout, mais un vrai runtime installé
//! hors Windows ne démarrerait pas. Repli hors Windows : le moteur Rust seul (ADR 0027), qui
//! génère sans Python. Pour ouvrir un autre système : un bloc `[python]` par cible dans
//! `generator-pins.toml` (sha256 relevés sur les vrais fichiers) et un chemin d'exécutable
//! choisi par `cfg`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use py_bridge::CancelToken;

use super::{GenerationError, GeneratorPins, Runtime};
use crate::archive::{extract_tar_gz, extract_zip, plan_zip, ArchiveLimits};
use crate::download::DownloadError;
use crate::download::Downloader;
use crate::hash::Sha256;
use crate::paths::Paths;

/// Version du format de `runtime.json`.
const MARKER_FORMAT: u32 = 1;
const MARKER_FILE: &str = "runtime.json";
/// Les seuls dossiers que ce module a le droit de supprimer se terminent par ce suffixe.
const PARTIAL_SUFFIX: &str = ".partial";

/// État du runtime sur le disque.
#[derive(Debug, Clone)]
pub enum RuntimeStatus {
    /// Rien d'installé (ou installation interrompue). `download_bytes` est la somme des
    /// tailles réelles (`size`) épinglées, à montrer au joueur avant le téléchargement.
    Missing { download_bytes: u64 },
    /// Installation complète et conforme aux pins.
    Ready(Runtime),
    /// Installation présente mais inutilisable ou périmée ; la raison est lisible.
    Broken(String),
}

/// Étape en cours, pour la progression affichée au joueur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupStep {
    Python,
    Archipelago,
    /// Nom de la wheel.
    Wheel(String),
    /// Extraction, mise en place et écriture du marqueur.
    Finalize,
}

/// Contenu de `runtime.json`.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Marker {
    format_version: u32,
    python: Sha256,
    archipelago: Sha256,
    /// Nom de fichier de la wheel -> sha256. `BTreeMap` : ordre stable dans le fichier.
    wheels: BTreeMap<String, Sha256>,
    /// Nom de la source -> sha256. `#[serde(default)]` : un ancien marqueur sans cette
    /// clé se lit avec une liste vide, puis diffère des pins actuels (réinstallation).
    #[serde(default)]
    sources: BTreeMap<String, Sha256>,
}

impl Marker {
    fn from_pins(pins: &GeneratorPins) -> Marker {
        Marker {
            format_version: MARKER_FORMAT,
            python: pins.python.sha256,
            archipelago: pins.archipelago.sha256,
            wheels: pins
                .wheels
                .iter()
                .map(|w| (w.filename.clone(), w.sha256))
                .collect(),
            sources: pins
                .source_packages
                .iter()
                .map(|s| (s.name.clone(), s.sha256))
                .collect(),
        }
    }
}

fn io_err(path: &Path, source: std::io::Error) -> GenerationError {
    GenerationError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Les chemins du runtime pour ces pins.
fn runtime_for(paths: &Paths, pins: &GeneratorPins) -> Runtime {
    let generator = paths.generator_dir(&pins.archipelago.version);
    Runtime {
        python_exe: paths.python_dir(&pins.python.version).join("python.exe"),
        archipelago_dir: generator.join("archipelago"),
        site_packages: Some(generator.join("site-packages")),
        ap_version: pins.archipelago.version.clone(),
    }
}

/// Regarde le disque sans rien modifier ni télécharger.
pub fn runtime_status(paths: &Paths, pins: &GeneratorPins) -> RuntimeStatus {
    let marker_path = paths
        .generator_dir(&pins.archipelago.version)
        .join(MARKER_FILE);
    if !marker_path.exists() {
        return RuntimeStatus::Missing {
            download_bytes: pins.download_bytes(),
        };
    }

    let text = match fs::read_to_string(&marker_path) {
        Ok(text) => text,
        Err(e) => return RuntimeStatus::Broken(format!("{MARKER_FILE} illisible : {e}")),
    };
    let marker: Marker = match serde_json::from_str(&text) {
        Ok(marker) => marker,
        Err(e) => return RuntimeStatus::Broken(format!("{MARKER_FILE} invalide : {e}")),
    };
    if marker.format_version != MARKER_FORMAT {
        return RuntimeStatus::Broken(format!(
            "{MARKER_FILE} : format {} inconnu",
            marker.format_version
        ));
    }
    // Le marqueur doit décrire exactement les pins actuels (même égalité sur toute la structure).
    if marker != Marker::from_pins(pins) {
        return RuntimeStatus::Broken(
            "les versions épinglées ont changé depuis l'installation".into(),
        );
    }

    let runtime = runtime_for(paths, pins);
    let site = runtime.site_packages.as_deref();
    if !runtime.python_exe.is_file() {
        return RuntimeStatus::Broken(format!("{} manquant", runtime.python_exe.display()));
    }
    if !runtime.archipelago_dir.is_dir() {
        return RuntimeStatus::Broken(format!("{} manquant", runtime.archipelago_dir.display()));
    }
    if let Some(site) = site {
        if !site.is_dir() {
            return RuntimeStatus::Broken(format!("{} manquant", site.display()));
        }
    }
    RuntimeStatus::Ready(runtime)
}

/// Installe (ou répare) le runtime, puis le rend. Sans effet si déjà `Ready`.
///
/// `progress` reçoit l'étape et le nombre d'octets reçus pour cette étape.
/// Tous les fichiers sont téléchargés et vérifiés AVANT toute extraction : un mauvais hash
/// ne laisse donc aucun dossier derrière lui.
///
/// Prend le même verrou que `generate` : une réinstallation ne tourne jamais pendant une
/// génération, ni deux installations en parallèle (`GenerationError::Busy` sinon). Ce verrou
/// ne vaut que pour CE process : deux instances du launcher ne se voient pas.
// `&mut dyn FnMut` : une fonction de rappel qui peut modifier son environnement,
// empruntée sans être copiée ; `dyn` = choisie à l'exécution.
pub fn ensure_runtime(
    paths: &Paths,
    pins: &GeneratorPins,
    downloader: &Downloader,
    progress: &mut dyn FnMut(SetupStep, u64),
) -> Result<Runtime, GenerationError> {
    ensure_runtime_cancellable(paths, pins, downloader, progress, &CancelToken::new())
}

/// Comme [`ensure_runtime`], annulable par `cancel` pendant les téléchargements
/// (`GenerationError::Cancelled`). Une fois l'extraction commencée, elle va au bout :
/// elle est courte, et chaque dossier est mis en place d'un coup (`.partial` puis renommage).
/// Les fichiers déjà vérifiés restent dans le cache : une nouvelle tentative ne les
/// retélécharge pas.
pub fn ensure_runtime_cancellable(
    paths: &Paths,
    pins: &GeneratorPins,
    downloader: &Downloader,
    progress: &mut dyn FnMut(SetupStep, u64),
    cancel: &CancelToken,
) -> Result<Runtime, GenerationError> {
    if let RuntimeStatus::Ready(runtime) = runtime_status(paths, pins) {
        return Ok(runtime);
    }
    // Rien à faire de plus si c'est prêt ; sinon on prend le verrou (libéré par `Drop`),
    // puis on revérifie : une autre installation a pu finir entre-temps.
    let _guard = super::run::LockGuard::acquire()?;
    if let RuntimeStatus::Ready(runtime) = runtime_status(paths, pins) {
        return Ok(runtime);
    }

    // 1. Téléchargements (cache par sha256 : rien n'est rouvert s'il est déjà là).
    let python_file = {
        let step = SetupStep::Python;
        downloader
            .get_cancellable(
                &pins.python.url,
                &pins.python.sha256,
                pins.python.max_bytes,
                &mut |n| progress(step.clone(), n),
                cancel,
            )
            .map_err(cancelled)?
    };
    let ap_file = {
        let step = SetupStep::Archipelago;
        downloader
            .get_cancellable(
                &pins.archipelago.url,
                &pins.archipelago.sha256,
                pins.archipelago.max_bytes,
                &mut |n| progress(step.clone(), n),
                cancel,
            )
            .map_err(cancelled)?
    };
    let mut wheel_files: Vec<(&crate::generation::PinnedWheel, PathBuf)> = Vec::new();
    for wheel in &pins.wheels {
        let step = SetupStep::Wheel(wheel.name.clone());
        let file = downloader
            .get_cancellable(
                &wheel.url,
                &wheel.sha256,
                wheel.max_bytes,
                &mut |n| progress(step.clone(), n),
                cancel,
            )
            .map_err(cancelled)?;
        wheel_files.push((wheel, file));
    }

    // Sources Python pures (sans wheel) : même téléchargement vérifié par sha256.
    let mut source_files: Vec<(&crate::generation::PinnedSource, PathBuf)> = Vec::new();
    for source in &pins.source_packages {
        // La progression se range sous « Wheel » : c'est une bibliothèque comme les autres
        // pour le joueur (on évite une nouvelle étape dans l'interface).
        let step = SetupStep::Wheel(source.name.clone());
        let file = downloader
            .get_cancellable(
                &source.url,
                &source.sha256,
                source.max_bytes,
                &mut |n| progress(step.clone(), n),
                cancel,
            )
            .map_err(cancelled)?;
        source_files.push((source, file));
    }

    // Dernière occasion d'annuler : après, on extrait jusqu'au bout.
    if cancel.is_cancelled() {
        return Err(GenerationError::Cancelled);
    }

    // 2. Mise en place.
    progress(SetupStep::Finalize, 0);
    let runtime = runtime_for(paths, pins);
    let generator = paths.generator_dir(&pins.archipelago.version);

    // Python : `python/<version>/` ; le dossier racine de l'archive est retiré.
    let python_dir = paths.python_dir(&pins.python.version);
    install_dir(paths, &python_dir, &mut |target| {
        let limits = ArchiveLimits {
            max_entries: pins.python.max_entries,
            max_total_bytes: pins.python.max_bytes.saturating_mul(8),
        };
        extract_tar_gz(
            &python_file,
            &pins.python.strip_prefix,
            target,
            &limits,
            &mut |_, _| {},
        )?;
        // Une archive sans python.exe à la racine n'est pas celle qu'on attend.
        let exe = target.join("python.exe");
        if !exe.is_file() {
            return Err(io_err(
                &exe,
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "python.exe absent de l'archive",
                ),
            ));
        }
        Ok(())
    })?;

    // Archipelago.
    install_dir(paths, &runtime.archipelago_dir, &mut |target| {
        let limits = ArchiveLimits {
            max_entries: pins.archipelago.max_entries,
            max_total_bytes: pins.archipelago.max_bytes.saturating_mul(8),
        };
        extract_zip(
            &ap_file,
            &pins.archipelago.strip_prefix,
            target,
            &limits,
            &mut |_, _| {},
        )?;
        Ok(())
    })?;

    // Wheels : des zips décompressés directement dans `site-packages`.
    let site_packages = generator.join("site-packages");
    install_dir(paths, &site_packages, &mut |target| {
        // 1. Tous les contrôles sur TOUTES les wheels avant d'écrire quoi que ce soit.
        // `BTreeSet` commun : un même fichier ne peut venir que d'une seule wheel
        // (comparaison insensible à la casse, comme le système de fichiers Windows).
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (wheel, file) in &wheel_files {
            let limits = wheel_limits(wheel);
            // `plan_zip` refuse déjà les chemins dangereux.
            for relative in plan_zip(file, "", &limits)? {
                let key = relative.to_string_lossy().replace('\\', "/").to_lowercase();
                // Une wheel avec `*.data/` demande un vrai installeur (scripts, headers…) :
                // on ne sait pas l'installer proprement, donc on refuse.
                let first = key.split('/').next().unwrap_or("");
                if first.ends_with(".data") {
                    return Err(GenerationError::UnsupportedWheel {
                        wheel: wheel.filename.clone(),
                        reason: "elle contient un dossier .data".to_string(),
                    });
                }
                if !seen.insert(key) {
                    return Err(GenerationError::DuplicateWheelFile {
                        wheel: wheel.filename.clone(),
                        path: relative.display().to_string(),
                    });
                }
            }
        }
        // Les sources : leurs fichiers atterrissent sous `<install_as>/`, et ne doivent
        // écraser aucun fichier d'une wheel (même contrôle, sans casse).
        for (source, file) in &source_files {
            for relative in plan_zip(file, &source.strip_prefix, &source_limits(source))? {
                let key = format!(
                    "{}/{}",
                    source.install_as,
                    relative.to_string_lossy().replace('\\', "/")
                )
                .to_lowercase();
                if !seen.insert(key) {
                    return Err(GenerationError::DuplicateWheelFile {
                        wheel: source.name.clone(),
                        path: relative.display().to_string(),
                    });
                }
            }
        }
        // 2. Extraction.
        for (wheel, file) in &wheel_files {
            extract_zip(file, "", target, &wheel_limits(wheel), &mut |_, _| {})?;
        }
        for (source, file) in &source_files {
            extract_zip(
                file,
                &source.strip_prefix,
                &target.join(&source.install_as),
                &source_limits(source),
                &mut |_, _| {},
            )?;
        }
        Ok(())
    })?;

    // 3. Le marqueur, en dernier : sans lui, l'installation est considérée incomplète.
    let marker_path = generator.join(MARKER_FILE);
    let text = serde_json::to_string_pretty(&Marker::from_pins(pins))
        .map_err(|e| GenerationError::Pins(format!("{MARKER_FILE} : {e}")))?;
    let tmp = generator.join(format!("{MARKER_FILE}.tmp"));
    fs::write(&tmp, text).map_err(|e| io_err(&tmp, e))?;
    fs::rename(&tmp, &marker_path).map_err(|e| io_err(&marker_path, e))?;

    Ok(runtime)
}

/// Une annulation du téléchargement devient l'annulation de l'installation ; les autres
/// erreurs de téléchargement restent telles quelles.
fn cancelled(error: DownloadError) -> GenerationError {
    match error {
        DownloadError::Cancelled => GenerationError::Cancelled,
        other => GenerationError::Download(other),
    }
}

fn wheel_limits(wheel: &crate::generation::PinnedWheel) -> ArchiveLimits {
    ArchiveLimits {
        max_entries: 5_000,
        max_total_bytes: wheel.max_bytes.saturating_mul(10),
    }
}

fn source_limits(source: &crate::generation::PinnedSource) -> ArchiveLimits {
    ArchiveLimits {
        max_entries: source.max_entries,
        max_total_bytes: source.max_bytes.saturating_mul(10),
    }
}

/// `<dossier><suffixe>` à côté du dossier (le nom peut contenir des points ou un `+`).
fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let mut name = dir
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    dir.with_file_name(name)
}

/// Supprime un dossier `.partial`. Refuse tout autre chemin : c'est la seule suppression
/// récursive du module, bornée au dossier de données de Raftup.
fn remove_partial(paths: &Paths, dir: &Path) -> Result<(), GenerationError> {
    let is_partial = dir
        .file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(PARTIAL_SUFFIX));
    if !is_partial || !dir.starts_with(&paths.data_dir) {
        return Err(io_err(
            dir,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "suppression refusée : pas un dossier .partial de Raftup",
            ),
        ));
    }
    if dir.is_dir() {
        fs::remove_dir_all(dir).map_err(|e| io_err(dir, e))?;
    }
    Ok(())
}

/// Prépare `final_dir` de façon atomique : `fill` remplit `<final_dir>.partial`, puis on renomme.
/// Un éventuel ancien `final_dir` est d'abord renommé en `.stale.partial` (donc supprimable),
/// il n'est jamais supprimé sous son vrai nom.
fn install_dir(
    paths: &Paths,
    final_dir: &Path,
    fill: &mut dyn FnMut(&Path) -> Result<(), GenerationError>,
) -> Result<(), GenerationError> {
    if let Some(parent) = final_dir.parent() {
        fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    let partial = sibling(final_dir, PARTIAL_SUFFIX);
    let stale = sibling(final_dir, ".stale.partial");

    // Restes d'une tentative interrompue.
    remove_partial(paths, &partial)?;
    remove_partial(paths, &stale)?;

    if let Err(error) = fill(&partial) {
        // On garde l'erreur d'origine, plus utile que celle du nettoyage.
        let _ = remove_partial(paths, &partial);
        return Err(error);
    }

    if final_dir.exists() {
        fs::rename(final_dir, &stale).map_err(|e| io_err(final_dir, e))?;
    }
    if let Err(e) = fs::rename(&partial, final_dir) {
        // On remet l'ancien en place si possible.
        if stale.exists() {
            let _ = fs::rename(&stale, final_dir);
        }
        let _ = remove_partial(paths, &partial);
        return Err(io_err(final_dir, e));
    }
    remove_partial(paths, &stale)
}
