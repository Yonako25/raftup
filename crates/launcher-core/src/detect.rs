//! Détection des jeux installés : Steam, Epic Games et dossier choisi à la main.
//!
//! Le module ne connaît pas les profils : il reçoit des `DetectRule` (une par jeu du
//! catalogue) et des `DetectionRoots` (où chercher). Les tests peuvent donc utiliser
//! de faux dossiers, sans Steam ni Epic installés.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use keyvalues_parser::{Obj, Value};
use serde::Deserialize;

/// Taille maximale lue pour un manifeste : un fichier absurde ne doit pas remplir la mémoire.
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

// Bits de `StateFlags` des manifestes Steam (valeurs publiques connues, à revérifier
// sur une vraie machine) : 4 = installé, 2/256/1024 = mise à jour en attente ou en cours,
// 64 = jeu lancé.
const FLAG_INSTALLED: u32 = 4;
const FLAG_UPDATING: u32 = 2 | 256 | 1024;
const FLAG_RUNNING: u32 = 64;

/// Ce qui empêche d'accepter un dossier de jeu choisi à la main.
#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    /// Le chemin n'est pas un dossier existant.
    #[error("{0} n'est pas un dossier")]
    NotADirectory(PathBuf),
    /// Le profil du jeu ne donne pas d'exécutable à chercher.
    #[error("aucun exécutable attendu n'est défini pour le jeu {game_id}")]
    NoExpectedExe { game_id: String },
    /// Le nom d'exécutable du profil sort du dossier (`..`, chemin absolu…).
    #[error("exécutable attendu invalide : {0}")]
    UnsafeExe(String),
    /// L'exécutable attendu n'est pas dans le dossier.
    #[error("{exe} est introuvable dans {dir} : ce n'est pas le bon dossier ?")]
    ExeMissing { dir: PathBuf, exe: String },
}

/// Comment reconnaître un jeu (extrait du profil, mais sans dépendre de lui).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectRule {
    pub game_id: String,
    pub steam_appid: Option<u32>,
    pub epic_app_name: Option<String>,
    pub expected_exe: Option<String>,
}

/// Où le jeu a été trouvé.
// `Ord` dérivé : l'ordre des variantes (Steam < Epic < Manual) sert au tri des résultats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Store {
    Steam,
    Epic,
    Manual,
}

/// L'état du jeu d'après son lanceur : seul `Ready` autorise d'installer un mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GameState {
    /// Installé et au repos.
    Ready,
    /// Installation ou mise à jour en cours (ou en attente).
    Updating,
    /// Le jeu est lancé.
    Running,
}

/// Un jeu trouvé sur la machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedGame {
    pub game_id: String,
    pub store: Store,
    pub install_dir: PathBuf,
    /// Version de build donnée par le lanceur (sert à repérer une mise à jour du jeu).
    pub build_id: Option<String>,
    pub state: GameState,
}

/// Les dossiers où chercher. `None` = ce lanceur n'est pas installé.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetectionRoots {
    /// Dossier d'installation de Steam (celui qui contient `steamapps`).
    pub steam_dir: Option<PathBuf>,
    /// Dossier des manifestes Epic (`*.item`).
    pub epic_manifests_dir: Option<PathBuf>,
}

impl DetectionRoots {
    /// Les dossiers du vrai système (registre Windows, `%PROGRAMDATA%`).
    pub fn from_system() -> DetectionRoots {
        let epic = std::env::var_os("PROGRAMDATA")
            .map(|p| {
                PathBuf::from(p)
                    .join("Epic")
                    .join("EpicGamesLauncher")
                    .join("Data")
                    .join("Manifests")
            })
            .filter(|p| p.is_dir());
        DetectionRoots {
            steam_dir: steam_dir_from_registry(),
            epic_manifests_dir: epic,
        }
    }
}

/// Trouve Steam par le registre. Seule fonction qui touche à `winreg`.
// `#[cfg(windows)]` : ce code n'est compilé que sous Windows ; ailleurs, la version
// du dessous renvoie simplement `None`.
#[cfg(windows)]
fn steam_dir_from_registry() -> Option<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    // Clés citées de mémoire (non vérifiées sur une machine) : on essaie les deux,
    // puis l'emplacement par défaut.
    let machine = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\WOW6432Node\\Valve\\Steam")
        .and_then(|k| k.get_value::<String, _>("InstallPath"))
        .ok();
    let user = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Valve\\Steam")
        .and_then(|k| k.get_value::<String, _>("SteamPath"))
        .ok();
    machine
        .into_iter()
        .chain(user)
        .map(PathBuf::from)
        .chain(std::iter::once(PathBuf::from(
            "C:\\Program Files (x86)\\Steam",
        )))
        .find(|p| p.join("steamapps").is_dir())
}

