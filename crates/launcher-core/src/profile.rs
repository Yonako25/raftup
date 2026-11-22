//! Le profil d'installation TOML : types, validation et `format_version`.
//!
//! Un profil décrit comment installer les mods d'un jeu (ADR 0015). Il est lu avec
//! `Profile::parse`, qui désérialise PUIS valide : un profil faux est refusé au chargement
//! (et par la CI, via le test du catalogue), jamais chez le joueur.
//!
//! Sécurité : le hash ne protège pas d'un profil malveillant fusionné. Les garde-fous ici
//! sont donc la liste blanche de domaines, HTTPS obligatoire, et des cibles d'écriture
//! limitées à `{game_dir}` et `{worlds_dir}` (jamais de `..`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::Deserialize;

use crate::hash::Sha256;
use crate::template::{self, TemplateError};

/// Version du format de profil que ce launcher sait lire.
pub const FORMAT_VERSION: u32 = 1;

/// Domaines autorisés pour les téléchargements (ADR 0015, décision 4).
pub const ALLOWED_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "thunderstore.io",
    "gcdn.thunderstore.io",
];

/// Taille maximale par défaut d'un téléchargement si l'étape n'en précise pas (512 Mio).
pub const DEFAULT_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Variables permises dans les étapes, `preserve` et `cleanup`.
const STEP_VARS: &[&str] = &["game_dir", "worlds_dir"];
/// Variables permises au lancement (celles des étapes, plus la connexion).
const LAUNCH_VARS: &[&str] = &[
    "game_dir",
    "worlds_dir",
    "host",
    "port",
    "address",
    "slot",
    "password",
];

/// Ce qui rend un profil inutilisable.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// Fichier de profil illisible.
    #[error("lecture de {path} : {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Le TOML est mal formé, ou contient un champ inconnu / une valeur interdite.
    #[error("profil {file} : {source}")]
    Toml {
        file: String,
        source: toml::de::Error,
    },
    /// Le profil vient d'un launcher plus récent.
    #[error("profil {file} : format_version {found}, ce launcher lit jusqu'à la version {max} : mettez Raftup à jour")]
    UnsupportedVersion { file: String, found: u32, max: u32 },
    /// Une règle de validation n'est pas respectée.
    #[error("profil {file} : {reason}")]
    Invalid { file: String, reason: String },
    /// Un gabarit (`{variable}`) est faux.
    #[error("profil {file} : {source}")]
    Template { file: String, source: TemplateError },
}

// ---------------------------------------------------------------------------
// Types du profil
// ---------------------------------------------------------------------------

/// Un profil complet (un fichier `profiles/<game.id>.toml`).
///
/// `deny_unknown_fields` : un champ inconnu (faute de frappe, champ d'un futur format)
/// est une erreur, jamais ignoré en silence.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub format_version: u32,
    pub game: GameInfo,
    pub credits: Credits,
    pub steps: Vec<Step>,
    /// `#[serde(default)]` : section facultative, vide si absente.
    #[serde(default)]
    pub preserve: Preserve,
    #[serde(default)]
    pub cleanup: Cleanup,
    pub launch: Launch,
    /// Images choisies par le créateur (ADR 0051), installées avec le jeu.
    /// Section facultative, donc `format_version` ne change pas.
    #[serde(default)]
    pub art: crate::art::ArtSection,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameInfo {
    pub id: String,
    pub name: String,
    /// Nom du jeu côté Archipelago.
    pub ap_game: String,
    pub status: GameStatus,
    /// Temps moyen pour visiter un lieu, en secondes (facultatif, ADR 0030) : sert à
    /// estimer la durée d'une partie. Absent : `estimate::DEFAULT_SECONDS_PER_LOCATION`.
    /// Champ facultatif, donc `format_version` ne change pas.
    #[serde(default)]
    pub seconds_per_location: Option<u32>,
    /// Identifiant du jeu chez IGDB (ADR 0047). **Ignoré** depuis l'ADR 0051 (images
    /// choisies par le créateur, section `[art]`) ; encore accepté pour que les profils
    /// qui l'ont gardé restent lisibles.
    #[serde(default)]
    pub igdb_id: Option<u64>,
    #[serde(default)]
    pub detect: DetectInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameStatus {
    Available,
    /// Profil jamais essayé avec le vrai jeu (ADR 0049) : installable comme
    /// `available`, mais le launcher l'annonce « Non testé ».
    Untested,
    Broken,
}

/// Règles de détection du jeu (lues par `detect.rs`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectInfo {
    #[serde(default)]
    pub steam_appid: Option<u32>,
    #[serde(default)]
    pub epic_app_name: Option<String>,
    #[serde(default)]
    pub expected_exe: Option<String>,
}

