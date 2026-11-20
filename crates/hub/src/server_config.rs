//! Fichier de configuration du serveur `rafthub-data/rafthub.toml` (ADR 0037).
//!
//! Il garde le choix fait dans l'assistant de premier lancement : quelle base de
//! données utiliser. Les autres réglages (SMTP, Discord…) vivent dans la base
//! elle-même (ADR 0036) ; ce fichier ne sert qu'à **trouver** la base.
//!
//! ```toml
//! format_version = 1
//! database_url = "sqlite://C:/rafthub/rafthub-data/rafthub.db"
//! ```
//!
//! L'URL d'une base externe contient son mot de passe : le fichier est donc aussi
//! secret que la base elle-même.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::HubError;

/// Version du format du fichier ; un fichier d'une autre version est refusé.
pub const FORMAT_VERSION: u32 = 1;
/// Nom du fichier, dans le dossier de données.
pub const FILE_NAME: &str = "rafthub.toml";
/// Nom de la base intégrée, dans le dossier de données.
pub const SQLITE_FILE_NAME: &str = "rafthub.db";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub format_version: u32,
    /// URL de la base : `sqlite://…` ou `postgres://…`.
    pub database_url: String,
}

impl ServerConfig {
    pub fn new(database_url: String) -> ServerConfig {
        ServerConfig {
            format_version: FORMAT_VERSION,
            database_url,
        }
    }

    /// URL à ouvrir : un chemin SQLite relatif est pris depuis `data_dir`.
    pub fn database_url_in(&self, data_dir: &Path) -> String {
        let Some(rest) = self.database_url.strip_prefix("sqlite://") else {
            return self.database_url.clone();
        };
        let (file, query) = match rest.split_once('?') {
            Some((file, query)) => (file, Some(query)),
            None => (rest, None),
        };
        if file.is_empty() || Path::new(file).is_absolute() || file.starts_with(':') {
            return self.database_url.clone();
        }
        let path = data_dir.join(file).display().to_string().replace('\\', "/");
        match query {
            Some(query) => format!("sqlite://{path}?{query}"),
            None => format!("sqlite://{path}"),
        }
    }

    /// Lit le fichier du dossier `data_dir`. `Ok(None)` s'il n'existe pas encore :
    /// c'est le premier lancement, l'assistant prend le relais.
    pub fn load(data_dir: &Path) -> Result<Option<ServerConfig>, HubError> {
        let path = data_dir.join(FILE_NAME);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(HubError::Internal(format!(
                    "lecture de {} : {error}",
                    path.display()
                )))
            }
        };
        // Pas le texte de l'erreur TOML : il pourrait citer l'URL et son mot de passe.
        let config: ServerConfig = toml::from_str(&text)
            .map_err(|_| HubError::Internal(format!("{} illisible", path.display())))?;
        if config.format_version != FORMAT_VERSION {
            return Err(HubError::Internal(format!(
                "{} : format_version {} inconnue (attendu {FORMAT_VERSION})",
                path.display(),
                config.format_version
            )));
        }
        Ok(Some(config))
    }

    /// Écrit le fichier dans `data_dir` (en passant par un fichier temporaire, pour
    /// ne jamais laisser un fichier à moitié écrit).
    pub fn save(&self, data_dir: &Path) -> Result<(), HubError> {
        let text = toml::to_string(self)
            .map_err(|error| HubError::Internal(format!("écriture de la config : {error}")))?;
        let path = data_dir.join(FILE_NAME);
        let temp = data_dir.join(format!("{FILE_NAME}.tmp"));
        let io = |error: std::io::Error| {
            HubError::Internal(format!("écriture de {} : {error}", path.display()))
        };
        std::fs::write(&temp, text).map_err(io)?;
        std::fs::rename(&temp, &path).map_err(io)?;
        Ok(())
    }
}

/// URL de la base intégrée telle qu'elle est écrite dans `rafthub.toml` : un chemin
/// **relatif** au dossier de données, pour que le hub retrouve sa base même si on
/// déplace son dossier. [`ServerConfig::database_url_in`] le complète au chargement.
pub const EMBEDDED_URL: &str = "sqlite://rafthub.db";

