//! Niveau « structure valide » (ADR 0045) : le paquet est bien formé.
//!
//! Pour une apworld : un zip lisible, aux chemins sûrs, qui contient le dossier du monde
//! `<name>/` avec son `__init__.py`. Pour un profil : les règles du catalogue embarqué.
//! Rien n'est extrait ni exécuté : on lit seulement la table des matières du zip.

use std::io::{Cursor, Read};

use launcher_core::registry::registry_profile;
use registry::{Package, PackageKind};

/// Nombre maximal d'entrées dans une apworld.
pub const MAX_ENTRIES: usize = 10_000;
/// Taille décompressée maximale annoncée par le zip (512 Mio).
pub const MAX_UNCOMPRESSED: u64 = 512 * 1024 * 1024;
/// Taille maximale du manifeste `archipelago.json`.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// Vérifie la structure d'un paquet. Rend un court résumé, ou la raison du refus.
pub fn check_structure(package: &Package, data: &[u8]) -> Result<String, String> {
    package.validate().map_err(|e| e.to_string())?;
    match package.kind {
        PackageKind::Apworld => check_apworld(&package.name, package.game.as_deref(), data),
        PackageKind::Profile => {
            let profile = registry_profile(package, data)?;
            Ok(format!("profil lisible (jeu « {} »)", profile.game.name))
        }
    }
}

/// Contrôle la table des matières d'une apworld.
pub fn check_apworld(name: &str, game: Option<&str>, data: &[u8]) -> Result<String, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|e| format!("archive zip illisible : {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("plus de {MAX_ENTRIES} fichiers dans l'archive"));
    }
    let prefix = format!("{name}/");
    let mut total: u64 = 0;
    let mut has_init = false;
    let mut manifest = None;
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| format!("entrée n° {i} illisible : {e}"))?;
        let raw = entry
            .name()
            .map_err(|e| format!("nom de l'entrée n° {i} illisible : {e}"))?
            .to_string();
        let path = raw.replace('\\', "/");
        check_entry_path(&path)?;
        // `saturating_add` : jamais de dépassement d'entier, même avec des tailles fausses.
        total = total.saturating_add(entry.size());
        if total > MAX_UNCOMPRESSED {
            return Err(format!(
                "plus de {MAX_UNCOMPRESSED} octets une fois décompressée"
            ));
        }
        if path == "archipelago.json" || path == format!("{prefix}archipelago.json") {
            manifest = Some(i);
        } else if !path.starts_with(&prefix) {
            return Err(format!(
                "« {path} » est hors du dossier du monde « {prefix} »"
            ));
        }
        if path == format!("{prefix}__init__.py") {
            has_init = true;
        }
    }
    if !has_init {
        return Err(format!("« {prefix}__init__.py » manquant"));
    }
    let mut summary = format!("{} fichier(s), {total} octets décompressés", zip.len());
    if let Some(index) = manifest {
        let manifest_game = read_manifest(&mut zip, index)?;
        if let (Some(found), Some(expected)) = (manifest_game.as_deref(), game) {
            if found != expected {
                return Err(format!(
                    "archipelago.json annonce le jeu « {found} », le registre « {expected} »"
                ));
            }
        }
        summary.push_str(", archipelago.json lisible");
    }
    Ok(summary)
}

/// Un chemin d'entrée sûr : relatif, sans `..`, sans lecteur ni caractère de contrôle.
fn check_entry_path(path: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("chemin « {path} » refusé : {why}"));
    if path.is_empty() || path.len() > 260 {
        return bad("vide ou trop long");
    }
    if path.starts_with('/') || path.contains(':') {
        return bad("chemin absolu");
    }
    if path.chars().any(char::is_control) {
        return bad("caractère de contrôle");
    }
    if path.split('/').any(|part| part == "..") {
        return bad("remonte d'un dossier (« .. »)");
    }
    Ok(())
}

/// Lit `archipelago.json` (objet JSON borné) et rend son champ `game` s'il existe.
fn read_manifest(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    index: usize,
) -> Result<Option<String>, String> {
    let entry = zip
        .by_index(index)
        .map_err(|e| format!("archipelago.json illisible : {e}"))?;
    let mut text = String::new();
    entry
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("archipelago.json illisible : {e}"))?;
    if text.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("archipelago.json trop gros".to_string());
    }
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("archipelago.json invalide : {e}"))?;
    if !json.is_object() {
        return Err("archipelago.json n'est pas un objet JSON".to_string());
    }
    Ok(json
        .get("game")
        .and_then(|g| g.as_str())
        .map(str::to_string))
}