/// Qui a fait les mods, et sous quelle licence (ou avec quelle autorisation).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credits {
    pub authors: Vec<String>,
    /// Identifiant SPDX, `"permission"` (autorisation écrite de l'auteur) ou `"unlicensed"`
    /// (le mod n'a aucune licence : Raftup ne le redistribue pas, il le télécharge chez
    /// l'auteur sur le PC du joueur ; décision de la mainteneuse, ADR 0021).
    pub license: String,
    #[serde(default)]
    pub permission_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    ModLoader,
    Mod,
    Apworld,
}

/// Une étape d'installation. `sha256` est toujours obligatoire.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub kind: StepKind,
    pub name: String,
    pub version: String,
    pub source: Source,
    pub sha256: Sha256,
    #[serde(default)]
    pub max_bytes: Option<u64>,
    pub install: InstallAction,
}

impl Step {
    /// Taille maximale à passer au téléchargeur.
    pub fn max_bytes_or_default(&self) -> u64 {
        self.max_bytes.unwrap_or(DEFAULT_MAX_BYTES)
    }
}

/// D'où vient le fichier. TOML : `{ url = "…" }` ou `{ thunderstore = "owner-name" }`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Url(String),
    Thunderstore(String),
}

impl Source {
    /// Adresse de téléchargement, sans appel réseau ni API.
    ///
    /// Thunderstore : `owner-name` est coupé au premier `-`.
    pub fn resolve(&self, version: &str) -> String {
        match self {
            Source::Url(url) => url.clone(),
            Source::Thunderstore(package) => {
                let (owner, name) = package.split_once('-').unwrap_or((package, ""));
                format!("https://thunderstore.io/package/download/{owner}/{name}/{version}/")
            }
        }
    }
}

/// Ce que l'étape fait du fichier téléchargé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallAction {
    /// Extraire une archive zip dans `into`, en retirant `strip_prefix`.
    ///
    /// `inner` (facultatif, ADR 0050) : chemin exact d'une autre archive zip **dans**
    /// le fichier téléchargé ; c'est elle qu'on extrait (installateur de SMAPI, dont les
    /// fichiers Windows sont dans `internal/windows/install.dat`).
    Extract {
        into: String,
        strip_prefix: String,
        inner: Option<String>,
    },
    /// Copier le fichier tel quel vers ce chemin.
    CopyTo(String),
}

/// Forme « brute » lue dans le TOML, avant conversion en `InstallAction`.
/// Cela donne de bons messages d'erreur (un enum `untagged` n'en donne presque pas).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstall {
    extract: Option<String>,
    into: Option<String>,
    strip_prefix: Option<String>,
    inner: Option<String>,
    copy_to: Option<String>,
}

// Désérialisation écrite à la main : serde lit d'abord RawInstall, puis on choisit la variante.
impl<'de> Deserialize<'de> for InstallAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawInstall::deserialize(deserializer)?;
        let bad = serde::de::Error::custom;
        match raw {
            RawInstall {
                extract: Some(format),
                into: Some(into),
                strip_prefix,
                inner,
                copy_to: None,
            } => {
                if format != "zip" {
                    return Err(bad(format!(
                        "format d'archive « {format} » non géré (seulement « zip »)"
                    )));
                }
                Ok(InstallAction::Extract {
                    into,
                    strip_prefix: strip_prefix.unwrap_or_default(),
                    inner,
                })
            }
            RawInstall {
                extract: None,
                into: None,
                strip_prefix: None,
                inner: None,
                copy_to: Some(path),
            } => Ok(InstallAction::CopyTo(path)),
            _ => Err(bad(
                "install : écrire `{ extract = \"zip\", into = \"…\" }` ou `{ copy_to = \"…\" }`"
                    .to_string(),
            )),
        }
    }
}