/// URL complète de la base intégrée, rangée dans `data_dir`. Les `\` de Windows deviennent
/// des `/`, que SQLite comprend.
pub fn sqlite_url(data_dir: &Path) -> String {
    let path: PathBuf = data_dir.join(SQLITE_FILE_NAME);
    format!("sqlite://{}", path.display().to_string().replace('\\', "/"))
}

/// Bases externes proposées par l'assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Postgres,
    Mysql,
    Mariadb,
}

impl Engine {
    /// Valeur du formulaire (`postgres`, `mysql`, `mariadb`).
    pub fn parse(value: &str) -> Option<Engine> {
        match value {
            "postgres" => Some(Engine::Postgres),
            "mysql" => Some(Engine::Mysql),
            "mariadb" => Some(Engine::Mariadb),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Postgres => "postgres",
            Engine::Mysql => "mysql",
            Engine::Mariadb => "mariadb",
        }
    }

    /// Nom affiché dans les messages.
    pub fn label(self) -> &'static str {
        match self {
            Engine::Postgres => "PostgreSQL",
            Engine::Mysql => "MySQL",
            Engine::Mariadb => "MariaDB",
        }
    }

    /// Port habituel du serveur.
    pub fn default_port(self) -> u16 {
        match self {
            Engine::Postgres => 5432,
            Engine::Mysql | Engine::Mariadb => 3306,
        }
    }

    /// Début de l'URL. MariaDB parle le protocole de MySQL : même pilote.
    fn scheme(self) -> &'static str {
        match self {
            Engine::Postgres => "postgres",
            Engine::Mysql | Engine::Mariadb => "mysql",
        }
    }
}

/// Informations de connexion à une base externe, telles que tapées dans
/// l'assistant.
#[derive(Debug, Clone)]
pub struct ExternalDb {
    pub engine: Engine,
    pub host: String,
    pub port: String,
    pub database: String,
    pub user: String,
    pub password: String,
}

impl ExternalDb {
    /// URL `postgres://utilisateur:motdepasse@hote:port/base` (ou `mysql://…`). Le crate `url` encode
    /// lui-même les caractères spéciaux du nom et du mot de passe (`@`, `:`, `/`…).
    /// L'erreur est un message pour l'assistant, sans le mot de passe.
    pub fn to_url(&self) -> Result<String, String> {
        let host = self.host.trim();
        if host.is_empty() {
            return Err(format!(
                "Indiquez l'adresse du serveur {}.",
                self.engine.label()
            ));
        }
        let port: u16 = match self.port.trim() {
            "" => self.engine.default_port(),
            text => text
                .parse()
                .map_err(|_| "Le port doit être un nombre entre 1 et 65535.".to_string())?,
        };
        let database = self.database.trim();
        if database.is_empty() {
            return Err("Indiquez le nom de la base.".to_string());
        }
        let user = self.user.trim();
        if user.is_empty() {
            return Err("Indiquez le nom d'utilisateur de la base.".to_string());
        }
        let invalid = || "Adresse du serveur invalide.".to_string();
        let mut url = url::Url::parse(&format!("{}://localhost", self.engine.scheme()))
            .map_err(|_| invalid())?;
        url.set_host(Some(host)).map_err(|_| invalid())?;
        url.set_port(Some(port)).map_err(|_| invalid())?;
        url.set_username(user).map_err(|_| invalid())?;
        if !self.password.is_empty() {
            url.set_password(Some(&self.password))
                .map_err(|_| invalid())?;
        }
        url.set_path(database);
        Ok(url.to_string())
    }
}

/// Variable d'environnement qui choisit le dossier de données à la place de
/// `rafthub-data/` à côté de l'exe. Sert surtout dans un conteneur Docker, où le
/// dossier doit être sur un volume (`/data`) et non à côté du binaire.
pub const DATA_DIR_ENV: &str = "RAFTUP_HUB_DATA_DIR";

/// Nom du dossier de données par défaut, à côté de l'exe.
pub const DEFAULT_DATA_DIR_NAME: &str = "rafthub-data";