#[cfg(not(windows))]
fn steam_dir_from_registry() -> Option<PathBuf> {
    None
}

/// Cherche tous les jeux connus des `rules` dans `roots`. Résultat trié par
/// `game_id` puis par lanceur. Une erreur de lecture n'arrête jamais la détection :
/// le jeu concerné est simplement absent du résultat.
pub fn detect_games(roots: &DetectionRoots, rules: &[DetectRule]) -> Vec<DetectedGame> {
    let mut found = Vec::new();
    if let Some(steam) = &roots.steam_dir {
        detect_steam(steam, rules, &mut found);
    }
    if let Some(epic) = &roots.epic_manifests_dir {
        detect_epic(epic, rules, &mut found);
    }
    found.sort_by(|a, b| {
        (&a.game_id, a.store, &a.install_dir).cmp(&(&b.game_id, b.store, &b.install_dir))
    });
    found
}

/// Accepte un dossier choisi par le joueur si l'exécutable attendu s'y trouve.
/// (Un hash de l'exécutable ne serait qu'un avertissement : non vérifié ici.)
pub fn validate_manual_dir(rule: &DetectRule, path: &Path) -> Result<DetectedGame, DetectError> {
    if !path.is_dir() {
        return Err(DetectError::NotADirectory(path.to_path_buf()));
    }
    let exe = rule
        .expected_exe
        .as_deref()
        .ok_or_else(|| DetectError::NoExpectedExe {
            game_id: rule.game_id.clone(),
        })?;
    if !is_safe_relative(exe) {
        return Err(DetectError::UnsafeExe(exe.to_string()));
    }
    if !path.join(exe).is_file() {
        return Err(DetectError::ExeMissing {
            dir: path.to_path_buf(),
            exe: exe.to_string(),
        });
    }
    Ok(DetectedGame {
        game_id: rule.game_id.clone(),
        store: Store::Manual,
        install_dir: path.to_path_buf(),
        build_id: None,
        state: GameState::Ready,
    })
}

/// Vrai si `text` est un chemin relatif sans `..`, sans racine ni lettre de disque.
fn is_safe_relative(text: &str) -> bool {
    let path = Path::new(text);
    !text.is_empty()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Lit un petit fichier texte ; `None` si absent, illisible ou trop gros.
fn read_small_text(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    // `take` plafonne la lecture ; on lit un octet de plus pour détecter un fichier trop gros.
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() as u64 > MAX_MANIFEST_BYTES {
        return None;
    }
    Some(text)
}

// ----------------------------------------------------------------------------
// Steam
// ----------------------------------------------------------------------------

/// Valeur texte d'une clé d'un objet VDF (recherche insensible à la casse, car Steam
/// n'est pas constant sur les majuscules).
fn vdf_str<'a>(obj: &'a Obj<'_>, key: &str) -> Option<&'a str> {
    obj.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .and_then(|(_, values)| values.first())
        .and_then(Value::get_str)
}

/// Les bibliothèques Steam : le dossier de Steam lui-même, puis celles de `libraryfolders.vdf`.
fn steam_libraries(steam_dir: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam_dir.to_path_buf()];
    let Some(text) = read_small_text(&steam_dir.join("steamapps").join("libraryfolders.vdf"))
    else {
        return libs;
    };
    let Ok(parsed) = keyvalues_parser::parse(&text) else {
        return libs;
    };
    let vdf = parsed.into_vdf();
    let Some(root) = vdf.value.get_obj() else {
        return libs;
    };
    for values in root.values() {
        for value in values {
            // Format récent : `"0" { "path" "D:\\SteamLibrary" … }`.
            // Ancien format : `"1" "D:\\SteamLibrary"`.
            let path = match value {
                Value::Obj(entry) => vdf_str(entry, "path"),
                Value::Str(s) => Some(s.as_ref()),
            };
            if let Some(path) = path {
                let path = PathBuf::from(path);
                if !libs.contains(&path) {
                    libs.push(path);
                }
            }
        }
    }
    libs
}