/// Fichiers jamais écrasés par une mise à jour (F07).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preserve {
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Fichiers créés par le jeu en tournant ; supprimés seulement après confirmation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cleanup {
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub exe: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub connect: ConnectMethod,
}

/// Comment le mod reçoit l'adresse, le slot et le mot de passe (F04).
/// `tag = "method"` : le champ `method` choisit la variante.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConnectMethod {
    CliArgs {
        #[serde(default)]
        args: Vec<String>,
    },
    ConfigFile {
        path: String,
        /// `"json"` seulement pour l'instant (ADR 0015, décision 9).
        format: String,
        #[serde(default)]
        set: BTreeMap<String, String>,
    },
    Env {
        #[serde(default)]
        vars: BTreeMap<String, String>,
    },
    Uri {
        scheme: String,
        template: String,
    },
    Manual {
        instructions: String,
    },
}

// ---------------------------------------------------------------------------
// Chargement et validation
// ---------------------------------------------------------------------------

impl Profile {
    /// Lit un profil puis le valide. `file_name` : nom du fichier (`<id>.toml`).
    pub fn parse(file_name: &str, text: &str) -> Result<Profile, ProfileError> {
        let file = file_name.to_string();

        // On lit d'abord le TOML brut pour contrôler la version : un profil futur peut
        // avoir des champs inconnus, et doit répondre « mettez à jour », pas « champ inconnu ».
        let raw: toml::Table = text.parse().map_err(|source| ProfileError::Toml {
            file: file.clone(),
            source,
        })?;
        if let Some(found) = raw.get("format_version").and_then(|v| v.as_integer()) {
            let found = u32::try_from(found).unwrap_or(u32::MAX);
            if found > FORMAT_VERSION {
                return Err(ProfileError::UnsupportedVersion {
                    file,
                    found,
                    max: FORMAT_VERSION,
                });
            }
        }

        let profile: Profile = raw.try_into().map_err(|source| ProfileError::Toml {
            file: file.clone(),
            source,
        })?;
        profile.validate(file_name)?;
        Ok(profile)
    }