/// Dossier de données : la valeur de [`DATA_DIR_ENV`] si elle est donnée (et non
/// vide), sinon `rafthub-data/` dans le dossier de l'exe. Fonction pure, sans lecture
/// de l'environnement ni création de dossier, pour pouvoir la tester.
/// `None` seulement si l'exe n'a pas de dossier parent (cas théorique).
pub fn resolve_data_dir(from_env: Option<&str>, exe: &Path) -> Option<PathBuf> {
    match from_env.map(str::trim).filter(|dir| !dir.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => exe
            .parent()
            .map(|parent| parent.join(DEFAULT_DATA_DIR_NAME)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn dossier_de_donnees_a_cote_de_l_exe_par_defaut() {
        let exe = Path::new("/opt/rafthub/rafthub");
        assert_eq!(
            resolve_data_dir(None, exe),
            Some(PathBuf::from("/opt/rafthub/rafthub-data"))
        );
        // Une variable vide (ou faite d'espaces) compte comme absente.
        assert_eq!(
            resolve_data_dir(Some("  "), exe),
            Some(PathBuf::from("/opt/rafthub/rafthub-data"))
        );
    }

    #[test]
    fn dossier_de_donnees_choisi_par_la_variable() {
        let exe = Path::new("/usr/local/bin/rafthub");
        assert_eq!(
            resolve_data_dir(Some("/data"), exe),
            Some(PathBuf::from("/data"))
        );
    }

    fn info(password: &str) -> ExternalDb {
        ExternalDb {
            engine: Engine::Postgres,
            host: "db.exemple.org".to_string(),
            port: String::new(),
            database: "rafthub".to_string(),
            user: "hub".to_string(),
            password: password.to_string(),
        }
    }

    #[test]
    fn url_postgres_avec_port_par_defaut() {
        assert_eq!(
            info("secret").to_url().unwrap(),
            "postgres://hub:secret@db.exemple.org:5432/rafthub"
        );
    }

    #[test]
    fn url_mysql_et_mariadb() {
        let mut db = info("secret");
        db.engine = Engine::Mariadb;
        assert_eq!(
            db.to_url().unwrap(),
            "mysql://hub:secret@db.exemple.org:3306/rafthub"
        );
        db.engine = Engine::Mysql;
        db.port = "3307".to_string();
        assert!(db.to_url().unwrap().ends_with(":3307/rafthub"));
        assert_eq!(Engine::parse("mariadb"), Some(Engine::Mariadb));
        assert_eq!(Engine::parse("oracle"), None);
    }

    #[test]
    fn mot_de_passe_avec_caracteres_speciaux_encode() {
        let url = info("a@b:c/d").to_url().unwrap();
        assert!(url.contains("hub:a%40b%3Ac%2Fd@db.exemple.org"), "{url}");
    }

    #[test]
    fn champs_manquants_refuses_sans_citer_le_mot_de_passe() {
        let mut missing = info("secret");
        missing.host = " ".to_string();
        let error = missing.to_url().unwrap_err();
        assert!(!error.contains("secret"));
        let mut bad_port = info("secret");
        bad_port.port = "99999".to_string();
        assert!(bad_port.to_url().is_err());
    }

    #[test]
    fn fichier_ecrit_puis_relu() {
        let dir = std::env::temp_dir().join(format!("rafthub-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(ServerConfig::load(&dir).unwrap(), None);
        let config = ServerConfig::new(sqlite_url(&dir));
        config.save(&dir).unwrap();
        assert_eq!(ServerConfig::load(&dir).unwrap(), Some(config));
        // Base intégrée : chemin relatif, complété par le dossier de données.
        let embedded = ServerConfig::new(EMBEDDED_URL.to_string());
        assert_eq!(embedded.database_url_in(&dir), sqlite_url(&dir));
        let pg = ServerConfig::new("postgres://h/b".to_string());
        assert_eq!(pg.database_url_in(&dir), "postgres://h/b");
        std::fs::write(
            dir.join(FILE_NAME),
            "format_version = 9\ndatabase_url = \"x\"\n",
        )
        .unwrap();
        assert!(ServerConfig::load(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
