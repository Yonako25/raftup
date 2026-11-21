//! Lecture et vérification du registre de paquets signés (V2, ADR 0032).
//!
//! Le registre complète le catalogue embarqué (ADR 0001) sans le remplacer :
//! 1. [`fetch_verified_index`] télécharge `index.json` et sa signature, et vérifie la
//!    signature minisign **avant** de lire le JSON ;
//! 2. [`fetch_package`] télécharge un paquet (borné à sa taille) et contrôle son sha256 ;
//! 3. [`add_registry_profiles`] ajoute au catalogue les profils du registre pour des jeux
//!    absents du catalogue embarqué : **le catalogue embarqué gagne toujours**.
//!
//! 4. [`fetch_registry`] puis [`apply_registry`] font les trois à la suite pour la
//!    Bibliothèque ;
//! 5. [`install_apworld`] pose une apworld vérifiée dans le dossier des mondes.
//!
//! Rien n'est exécuté ici : une apworld posée n'est lue que par le générateur.
//!
//! Note : la crate s'appelle aussi `registry`. On écrit `::registry::…` (chemin qui part de
//! la liste des crates) pour ne pas la confondre avec ce module, `crate::registry`.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use ::registry::{check_package, verify_index, RegistryError};
// `pub use` : l'interface (`launcher-ui`) ne dépend que de `launcher-core` ; elle trouve ici
// les types de l'index sans dépendre elle-même de la crate `registry`.
pub use ::registry::{Index, Package, PackageKind, Trust, TrustLevel};

use crate::catalog::Catalog;
use crate::download::{DownloadError, Fetcher};
use crate::profile::Profile;

/// Clé publique minisign du registre (ligne base64 « RW… », identifiant `40CD99C56F16E1EA`),
/// **distincte** de celle des mises à jour (ADR 0032). Créée le 19 novembre 2026 par
/// `raftup-registry keygen` ; la clé secrète reste hors du dépôt, chez la mainteneuse.
pub const REGISTRY_PUBLIC_KEY: &str = "RWTq4RZvxZnNQOqFM6x2kbpozFW70qYGXHTnUn2VKLKe5mphpi/F6wbo";

/// Adresse de publication de l'index (ADR 0032) : une release GitHub à étiquette fixe,
/// `registre`, distincte des releases du launcher. Republier = remplacer ses deux fichiers
/// (`index.json` et `index.json.minisig`) ; l'adresse ne change jamais. C'est la signature,
/// pas l'adresse, qui protège l'index.
pub const REGISTRY_INDEX_URL: &str =
    "https://github.com/Yonako25/raftup/releases/download/registre/index.json";

/// Suffixe ajouté à l'adresse de l'index pour trouver sa signature.
pub const SIGNATURE_SUFFIX: &str = ".minisig";

/// Ce qui fait échouer la lecture du registre.
#[derive(Debug, thiserror::Error)]
pub enum RegistryLoadError {
    /// Index, signature ou paquet refusé (voir ADR 0032).
    // `#[from]` : `?` convertit tout seul une `RegistryError` en cette variante.
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// Échec du téléchargement.
    #[error(transparent)]
    Download(#[from] DownloadError),
    /// Adresse refusée (HTTPS, liste blanche).
    #[error("adresse du registre refusée : « {url} » : {reason}")]
    BadUrl { url: String, reason: String },
    /// Apworld demandée absente de l'index.
    #[error("apworld « {0} » absente du registre")]
    UnknownApworld(String),
    /// Un autre fichier porte déjà ce nom dans le dossier des mondes : on ne l'écrase pas
    /// (il peut venir d'un profil de la Bibliothèque, qui garde un reçu de ce fichier).
    #[error("un autre fichier « {} » est déjà installé : Raftup ne l'écrase pas", path.display())]
    OtherApworldPresent { path: PathBuf },
    /// Lecture ou écriture d'un fichier local.
    #[error("{} : {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Ce que [`add_registry_profiles`] a fait de chaque profil de l'index (noms triés).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// Profils ajoutés au catalogue.
    pub added: Vec<String>,
    /// Profils ignorés car le catalogue embarqué a déjà ce jeu.
    pub kept_embedded: Vec<String>,
    /// Profils dont on n'a pas les octets (pas téléchargés).
    pub missing: Vec<String>,
    /// Profils refusés, avec la raison (sha256, taille, profil invalide…).
    pub rejected: Vec<(String, String)>,
}

/// Télécharge `index_url` et `index_url + ".minisig"`, vérifie la signature avec
/// `public_key`, puis lit et valide l'index.
pub fn fetch_verified_index(
    fetcher: &dyn Fetcher,
    index_url: &str,
    public_key: &str,
) -> Result<Index, RegistryLoadError> {
    check_url(index_url)?;
    let signature_url = format!("{index_url}{SIGNATURE_SUFFIX}");
    let index_bytes = fetch_bounded(fetcher, index_url, ::registry::INDEX_MAX_BYTES)?;
    let signature_bytes = fetch_bounded(fetcher, &signature_url, ::registry::SIGNATURE_MAX_BYTES)?;
    // Une signature minisign est du texte ; des octets non UTF-8 sont une signature fausse.
    let signature = String::from_utf8(signature_bytes)
        .map_err(|e| RegistryError::BadSignature(e.to_string()))?;
    Ok(verify_index(&index_bytes, &signature, public_key)?)
}

/// Télécharge un paquet de l'index (au plus `package.size` octets) et contrôle taille et
/// sha256. Rend les octets vérifiés ; rien n'est écrit sur le disque.
pub fn fetch_package(
    fetcher: &dyn Fetcher,
    package: &Package,
) -> Result<Vec<u8>, RegistryLoadError> {
    package.validate()?;
    let data = fetch_bounded(fetcher, &package.url, package.size)?;
    check_package(package, &data)?;
    Ok(data)
}

/// Contrôle les octets d'une apworld du registre. Rend son entrée.
pub fn verify_apworld<'a>(
    index: &'a Index,
    name: &str,
    data: &[u8],
) -> Result<&'a Package, RegistryLoadError> {
    // `'a` : l'entrée rendue est empruntée à `index`, elle vit aussi longtemps que lui.
    let package = index
        .get(PackageKind::Apworld, name)
        .ok_or_else(|| RegistryLoadError::UnknownApworld(name.to_string()))?;
    check_package(package, data)?;
    Ok(package)
}