    fn validate(&self, file: &str) -> Result<(), ProfileError> {
        let invalid = |reason: String| ProfileError::Invalid {
            file: file.to_string(),
            reason,
        };
        let tpl = |source: TemplateError| ProfileError::Template {
            file: file.to_string(),
            source,
        };

        // --- Identité ---
        let id = &self.game.id;
        let id_ok = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !id_ok {
            return Err(invalid(format!(
                "game.id « {id} » : seulement a-z, 0-9 et « - »"
            )));
        }
        if file != format!("{id}.toml") {
            return Err(invalid(format!(
                "le fichier doit s'appeler {id}.toml (nom reçu : {file})"
            )));
        }
        if self.game.name.trim().is_empty() || self.game.ap_game.trim().is_empty() {
            return Err(invalid(
                "game.name et game.ap_game sont obligatoires".into(),
            ));
        }
        if let Some(exe) = &self.game.detect.expected_exe {
            if exe.is_empty() || exe.contains(['/', '\\', ':']) || exe.contains("..") {
                return Err(invalid(format!(
                    "game.detect.expected_exe « {exe} » doit être un simple nom de fichier"
                )));
            }
        }

        if let Some(seconds) = self.game.seconds_per_location {
            if !(1..=crate::estimate::MAX_SECONDS_PER_LOCATION).contains(&seconds) {
                return Err(invalid(format!(
                    "game.seconds_per_location {seconds} : entre 1 et {} secondes",
                    crate::estimate::MAX_SECONDS_PER_LOCATION
                )));
            }
        }

        // --- Images du créateur (ADR 0051) ---
        for kind in crate::art::ArtKind::ALL {
            if let Some(file) = self.art.get(kind) {
                check_url(&file.url, crate::art::ART_HOSTS)
                    .map_err(|reason| invalid(format!("art.{} : {reason}", kind.as_str())))?;
            }
        }

        // --- Crédits ---
        if self.credits.authors.is_empty() || self.credits.license.trim().is_empty() {
            return Err(invalid(
                "credits.authors et credits.license sont obligatoires".into(),
            ));
        }
        if self.credits.license == "permission" && self.credits.permission_url.is_none() {
            return Err(invalid(
                "licence « permission » : credits.permission_url est obligatoire".into(),
            ));
        }
        if let Some(url) = &self.credits.permission_url {
            if !url.starts_with("https://") {
                return Err(invalid(
                    "credits.permission_url doit être en https://".into(),
                ));
            }
        }

        // --- Étapes ---
        let mut seen = BTreeSet::new();
        for step in &self.steps {
            let step_id = &step.id;
            let ok = !step_id.is_empty()
                && step_id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !ok {
                return Err(invalid(format!(
                    "étape « {step_id} » : id avec seulement a-z, 0-9 et « - »"
                )));
            }
            if !seen.insert(step_id.as_str()) {
                return Err(invalid(format!("id d'étape en double : {step_id}")));
            }
            if step.name.trim().is_empty() {
                return Err(invalid(format!("étape {step_id} : name est obligatoire")));
            }
            // La version entre dans une URL (Thunderstore) : caractères sûrs seulement.
            let version_ok = !step.version.is_empty()
                && step
                    .version
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'));
            if !version_ok {
                return Err(invalid(format!(
                    "étape {step_id} : version « {} » invalide",
                    step.version
                )));
            }
            if step.max_bytes == Some(0) {
                return Err(invalid(format!(
                    "étape {step_id} : max_bytes doit être > 0"
                )));
            }
            check_source(&step.source)
                .map_err(|reason| invalid(format!("étape {step_id} : {reason}")))?;

            match &step.install {
                InstallAction::Extract {
                    into,
                    strip_prefix,
                    inner,
                } => {
                    check_target(file, into, STEP_VARS, STEP_VARS, true)?;
                    // L'archive intérieure est cherchée par son nom exact dans le zip : un
                    // chemin sûr (pas de `..`, pas de chemin absolu), comme une entrée.
                    if let Some(inner) = inner {
                        let unsafe_inner = inner.is_empty()
                            || crate::archive::safe_relative_path(inner).is_err()
                            || inner.contains(['{', '}']);
                        if unsafe_inner {
                            return Err(invalid(format!(
                                "étape {step_id} : inner « {inner} » interdit"
                            )));
                        }
                    }
                    let bad_prefix = strip_prefix.contains("..")
                        || strip_prefix.starts_with(['/', '\\'])
                        || strip_prefix.contains([':', '{', '}', '\0', '\n', '\r']);
                    if bad_prefix {
                        return Err(invalid(format!(
                            "étape {step_id} : strip_prefix « {strip_prefix} » interdit"
                        )));
                    }
                }
                InstallAction::CopyTo(path) => {
                    check_target(file, path, STEP_VARS, STEP_VARS, false)?;
                }
            }
        }

        // --- preserve / cleanup ---
        for path in self.preserve.paths.iter().chain(&self.cleanup.paths) {
            check_target(file, path, STEP_VARS, STEP_VARS, false)?;
        }

        // --- Lancement ---
        check_target(file, &self.launch.exe, LAUNCH_VARS, &["game_dir"], false)?;
        for arg in &self.launch.args {
            template::check(arg, LAUNCH_VARS).map_err(tpl)?;
        }
        match &self.launch.connect {
            ConnectMethod::CliArgs { args } => {
                for arg in args {
                    template::check(arg, LAUNCH_VARS).map_err(tpl)?;
                }
            }
            ConnectMethod::ConfigFile { path, format, set } => {
                if format != "json" {
                    return Err(invalid(format!(
                        "connect.format « {format} » non géré (seulement « json »)"
                    )));
                }
                check_target(file, path, LAUNCH_VARS, &["game_dir"], false)?;
                for value in set.values() {
                    template::check(value, LAUNCH_VARS).map_err(tpl)?;
                }
            }
            ConnectMethod::Env { vars } => {
                for (name, value) in vars {
                    if name.is_empty() || name.contains(['=', '\0']) {
                        return Err(invalid(format!("connect.vars : nom « {name} » invalide")));
                    }
                    template::check(value, LAUNCH_VARS).map_err(tpl)?;
                }
            }
            ConnectMethod::Uri {
                scheme,
                template: t,
            } => {
                let scheme_ok = scheme
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase())
                    && scheme.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '.' | '-')
                    });
                if !scheme_ok {
                    return Err(invalid(format!("connect.scheme « {scheme} » invalide")));
                }
                template::check(t, LAUNCH_VARS).map_err(tpl)?;
            }
            ConnectMethod::Manual { instructions } => {
                if instructions.trim().is_empty() {
                    return Err(invalid("connect.instructions est vide".into()));
                }
            }
        }
        Ok(())
    }
}

