//! Le fichier d'épinglage `generator-pins.toml` : ce que le launcher a le droit de télécharger
//! pour faire tourner le générateur (ADR 0017).
//!
//! Le fichier est embarqué dans le binaire (`include_str!`) puis validé au chargement :
//! une valeur fausse est arrêtée par les tests de la CI, jamais chez le joueur.
//! Rien n'est redistribué : on ne garde ici que des adresses, des tailles et des hash.

use std::collections::BTreeSet;

use serde::Deserialize;

use super::GenerationError;
use crate::hash::Sha256;

/// Version du format du fichier d'épinglage que ce launcher sait lire.
pub const FORMAT_VERSION: u32 = 1;

/// Domaines autorisés pour ces téléchargements. Liste locale : on ne touche pas à
/// `profile::ALLOWED_HOSTS`, qui concerne les mods des profils.
const ALLOWED_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "files.pythonhosted.org",
];

/// Le fichier réel, embarqué à la compilation. Le chemin part du dossier de ce fichier source.
const PINS_TOML: &str = include_str!("../../generator-pins.toml");

/// Tout l'épinglage. `deny_unknown_fields` : un champ inconnu est une erreur, pas ignoré.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorPins {
    pub format_version: u32,
    pub python: PinnedArchive,
    pub archipelago: PinnedArchive,
    pub wheels: Vec<PinnedWheel>,
}

impl GeneratorPins {
    /// Total des tailles réelles à télécharger (Python, Archipelago, wheels), en octets :
    /// ce qu'on montre au joueur avant de lui demander son accord.
    pub fn download_bytes(&self) -> u64 {
        let wheels: u64 = self.wheels.iter().map(|w| w.size).sum();
        self.python
            .size
            .saturating_add(self.archipelago.size)
            .saturating_add(wheels)
    }
}

/// Une archive (`.tar.gz` ou `.zip`) à télécharger puis extraire.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedArchive {
    pub version: String,
    pub url: String,
    pub sha256: Sha256,
    /// Taille réelle du fichier, en octets (relevée sur le téléchargement).
    pub size: u64,
    /// Taille maximale du téléchargement, en octets (plafond, au moins `size`).
    pub max_bytes: u64,
    /// Dossier racine de l'archive, retiré à l'extraction.
    pub strip_prefix: String,
    /// Nombre maximal d'entrées (protège d'une archive piégée).
    pub max_entries: usize,
}

/// Une wheel Python, décompressée directement dans `site-packages`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedWheel {
    pub name: String,
    pub version: String,
    pub filename: String,
    pub url: String,
    pub sha256: Sha256,
    /// Taille réelle du fichier, en octets.
    pub size: u64,
    pub max_bytes: u64,
}

/// Les valeurs épinglées du launcher (fichier embarqué), validées.
pub fn pins() -> Result<GeneratorPins, GenerationError> {
    parse_pins(PINS_TOML)
}

/// Lit et valide un texte TOML d'épinglage. Publique pour que les tests fabriquent
/// des fichiers faux et vérifient qu'ils sont refusés.
pub fn parse_pins(text: &str) -> Result<GeneratorPins, GenerationError> {
    let pins: GeneratorPins =
        toml::from_str(text).map_err(|e| GenerationError::Pins(e.to_string()))?;
    validate(&pins).map_err(GenerationError::Pins)?;
    Ok(pins)
}

/// Règles de validation. L'erreur est un simple texte, que l'appelant enveloppe.
fn validate(pins: &GeneratorPins) -> Result<(), String> {
    if pins.format_version != FORMAT_VERSION {
        return Err(format!(
            "format_version {} non pris en charge (attendu {FORMAT_VERSION})",
            pins.format_version
        ));
    }
    check_archive("python", &pins.python)?;
    check_archive("archipelago", &pins.archipelago)?;

    if pins.wheels.is_empty() {
        return Err("aucune wheel épinglée".to_string());
    }
    // Un `BTreeSet` pour repérer les doublons, avec un ordre qui ne dépend de rien.
    let mut seen = BTreeSet::new();
    for wheel in &pins.wheels {
        check_wheel(wheel)?;
        if !seen.insert(normalize_name(&wheel.name)) {
            return Err(format!("wheel « {} » en double", wheel.name));
        }
    }
    Ok(())
}

fn check_archive(label: &str, archive: &PinnedArchive) -> Result<(), String> {
    if archive.version.trim().is_empty() {
        return Err(format!("[{label}] : version vide"));
    }
    check_url(&archive.url).map_err(|e| format!("[{label}] : {e}"))?;
    if archive.max_bytes == 0 || archive.max_entries == 0 {
        return Err(format!(
            "[{label}] : max_bytes et max_entries doivent être > 0"
        ));
    }
    check_size(label, archive.size, archive.max_bytes)?;
    // Le préfixe est retiré des chemins de l'archive : il ne doit pas pouvoir en sortir.
    let prefix = &archive.strip_prefix;
    if prefix.is_empty()
        || prefix.starts_with('/')
        || prefix.contains(['\\', ':', '\0'])
        || prefix.split('/').any(|part| part == "..")
    {
        return Err(format!("[{label}] : strip_prefix « {prefix} » invalide"));
    }
    Ok(())
}