/// Lit un profil du registre : taille et sha256, UTF-8, mêmes règles que le catalogue
/// embarqué (`Profile::parse`, qui impose aussi `game.id` = nom du paquet).
pub fn registry_profile(package: &Package, data: &[u8]) -> Result<Profile, String> {
    if package.kind != PackageKind::Profile {
        return Err(format!("« {} » n'est pas un profil", package.name));
    }
    check_package(package, data).map_err(|e| e.to_string())?;
    let text = std::str::from_utf8(data).map_err(|_| "profil non UTF-8".to_string())?;
    Profile::parse(&format!("{}.toml", package.name), text).map_err(|e| e.to_string())
}

/// Ajoute au catalogue les profils de l'index dont `files` contient les octets (clé : nom
/// du paquet). Un profil faux est écarté avec sa raison, sans toucher au reste.
pub fn add_registry_profiles(
    catalog: &mut Catalog,
    index: &Index,
    files: &BTreeMap<String, Vec<u8>>,
) -> MergeReport {
    let mut report = MergeReport::default();
    for package in index.of_kind(PackageKind::Profile) {
        let name = package.name.clone();
        if catalog.get(&name).is_some() {
            report.kept_embedded.push(name);
            continue;
        }
        let Some(data) = files.get(&name) else {
            report.missing.push(name);
            continue;
        };
        match registry_profile(package, data) {
            Ok(profile) => {
                catalog.profiles.insert(name.clone(), profile);
                report.added.push(name);
            }
            Err(reason) => report.rejected.push((name, reason)),
        }
    }
    // Ordre stable, quel que soit l'ordre de l'index.
    report.added.sort();
    report.kept_embedded.sort();
    report.missing.sort();
    report.rejected.sort();
    report
}

/// Ce que [`fetch_registry`] a rapporté du réseau, prêt à être fusionné au catalogue.
#[derive(Debug, Clone)]
pub struct RegistryFetch {
    /// Index vérifié (signature puis contenu).
    pub index: Index,
    /// Octets vérifiés (taille + sha256) des profils de jeux absents du catalogue.
    pub files: BTreeMap<String, Vec<u8>>,
    /// Profils dont le téléchargement ou le contrôle a échoué : (nom, raison).
    pub failed: Vec<(String, String)>,
}

/// Lit le registre pour la Bibliothèque : l'index vérifié, puis les profils des jeux que
/// `catalog` n'a pas encore. Les jeux déjà présents ne sont même pas téléchargés.
///
/// Seule une erreur sur l'index fait échouer l'appel ; un profil qui ne se télécharge pas
/// est noté dans `failed` et le reste continue.
pub fn fetch_registry(
    fetcher: &dyn Fetcher,
    index_url: &str,
    public_key: &str,
    catalog: &Catalog,
) -> Result<RegistryFetch, RegistryLoadError> {
    let index = fetch_verified_index(fetcher, index_url, public_key)?;
    let mut files = BTreeMap::new();
    let mut failed = Vec::new();
    for package in index.of_kind(PackageKind::Profile) {
        if catalog.get(&package.name).is_some() {
            continue;
        }
        match fetch_package(fetcher, package) {
            Ok(data) => {
                files.insert(package.name.clone(), data);
            }
            Err(e) => failed.push((package.name.clone(), e.to_string())),
        }
    }
    failed.sort();
    Ok(RegistryFetch {
        index,
        files,
        failed,
    })
}