/// Contrôle une source : HTTPS, domaine autorisé, ou nom de paquet Thunderstore valide.
fn check_source(source: &Source) -> Result<(), String> {
    match source {
        Source::Url(url) => check_url(url, ALLOWED_HOSTS),
        Source::Thunderstore(package) => {
            let valid =
                |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            match package.split_once('-') {
                Some((owner, name)) if valid(owner) && valid(name) => Ok(()),
                _ => Err(format!(
                    "source thunderstore « {package} » : attendu « proprietaire-nom »"
                )),
            }
        }
    }
}

/// Contrôle une adresse : HTTPS, sans caractère douteux, hôte dans `hosts`.
fn check_url(url: &str, hosts: &[&str]) -> Result<(), String> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(format!("source « {url} » : HTTPS obligatoire"));
    };
    if url.contains(['{', '}', ' ', '\0', '\n', '\r']) {
        return Err(format!("source « {url} » : caractère interdit"));
    }
    // L'hôte s'arrête au premier « / », « ? » ou « # ». Un « @ » (identifiants)
    // ou un « : » (port) servirait à tromper la liste blanche : refusés.
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.contains(['@', ':']) {
        return Err(format!("source « {url} » : hôte invalide"));
    }
    if !hosts.contains(&host.to_ascii_lowercase().as_str()) {
        return Err(format!(
            "source « {url} » : domaine « {host} » hors de la liste blanche"
        ));
    }
    Ok(())
}

/// Contrôle un chemin du profil : gabarit valide, enraciné dans une des racines
/// autorisées, sans `..`, sans second gabarit ni `:` (lecteur ou flux NTFS) après la racine.
///
/// - `vars` : variables permises dans le gabarit ;
/// - `roots` : variables permises comme racine (`game_dir`, éventuellement `worlds_dir`) ;
/// - `allow_bare_root` : accepter la racine seule (`into = "{game_dir}"`).
fn check_target(
    file: &str,
    path: &str,
    vars: &[&str],
    roots: &[&str],
    allow_bare_root: bool,
) -> Result<(), ProfileError> {
    let invalid = |reason: String| ProfileError::Invalid {
        file: file.to_string(),
        reason,
    };
    template::check(path, vars).map_err(|source| ProfileError::Template {
        file: file.to_string(),
        source,
    })?;
    if path.contains("..") || path.contains(['\0', '\n', '\r']) {
        return Err(invalid(format!("chemin « {path} » : « .. » interdit")));
    }
    let rest = roots
        .iter()
        .find_map(|root| path.strip_prefix(&format!("{{{root}}}")));
    let Some(rest) = rest else {
        return Err(invalid(format!(
            "chemin « {path} » : doit commencer par {}",
            roots
                .iter()
                .map(|r| format!("{{{r}}}"))
                .collect::<Vec<_>>()
                .join(" ou ")
        )));
    };
    if !(rest.is_empty() || rest.starts_with(['/', '\\'])) {
        return Err(invalid(format!(
            "chemin « {path} » : un séparateur doit suivre la racine"
        )));
    }
    if rest.contains(['{', '}', ':']) {
        return Err(invalid(format!(
            "chemin « {path} » : pas de variable ni de « : » après la racine"
        )));
    }
    if !allow_bare_root && rest.trim_matches(['/', '\\']).is_empty() {
        return Err(invalid(format!(
            "chemin « {path} » : nom de fichier manquant"
        )));
    }
    Ok(())
}
