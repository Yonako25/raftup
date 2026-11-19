//! Lecture et vérification du registre de paquets signés (V2, ADR 0032).
//!
//! Le registre complète le catalogue embarqué (ADR 0001) sans le remplacer :
//! 1. [`fetch_verified_index`] télécharge `index.json` et sa signature, et vérifie la
//!    signature minisign **avant** de lire le JSON ;
//! 2. [`fetch_package`] télécharge un paquet (borné à sa taille) et contrôle son sha256 ;
//! 3. [`add_registry_profiles`] ajoute au catalogue les profils du registre pour des jeux
//!    absents du catalogue embarqué : **le catalogue embarqué gagne toujours**.
//!
//! Rien n'est installé ni exécuté ici.
//!
//! Note : la crate s'appelle aussi `registry`. On écrit `::registry::…` (chemin qui part de
//! la liste des crates) pour ne pas la confondre avec ce module, `crate::registry`.

use std::collections::BTreeMap;
use std::io::Read;

use ::registry::{check_package, verify_index, Index, Package, PackageKind, RegistryError};

use crate::catalog::Catalog;
use crate::download::{DownloadError, Fetcher};
use crate::profile::Profile;

/// Clé publique minisign du registre (ligne base64 « RW… »), **distincte** de celle des
/// mises à jour (ADR 0032). Vide tant que la mainteneuse n'a pas lancé
/// `raftup-registry keygen` : toute vérification échoue alors (`PublicKeyNotConfigured`).
pub const REGISTRY_PUBLIC_KEY: &str = "";

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
    let package = index.get(PackageKind::Apworld, name).ok_or_else(|| {
        RegistryError::InvalidIndex(format!("apworld « {name} » absente du registre"))
    })?;
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
    fn cle_du_registre_distincte_de_celle_des_mises_a_jour() {
        assert_ne!(REGISTRY_PUBLIC_KEY, crate::update::UPDATE_PUBLIC_KEY);
    }
}
