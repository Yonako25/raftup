//! Plan de lancement (`plan_launch`, fonction pure) puis lancement du jeu, sans shell.
//!
//! Deux temps (ADR 0015) :
//! 1. [`plan_launch`] calcule tout ce qu'il faut faire (programme, arguments, variables
//!    d'environnement, fichier de config à modifier) sans rien toucher : facile à tester ;
//! 2. [`launch`] exécute le plan : il modifie la config, puis démarre l'exécutable
//!    **directement** (jamais via `cmd` ou un shell), donc pas d'injection de commande.
//!
//! Mot de passe : le profil choisit UNE méthode de connexion. L'ordre de préférence des
//! profils est `env`, puis `config_file`, puis `cli_args` (le mot de passe d'un argument est
//! visible dans la liste des processus). Il est masqué dans tous les `Debug` de ce module.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use client_core::DEFAULT_PORT;
use serde_json::{Map, Value};

use crate::profile::{ConnectMethod, Profile};
use crate::template::{self, TemplateError, Vars};

/// Taille maximale d'un fichier de config JSON qu'on accepte de relire (1 Mio).
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Ce qui empêche de lancer un jeu.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// Exécutable introuvable ou impossible à démarrer.
    #[error("lancement de {path} : {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    /// Un gabarit du profil ne peut pas être rempli (variable inconnue, valeur interdite…).
    #[error(transparent)]
    Template(#[from] TemplateError),

    /// L'URI construite ne commence pas par le schéma du profil.
    #[error("l'URI de connexion doit commencer par « {scheme}: »")]
    BadUri { scheme: String },

    /// Le fichier de configuration du mod ne peut pas être mis à jour.
    #[error("configuration du mod {path} : {reason}")]
    Config { path: PathBuf, reason: String },

    /// Méthode de lancement pas encore prise en charge (décision 9 de l'ADR 0015).
    #[error("lancement par {0} pas encore pris en charge")]
    Unsupported(&'static str),
}

/// Adresse, slot et mot de passe saisis par le joueur.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub address: String,
    pub slot: String,
    pub password: Option<String>,
}

// `Debug` écrit à la main : le dérivé afficherait le mot de passe (NF09).
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("address", &self.address)
            .field("slot", &self.slot)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish()
    }
}

/// Modification à faire dans un fichier JSON de config du mod.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigEdit {
    /// Fichier à modifier (créé s'il n'existe pas).
    pub path: PathBuf,
    /// Clé (avec `.` pour descendre dans les objets : `a.b`) vers valeur texte.
    pub values: BTreeMap<String, String>,
}

/// Ce que `launch` va faire, calculé sans rien exécuter.
#[derive(Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub exe: PathBuf,
    pub args: Vec<String>,
    /// Variables d'environnement à ajouter à celles du launcher.
    pub env: BTreeMap<String, String>,
    pub config_edit: Option<ConfigEdit>,
    /// URI de connexion (méthode `uri`).
    pub uri: Option<String>,
    /// Consigne à montrer au joueur (méthode `manual`).
    pub manual_instructions: Option<String>,
    /// Mot de passe, gardé seulement pour le masquer dans `Debug`.
    secret: Option<String>,
}

// `Debug` du plan : on fabrique le texte du dérivé puis on remplace le mot de passe.
impl fmt::Debug for LaunchPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = format!(
            "LaunchPlan {{ exe: {:?}, args: {:?}, env: {:?}, config_edit: {:?}, uri: {:?}, \
             manual_instructions: {:?} }}",
            self.exe,
            self.args,
            self.env,
            self.config_edit.as_ref().map(|c| (&c.path, &c.values)),
            self.uri,
            self.manual_instructions
        );
        match self.secret.as_deref() {
            Some(secret) if !secret.is_empty() => f.write_str(&text.replace(secret, "***")),
            _ => f.write_str(&text),
        }
    }
}

/// Sépare `ws://hote:port/chemin` en (hôte, port). Port par défaut : 38281.
fn split_address(address: &str) -> (String, String) {
    let address = address.trim();
    let without_scheme = address.split_once("://").map_or(address, |(_, rest)| rest);
    let host_port = without_scheme.split('/').next().unwrap_or("");
    // IPv6 : « [::1]:38281 », les deux-points de l'adresse sont entre crochets.
    let after_bracket = host_port.rfind(']').map_or(0, |i| i + 1);
    match host_port[after_bracket..].rfind(':') {
        Some(i) => {
            let split = after_bracket + i;
            (
                host_port[..split].to_string(),
                host_port[split + 1..].to_string(),
            )
        }
        None => (host_port.to_string(), DEFAULT_PORT.to_string()),
    }
}