/// Ajoute au catalogue les profils lus par [`fetch_registry`]. Un échec de téléchargement
/// apparaît dans `rejected` (avec sa raison) plutôt que dans `missing`.
pub fn apply_registry(catalog: &mut Catalog, fetched: &RegistryFetch) -> MergeReport {
    let mut report = add_registry_profiles(catalog, &fetched.index, &fetched.files);
    for (name, reason) in &fetched.failed {
        report.missing.retain(|m| m != name);
        report.rejected.push((name.clone(), reason.clone()));
    }
    report.rejected.sort();
    report
}

/// Résultat de [`install_apworld`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApworldInstall {
    /// Apworld téléchargée, vérifiée et posée à ce chemin.
    Installed(PathBuf),
    /// Le même fichier (même sha256) était déjà là : rien n'a été téléchargé.
    AlreadyThere(PathBuf),
}

/// Chemin où une apworld du registre est posée : `<worlds_dir>/<nom>.apworld`.
///
/// Archipelago importe une apworld sous le nom de son fichier : le nom du paquet est donc
/// celui du dossier du monde dans le zip (`slime_rancher`). Il est sûr comme nom de
/// fichier : l'index n'accepte que `[a-z0-9_-]` (ADR 0032).
pub fn apworld_path(worlds_dir: &Path, name: &str) -> PathBuf {
    worlds_dir.join(format!("{name}.apworld"))
}

/// Télécharge l'apworld `name` de l'index, contrôle taille et sha256, puis l'écrit dans
/// `worlds_dir` (fichier temporaire puis renommage : jamais de fichier à moitié écrit).
///
/// Un fichier identique déjà présent est gardé tel quel ; un fichier différent du même nom
/// n'est **jamais** écrasé (erreur `OtherApworldPresent`).
pub fn install_apworld(
    fetcher: &dyn Fetcher,
    index: &Index,
    name: &str,
    worlds_dir: &Path,
) -> Result<ApworldInstall, RegistryLoadError> {
    let package = index
        .get(PackageKind::Apworld, name)
        .ok_or_else(|| RegistryLoadError::UnknownApworld(name.to_string()))?;
    let target = apworld_path(worlds_dir, name);
    if target.exists() {
        let present = std::fs::read(&target).map_err(io_error(&target))?;
        return if check_package(package, &present).is_ok() {
            Ok(ApworldInstall::AlreadyThere(target))
        } else {
            Err(RegistryLoadError::OtherApworldPresent { path: target })
        };
    }
    let data = fetch_package(fetcher, package)?;
    // Second contrôle, par le nom cette fois : les octets posés sont bien ceux de l'index.
    verify_apworld(index, name, &data)?;
    std::fs::create_dir_all(worlds_dir).map_err(io_error(worlds_dir))?;
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, &data).map_err(io_error(&tmp))?;
    std::fs::rename(&tmp, &target).map_err(io_error(&target))?;
    Ok(ApworldInstall::Installed(target))
}

/// Fabrique la conversion d'une erreur d'entrée-sortie en `RegistryLoadError::Io`.
/// `impl Fn` : on rend une fermeture qui garde sa propre copie du chemin.
fn io_error(path: &Path) -> impl Fn(std::io::Error) -> RegistryLoadError {
    let path = path.to_path_buf();
    move |source| RegistryLoadError::Io {
        path: path.clone(),
        source,
    }
}

/// Lit au plus `max` octets ; au-delà, refus (un serveur ne remplit pas la mémoire).
fn fetch_bounded(fetcher: &dyn Fetcher, url: &str, max: u64) -> Result<Vec<u8>, RegistryLoadError> {
    check_url(url)?;
    let reader = fetcher.open(url)?;
    let mut bytes = Vec::new();
    // `take(max + 1)` : un octet de plus que permis suffit à savoir que c'est trop gros.
    reader
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| DownloadError::Network {
            url: url.to_string(),
            reason: e.to_string(),
        })?;
    if bytes.len() as u64 > max {
        return Err(DownloadError::TooLarge {
            url: url.to_string(),
            max,
        }
        .into());
    }
    Ok(bytes)
}

fn check_url(url: &str) -> Result<(), RegistryLoadError> {
    ::registry::index::check_url(url).map_err(|reason| RegistryLoadError::BadUrl {
        url: url.to_string(),
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meme_liste_blanche_que_les_profils() {
        assert_eq!(::registry::ALLOWED_HOSTS, crate::profile::ALLOWED_HOSTS);
    }

    #[test]
    fn adresse_de_publication_acceptee() {
        // L'adresse figée dans le code doit passer les mêmes règles que toute autre
        // (HTTPS, liste blanche), sinon le launcher ne pourrait jamais lire le registre.
        assert!(check_url(REGISTRY_INDEX_URL).is_ok());
        assert_ne!(REGISTRY_INDEX_URL, crate::update::UPDATE_MANIFEST_URL);
    }

    #[test]
    fn cle_du_registre_distincte_de_celle_des_mises_a_jour() {
        assert_ne!(REGISTRY_PUBLIC_KEY, crate::update::UPDATE_PUBLIC_KEY);
    }
}
