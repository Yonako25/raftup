//! Format de l'index du registre (`index.json`, ADR 0032).
//!
//! L'index liste des **références** : adresse chez l'auteur, sha256, taille, licence.
//! Il ne contient jamais le paquet lui-même.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::RegistryError;

/// Version du format d'index que ce programme sait lire et écrire.
pub const FORMAT_VERSION: u32 = 1;

/// Nombre maximal de paquets dans un index.
pub const MAX_PACKAGES: usize = 10_000;

/// Taille maximale d'un paquet référencé (512 Mio, comme les téléchargements de l'ADR 0015).
pub const PACKAGE_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Domaines permis pour l'adresse d'un paquet : copie de `launcher_core::profile::ALLOWED_HOSTS`
/// (ADR 0015). La crate ne peut pas dépendre de `launcher-core` (c'est l'inverse) : un test
/// de `launcher-core` vérifie que les deux listes sont égales.
pub const ALLOWED_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "thunderstore.io",
    "gcdn.thunderstore.io",
];

/// Sorte de paquet. Rien d'autre n'est accepté : jamais `rom`, `asset` ni jeu.
///
/// `Ord` : l'ordre de déclaration sert au tri de l'index (apworlds puis profils).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageKind {
    /// Un monde Archipelago (`.apworld`), installé dans le dossier des mondes.
    Apworld,
    /// Un profil d'installation (`<game.id>.toml`, ADR 0015).
    Profile,
}

impl PackageKind {
    /// Nom écrit dans l'index.
    pub fn as_str(self) -> &'static str {
        match self {
            PackageKind::Apworld => "apworld",
            PackageKind::Profile => "profile",
        }
    }
}

/// Un paquet référencé par l'index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub kind: PackageKind,
    pub name: String,
    pub version: String,
    /// Nom du jeu Archipelago, facultatif. `skip_serializing_if` : absent du JSON si `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub license: String,
    pub authors: Vec<String>,
}

/// L'index complet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub format_version: u32,
    /// Nom du registre (par exemple « raftup »).
    pub registry: String,
    pub packages: Vec<Package>,
}

/// Première lecture : seulement `format_version`, sans refuser les autres champs.
/// Un index d'un format plus récent peut avoir des champs que ce programme ne connaît pas.
#[derive(Deserialize)]
struct Header {
    format_version: u32,
}

impl Index {
    /// Index vide d'un registre.
    pub fn new(registry: &str) -> Index {
        Index {
            format_version: FORMAT_VERSION,
            registry: registry.to_string(),
            packages: Vec::new(),
        }
    }

    /// Lit et valide un index JSON. Ne vérifie **pas** la signature : voir
    /// [`crate::verify_index`], qui le fait avant d'appeler cette fonction.
    pub fn parse(bytes: &[u8]) -> Result<Index, RegistryError> {
        let header: Header =
            serde_json::from_slice(bytes).map_err(|e| RegistryError::Parse(e.to_string()))?;
        if header.format_version != FORMAT_VERSION {
            return Err(RegistryError::UnsupportedFormat {
                found: header.format_version,
                supported: FORMAT_VERSION,
            });
        }
        let index: Index =
            serde_json::from_slice(bytes).map_err(|e| RegistryError::Parse(e.to_string()))?;
        index.validate()?;
        Ok(index)
    }