/// Calcule le plan de lancement. Fonction pure : rien n'est lu ni écrit.
///
/// - `game_dir` : dossier du jeu (`{game_dir}`) ;
/// - `worlds_dir` : dossier des apworlds (`{worlds_dir}`).
pub fn plan_launch(
    profile: &Profile,
    game_dir: &Path,
    worlds_dir: &Path,
    credentials: &Credentials,
) -> Result<LaunchPlan, LaunchError> {
    let (host, port) = split_address(&credentials.address);
    let vars = Vars::new()
        .with("game_dir", &game_dir.to_string_lossy())
        .with("worlds_dir", &worlds_dir.to_string_lossy())
        .with("address", &credentials.address)
        .with("host", &host)
        .with("port", &port)
        .with("slot", &credentials.slot)
        .with("password", credentials.password.as_deref().unwrap_or(""));

    let launch = &profile.launch;
    let exe = PathBuf::from(template::expand(&launch.exe, &vars)?);
    let mut args = expand_all(&launch.args, &vars)?;
    let mut env = BTreeMap::new();
    let mut config_edit = None;
    let mut uri = None;
    let mut manual_instructions = None;

    match &launch.connect {
        ConnectMethod::CliArgs { args: more } => args.extend(expand_all(more, &vars)?),
        ConnectMethod::Env { vars: wanted } => {
            for (name, value) in wanted {
                env.insert(name.clone(), template::expand(value, &vars)?);
            }
        }
        ConnectMethod::ConfigFile { path, set, .. } => {
            let mut values = BTreeMap::new();
            for (key, value) in set {
                values.insert(key.clone(), template::expand(value, &vars)?);
            }
            config_edit = Some(ConfigEdit {
                path: PathBuf::from(template::expand(path, &vars)?),
                values,
            });
        }
        ConnectMethod::Uri {
            scheme,
            template: t,
        } => {
            let built = template::expand(t, &vars)?;
            if !built.starts_with(&format!("{scheme}:")) {
                return Err(LaunchError::BadUri {
                    scheme: scheme.clone(),
                });
            }
            uri = Some(built);
        }
        ConnectMethod::Manual { instructions } => manual_instructions = Some(instructions.clone()),
    }

    Ok(LaunchPlan {
        exe,
        args,
        env,
        config_edit,
        uri,
        manual_instructions,
        secret: credentials.password.clone(),
    })
}

fn expand_all(templates: &[String], vars: &Vars) -> Result<Vec<String>, LaunchError> {
    templates
        .iter()
        .map(|t| template::expand(t, vars).map_err(LaunchError::from))
        .collect()
}

/// Exécute le plan : met à jour la config du mod, puis démarre le jeu sans shell.
///
/// Renvoie le processus lancé (l'appelant peut le suivre ou l'ignorer).
pub fn launch(plan: &LaunchPlan) -> Result<Child, LaunchError> {
    if plan.uri.is_some() {
        // Ouvrir une URI demande un code propre à chaque plateforme : écrit au premier
        // jeu qui en a besoin (ADR 0015, décision 9).
        return Err(LaunchError::Unsupported("uri"));
    }
    if !plan.exe.is_file() {
        return Err(LaunchError::Io {
            path: plan.exe.clone(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "exécutable introuvable"),
        });
    }
    if let Some(edit) = &plan.config_edit {
        apply_config_edit(edit)?;
    }
    // `Command` lance le programme directement, avec ses arguments séparés : aucun shell
    // n'interprète le texte, donc un slot « a & b » reste un seul argument.
    let mut command = Command::new(&plan.exe);
    command.args(&plan.args).envs(&plan.env);
    if let Some(dir) = plan.exe.parent() {
        command.current_dir(dir);
    }
    command.spawn().map_err(|source| LaunchError::Io {
        path: plan.exe.clone(),
        source,
    })
}

/// Fusionne les valeurs dans le fichier JSON : les autres clés sont conservées.
fn apply_config_edit(edit: &ConfigEdit) -> Result<(), LaunchError> {
    let fail = |reason: String| LaunchError::Config {
        path: edit.path.clone(),
        reason,
    };

    let mut root = match fs::File::open(&edit.path) {
        Ok(file) => {
            let mut text = String::new();
            // `take` borne la lecture : un fichier géant ne remplit pas la mémoire.
            file.take(MAX_CONFIG_BYTES + 1)
                .read_to_string(&mut text)
                .map_err(|e| fail(e.to_string()))?;
            if text.len() as u64 > MAX_CONFIG_BYTES {
                return Err(fail("fichier trop volumineux".into()));
            }
            match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(map)) => map,
                Ok(_) => return Err(fail("le JSON n'est pas un objet".into())),
                Err(e) => return Err(fail(format!("JSON invalide : {e}"))),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(fail(e.to_string())),
    };

    for (key, value) in &edit.values {
        set_nested(&mut root, key, value).map_err(fail)?;
    }

    let text =
        serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| fail(e.to_string()))?;
    if let Some(parent) = edit.path.parent() {
        fs::create_dir_all(parent).map_err(|e| fail(e.to_string()))?;
    }
    // Écriture atomique : fichier temporaire puis renommage, pour ne jamais laisser une
    // config à moitié écrite si le launcher est interrompu.
    let mut tmp_name = edit.path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    fs::write(&tmp, text).map_err(|e| fail(e.to_string()))?;
    fs::rename(&tmp, &edit.path).map_err(|e| fail(e.to_string()))?;
    Ok(())
}

/// Écrit `value` à la clé `a.b.c`, en créant les objets intermédiaires.
fn set_nested(root: &mut Map<String, Value>, key: &str, value: &str) -> Result<(), String> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(format!("clé « {key} » invalide"));
    }
    let (last, parents) = parts.split_last().ok_or_else(|| "clé vide".to_string())?;
    let mut current = root;
    for part in parents {
        let entry = current
            .entry((*part).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        // `as_object_mut` : emprunt modifiable de l'objet, ou `None` si ce n'en est pas un.
        current = entry
            .as_object_mut()
            .ok_or_else(|| format!("« {part} » existe mais n'est pas un objet"))?;
    }
    current.insert((*last).to_string(), Value::String(value.to_string()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoupe_de_l_adresse() {
        assert_eq!(
            split_address("ws://h.fr:1234/x"),
            ("h.fr".into(), "1234".into())
        );
        assert_eq!(split_address("h.fr"), ("h.fr".into(), "38281".into()));
        assert_eq!(split_address("[::1]:5"), ("[::1]".into(), "5".into()));
        assert_eq!(split_address("[::1]"), ("[::1]".into(), "38281".into()));
    }
}