fn detect_steam(steam_dir: &Path, rules: &[DetectRule], found: &mut Vec<DetectedGame>) {
    let libraries = steam_libraries(steam_dir);
    for rule in rules {
        let Some(appid) = rule.steam_appid else {
            continue;
        };
        // Une bibliothèque sur un disque absent : le manifeste est introuvable, on passe.
        for lib in &libraries {
            if let Some(game) = steam_game_in_library(lib, appid, &rule.game_id) {
                found.push(game);
            }
        }
    }
}

/// Lit `appmanifest_<appid>.acf` d'une bibliothèque ; `None` au moindre problème.
fn steam_game_in_library(library: &Path, appid: u32, game_id: &str) -> Option<DetectedGame> {
    let steamapps = library.join("steamapps");
    let text = read_small_text(&steamapps.join(format!("appmanifest_{appid}.acf")))?;
    let parsed = keyvalues_parser::parse(&text).ok()?;
    let vdf = parsed.into_vdf();
    let state = vdf.value.get_obj()?;

    let installdir = vdf_str(state, "installdir")?;
    // `installdir` doit rester un simple nom relatif : un manifeste ne doit pas pouvoir
    // nous faire pointer ailleurs que dans `steamapps/common`.
    if !is_safe_relative(installdir) {
        return None;
    }
    let install_dir = steamapps.join("common").join(installdir);
    // Manifeste obsolète : le dossier a pu être supprimé à la main.
    if !install_dir.is_dir() {
        return None;
    }

    let flags: u32 = vdf_str(state, "StateFlags")?.trim().parse().ok()?;
    // `&` teste un bit précis ; comparer avec `==` raterait les combinaisons (ex. 6 = 4 + 2).
    let game_state = if flags & FLAG_UPDATING != 0 {
        GameState::Updating
    } else if flags & FLAG_INSTALLED == 0 {
        // Ni installé ni en cours d'installation : le jeu n'est pas là.
        return None;
    } else if flags & FLAG_RUNNING != 0 {
        GameState::Running
    } else {
        GameState::Ready
    };

    Some(DetectedGame {
        game_id: game_id.to_string(),
        store: Store::Steam,
        install_dir,
        build_id: vdf_str(state, "buildid").map(str::to_string),
        state: game_state,
    })
}

// ----------------------------------------------------------------------------
// Epic
// ----------------------------------------------------------------------------

/// Un manifeste Epic. Aucun schéma officiel : tous les champs sont optionnels et les
/// champs inconnus sont ignorés (serde ignore par défaut ce qu'il ne connaît pas).
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct EpicManifest {
    install_location: Option<String>,
    app_name: Option<String>,
    app_version_string: Option<String>,
    // Nom cité de mémoire (à vérifier) : sert seulement à signaler une installation inachevée.
    #[serde(rename = "bIsIncompleteInstall")]
    is_incomplete_install: Option<bool>,
}

fn detect_epic(manifests_dir: &Path, rules: &[DetectRule], found: &mut Vec<DetectedGame>) {
    let Ok(entries) = std::fs::read_dir(manifests_dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("item"))
        })
        .collect();
    // `read_dir` ne garantit aucun ordre : on trie pour un résultat reproductible.
    files.sort();

    for file in files {
        let Some(text) = read_small_text(&file) else {
            continue;
        };
        // Manifeste cassé : ignoré, les autres continuent.
        let Ok(manifest) = serde_json::from_str::<EpicManifest>(&text) else {
            continue;
        };
        let (Some(app_name), Some(location)) = (&manifest.app_name, &manifest.install_location)
        else {
            continue;
        };
        let Some(rule) = rules
            .iter()
            .find(|r| r.epic_app_name.as_deref() == Some(app_name.as_str()))
        else {
            continue;
        };
        let install_dir = PathBuf::from(location);
        if !install_dir.is_dir() {
            continue;
        }
        found.push(DetectedGame {
            game_id: rule.game_id.clone(),
            store: Store::Epic,
            install_dir,
            build_id: manifest.app_version_string.clone(),
            state: if manifest.is_incomplete_install == Some(true) {
                GameState::Updating
            } else {
                GameState::Ready
            },
        });
    }
}