    /// Écrit l'index en JSON, paquets triés par (`kind`, `name`) : le même contenu donne
    /// toujours les mêmes octets. Valide l'index avant.
    pub fn to_bytes(&self) -> Result<Vec<u8>, RegistryError> {
        let mut sorted = self.clone();
        sorted.sort();
        sorted.validate()?;
        let mut bytes = serde_json::to_vec_pretty(&sorted)
            .map_err(|e| RegistryError::InvalidIndex(e.to_string()))?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Range les paquets par (`kind`, `name`).
    pub fn sort(&mut self) {
        // `sort_by` avec une clé faite de références : pas de copie des chaînes.
        self.packages
            .sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
    }

    /// Vérifie toutes les règles de l'ADR 0032.
    pub fn validate(&self) -> Result<(), RegistryError> {
        if self.format_version != FORMAT_VERSION {
            return Err(RegistryError::UnsupportedFormat {
                found: self.format_version,
                supported: FORMAT_VERSION,
            });
        }
        if let Err(reason) = check_name(&self.registry) {
            return Err(RegistryError::InvalidIndex(format!(
                "nom du registre : {reason}"
            )));
        }
        if self.packages.len() > MAX_PACKAGES {
            return Err(RegistryError::InvalidIndex(format!(
                "plus de {MAX_PACKAGES} paquets"
            )));
        }
        // `BTreeSet` de couples empruntés : détecte deux paquets de même sorte et même nom.
        let mut seen: BTreeSet<(PackageKind, &str)> = BTreeSet::new();
        for package in &self.packages {
            package.validate()?;
            if !seen.insert((package.kind, package.name.as_str())) {
                return Err(RegistryError::InvalidIndex(format!(
                    "{} « {} » présent deux fois (une seule version par paquet)",
                    package.kind.as_str(),
                    package.name
                )));
            }
        }
        Ok(())
    }

    /// Le paquet d'une sorte et d'un nom donnés.
    pub fn get(&self, kind: PackageKind, name: &str) -> Option<&Package> {
        self.packages
            .iter()
            .find(|p| p.kind == kind && p.name == name)
    }

    /// Les paquets d'une sorte, dans l'ordre de l'index.
    pub fn of_kind(&self, kind: PackageKind) -> impl Iterator<Item = &Package> {
        // `move` : la fermeture garde sa propre copie de `kind` (un simple `Copy`).
        self.packages.iter().filter(move |p| p.kind == kind)
    }
}

impl Package {
    /// Vérifie les champs d'un paquet.
    pub fn validate(&self) -> Result<(), RegistryError> {
        let bad = |reason: String| RegistryError::InvalidPackage {
            package: format!("{} {}", self.kind.as_str(), self.name),
            reason,
        };
        check_name(&self.name).map_err(|r| bad(format!("nom : {r}")))?;
        check_version(&self.version).map_err(|r| bad(format!("version : {r}")))?;
        if let Some(game) = &self.game {
            check_text(game, 128).map_err(|r| bad(format!("jeu : {r}")))?;
        }
        check_url(&self.url).map_err(|r| bad(format!("adresse « {} » : {r}", self.url)))?;
        check_sha256(&self.sha256).map_err(|r| bad(format!("sha256 : {r}")))?;
        if self.size == 0 || self.size > PACKAGE_MAX_BYTES {
            return Err(bad(format!(
                "taille {} hors de 1 à {PACKAGE_MAX_BYTES} octets",
                self.size
            )));
        }
        check_text(&self.license, 64).map_err(|r| bad(format!("licence : {r}")))?;
        if self.authors.is_empty() {
            return Err(bad("au moins un auteur est obligatoire".to_string()));
        }
        for author in &self.authors {
            check_text(author, 100).map_err(|r| bad(format!("auteur : {r}")))?;
        }
        Ok(())
    }
}

/// `[a-z0-9_-]`, 1 à 64 caractères, commence par une lettre ou un chiffre.
fn check_name(name: &str) -> Result<(), String> {
    let first_ok = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let all_ok = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if name.len() > 64 || !first_ok || !all_ok {
        return Err(format!(
            "« {name} » : 1 à 64 caractères parmi a-z, 0-9, _ et -, en commençant par une lettre ou un chiffre"
        ));
    }
    Ok(())
}

/// `[0-9A-Za-z.+-]`, 1 à 32 caractères, commence par un chiffre ou une lettre.
fn check_version(version: &str) -> Result<(), String> {
    let first_ok = version
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    let all_ok = version
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'));
    if version.len() > 32 || !first_ok || !all_ok {
        return Err(format!(
            "« {version} » : 1 à 32 caractères parmi 0-9, A-Z, a-z, « . », « + » et « - »"
        ));
    }
    Ok(())
}

/// Texte court, non vide, sans caractère de contrôle (affiché tel quel dans le launcher).
fn check_text(text: &str, max_chars: usize) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("vide".to_string());
    }
    if text.chars().count() > max_chars {
        return Err(format!("plus de {max_chars} caractères"));
    }
    if text.chars().any(char::is_control) {
        return Err("caractère de contrôle".to_string());
    }
    Ok(())
}

/// 64 caractères hexadécimaux minuscules.
fn check_sha256(hex: &str) -> Result<(), String> {
    let ok = hex.len() == 64
        && hex
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    if !ok {
        return Err(format!(
            "« {hex} » : 64 caractères hexadécimaux minuscules attendus"
        ));
    }
    Ok(())
}

/// HTTPS, hôte de la liste blanche, ni identifiants ni port (mêmes règles que l'ADR 0022).
pub fn check_url(url: &str) -> Result<(), String> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err("HTTPS obligatoire".to_string());
    };
    if url.len() > 2048 {
        return Err("adresse trop longue".to_string());
    }
    if url.chars().any(|c| c.is_control() || c == ' ' || c == '\\') {
        return Err("caractère interdit".to_string());
    }
    // L'hôte s'arrête au premier « / », « ? » ou « # ». Un « @ » (identifiants) ou un
    // « : » (port) servirait à tromper la liste blanche : refusés.
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.contains(['@', ':']) {
        return Err("hôte invalide".to_string());
    }
    if !ALLOWED_HOSTS.contains(&host.to_ascii_lowercase().as_str()) {
        return Err(format!("domaine « {host} » hors de la liste blanche"));
    }
    // Il faut un chemin vers un fichier, pas seulement le domaine.
    let path = &rest[host.len()..];
    if path.len() <= 1 {
        return Err("chemin du fichier manquant".to_string());
    }
    Ok(())
}
