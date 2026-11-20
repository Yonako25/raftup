//! `raftup-registry check` : vérifie les paquets d'un fichier source et écrit le rapport.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use launcher_core::download::Fetcher;
use registry::index::{check_url, PACKAGE_MAX_BYTES};
use registry::publish::{read_package, read_source, SourceEntry};

use crate::check::{check_package, Candidate, CheckSettings, Generator};
use crate::report::{Report, REPORT_FORMAT_VERSION};

/// Noms des fichiers du rapport.
pub const REPORT_JSON: &str = "rapport.json";
pub const REPORT_MARKDOWN: &str = "rapport.md";

/// D'où viennent les octets des paquets.
pub enum PackageSource<'a> {
    /// Le champ `file` du fichier source (chez la mainteneuse).
    LocalFiles,
    /// L'adresse `url`, chez l'auteur (la CI n'a pas les copies locales).
    Download(&'a dyn Fetcher),
}

/// Ce que demande `check`.
pub struct CheckRequest<'a> {
    pub source: &'a Path,
    /// Ne vérifier que ce paquet (par son nom).
    pub only: Option<&'a str>,
    pub packages: PackageSource<'a>,
    /// Dossier de travail : copies `<name>.apworld` données au générateur.
    pub work_dir: &'a Path,
    pub settings: CheckSettings,
}

/// Vérifie les paquets et rend le rapport. `generator` : `Err(raison)` s'il n'y en a pas.
/// Erreur : source illisible, paquet introuvable, ou problème de l'environnement.
pub fn run_check(
    request: &CheckRequest<'_>,
    mut generator: Result<&mut dyn Generator, String>,
) -> Result<Report, String> {
    let source = read_source(request.source).map_err(|e| e.to_string())?;
    let entries: Vec<&SourceEntry> = source
        .packages
        .iter()
        .filter(|p| request.only.is_none_or(|name| p.name == name))
        .collect();
    if entries.is_empty() {
        return Err(match request.only {
            Some(name) => format!("aucun paquet « {name} » dans {}", request.source.display()),
            None => format!("aucun paquet dans {}", request.source.display()),
        });
    }
    let generator_name = generator.as_ref().ok().map(|g| g.name());
    let mut packages = Vec::new();
    for entry in entries {
        let data = load(entry, &request.packages)?;
        let package = entry.to_package(&data);
        // Un nom invalide (« ../x ») ne doit jamais servir de nom de fichier : on ne copie
        // rien, et la vérification de structure le refusera avant tout appel au générateur.
        let apworld_path = if package.validate().is_ok() {
            copy_for_generator(request.work_dir, entry, &data)?
        } else {
            request.work_dir.join("paquet-invalide.apworld")
        };
        let candidate = Candidate {
            package: &package,
            data: &data,
            apworld_path: &apworld_path,
        };
        // L'horloge du fuzzing part pour chaque paquet. `move` : la fermeture garde `start`.
        let start = Instant::now();
        let mut elapsed = move || start.elapsed();
        // On prête le générateur (`&mut **g` : on passe à travers la référence de `Ok`)
        // sans le rendre : il resservira au paquet suivant.
        let lent = match &mut generator {
            Ok(g) => Ok(&mut **g as &mut dyn Generator),
            Err(reason) => Err(reason.clone()),
        };
        let report = check_package(&candidate, lent, &request.settings, &mut elapsed)?;
        packages.push(report);
    }
    Ok(Report {
        format_version: REPORT_FORMAT_VERSION,
        tool: format!("raftup-registry {}", env!("CARGO_PKG_VERSION")),
        generator: generator_name,
        settings: request.settings.for_report(),
        packages,
    })
}

/// Écrit `rapport.json` et `rapport.md` dans `dir`. Rend leurs chemins.
pub fn write_report(report: &Report, dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    fs::create_dir_all(dir).map_err(|e| format!("{} : {e}", dir.display()))?;
    let json = dir.join(REPORT_JSON);
    let markdown = dir.join(REPORT_MARKDOWN);
    fs::write(&json, report.to_json()).map_err(|e| format!("{} : {e}", json.display()))?;
    fs::write(&markdown, report.to_markdown())
        .map_err(|e| format!("{} : {e}", markdown.display()))?;
    Ok((json, markdown))
}

/// Les octets d'un paquet, depuis le fichier local ou chez l'auteur.
fn load(entry: &SourceEntry, source: &PackageSource<'_>) -> Result<Vec<u8>, String> {
    let label = format!("{} {}", entry.kind.as_str(), entry.name);
    match source {
        PackageSource::LocalFiles => {
            read_package(&entry.file).map_err(|e| format!("{label} : {e}"))
        }
        PackageSource::Download(fetcher) => {
            check_url(&entry.url).map_err(|e| format!("{label} : adresse refusée : {e}"))?;
            let reader = fetcher
                .open(&entry.url)
                .map_err(|e| format!("{label} : téléchargement : {e}"))?;
            let mut data = Vec::new();
            // `take` borne la lecture : un fichier géant ne remplit pas la mémoire.
            reader
                .take(PACKAGE_MAX_BYTES + 1)
                .read_to_end(&mut data)
                .map_err(|e| format!("{label} : téléchargement : {e}"))?;
            if data.len() as u64 > PACKAGE_MAX_BYTES {
                return Err(format!("{label} : plus de {PACKAGE_MAX_BYTES} octets"));
            }
            Ok(data)
        }
    }
}

/// Copie le paquet sous `<work>/paquets/<name>.apworld`, le nom qu'attend Archipelago.
/// (`name` est déjà validé `[a-z0-9_-]` : il ne peut pas sortir du dossier.)
fn copy_for_generator(
    work_dir: &Path,
    entry: &SourceEntry,
    data: &[u8],
) -> Result<PathBuf, String> {
    let dir = work_dir.join("paquets");
    fs::create_dir_all(&dir).map_err(|e| format!("{} : {e}", dir.display()))?;
    let path = dir.join(format!("{}.apworld", entry.name));
    fs::write(&path, data).map_err(|e| format!("{} : {e}", path.display()))?;
    Ok(path)
}