fn check_wheel(wheel: &PinnedWheel) -> Result<(), String> {
    let name = &wheel.name;
    if name.trim().is_empty() || wheel.version.trim().is_empty() {
        return Err("wheel sans nom ou sans version".to_string());
    }
    check_url(&wheel.url).map_err(|e| format!("wheel {name} : {e}"))?;
    check_size(&format!("wheel {name}"), wheel.size, wheel.max_bytes)?;
    let file = &wheel.filename;
    if file.is_empty() || file.contains(['/', '\\', ':', '\0']) || file.contains("..") {
        return Err(format!("wheel {name} : filename « {file} » dangereux"));
    }
    check_wheel_tags(file).map_err(|e| format!("wheel {name} : {e}"))?;
    // L'URL doit bien désigner ce fichier : évite un filename et une URL qui divergent.
    let expected_tail = format!("/{file}");
    let path = wheel.url.split(['?', '#']).next().unwrap_or("");
    if !path.ends_with(&expected_tail) {
        return Err(format!("wheel {name} : l'URL ne se termine pas par {file}"));
    }
    Ok(())
}

/// `0 < size <= max_bytes` : la taille réelle doit tenir sous le plafond.
fn check_size(label: &str, size: u64, max_bytes: u64) -> Result<(), String> {
    if size == 0 || size > max_bytes {
        return Err(format!(
            "{label} : size ({size}) doit être > 0 et <= max_bytes ({max_bytes})"
        ));
    }
    Ok(())
}

/// Normalisation PEP 503 simplifiée : minuscules, `-`, `_` et `.` équivalents.
fn normalize_name(name: &str) -> String {
    name.to_ascii_lowercase().replace(['-', '.'], "_")
}

/// HTTPS obligatoire, hôte dans la liste blanche, sans identifiants ni port.
fn check_url(url: &str) -> Result<(), String> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(format!("« {url} » : HTTPS obligatoire"));
    };
    if url.contains(['{', '}', ' ', '\0', '\n', '\r', '\\']) {
        return Err(format!("« {url} » : caractère interdit"));
    }
    // L'hôte s'arrête au premier « / », « ? » ou « # ». Un « @ » ou un « : » servirait
    // à tromper la liste blanche (identifiants, port) : refusés.
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.contains(['@', ':']) {
        return Err(format!("« {url} » : hôte invalide"));
    }
    if !ALLOWED_HOSTS.contains(&host.to_ascii_lowercase().as_str()) {
        return Err(format!(
            "« {url} » : domaine « {host} » hors de la liste blanche"
        ));
    }
    Ok(())
}

/// Accepte seulement les wheels utilisables avec CPython 3.12 sur Windows x86_64 :
/// `py3-none-any` (Python pur), `cp312-cp312-win_amd64` ou `cp3N-abi3-win_amd64` (N <= 12).
/// Nom de wheel (PEP 427) : `nom-version[-build]-python-abi-plateforme.whl`.
fn check_wheel_tags(filename: &str) -> Result<(), String> {
    let stem = filename
        .strip_suffix(".whl")
        .ok_or_else(|| format!("« {filename} » n'est pas une wheel (.whl)"))?;
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() != 5 && parts.len() != 6 {
        return Err(format!("« {filename} » : nom de wheel mal formé"));
    }
    let (py, abi, platform) = (
        parts[parts.len() - 3],
        parts[parts.len() - 2],
        parts[parts.len() - 1],
    );
    let py_tags: Vec<&str> = py.split('.').collect();
    let ok = match platform {
        // Python pur : « py3 » (ou « py2.py3 ») sans ABI.
        "any" => abi == "none" && py_tags.contains(&"py3"),
        "win_amd64" => {
            py_tags.contains(&"cp312") && (abi == "cp312" || abi == "abi3")
                || abi == "abi3" && py_tags.iter().any(|t| is_cpython3_up_to_12(t))
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "« {filename} » : incompatible avec CPython 3.12 Windows x86_64"
        ))
    }
}

/// `cp3N` avec N entre 2 et 12 (une wheel `abi3` de ces versions tourne en 3.12).
fn is_cpython3_up_to_12(tag: &str) -> bool {
    tag.strip_prefix("cp3")
        .and_then(|n| n.parse::<u32>().ok())
        .is_some_and(|n| (2..=12).contains(&n))
}
