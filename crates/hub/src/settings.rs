//! Réglages du hub modifiables depuis la console d'admin (ADR 0036).
//!
//! Chaque réglage a trois sources possibles, lues dans cet ordre :
//! 1. la **base** (table `settings`, écrite par la console) ;
//! 2. l'**environnement** du process (variables `RAFTUP_HUB_…` de l'ADR 0033, ou
//!    options de la ligne de commande), rangé dans [`EnvSettings`] au démarrage ;
//! 3. une valeur **par défaut**.
//!
//! Les réglages sont relus à chaque usage (envoi d'un e-mail, connexion Discord,
//! inscription) : un changement fait dans la console vaut tout de suite, sans
//! redémarrer le hub.
//!
//! Les secrets (adresse SMTP avec son mot de passe, secret Discord) s'écrivent mais
//! ne se relisent jamais vers l'extérieur : [`PublicSettings`] dit seulement s'ils
//! sont configurés.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

use crate::discord::DiscordConfig;
use crate::error::HubError;
use crate::mail::{self, SmtpMailer};

/// Nom du hub par défaut.
pub const DEFAULT_HUB_NAME: &str = "Rafthub";
/// Longueur maximale d'une valeur de réglage.
const MAX_VALUE_CHARS: usize = 512;

/// Mode d'inscription des nouveaux joueurs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationMode {
    /// Tout le monde peut créer un compte.
    #[default]
    Open,
    /// Il faut un code d'invitation créé dans la console.
    Invite,
    /// Plus aucune inscription.
    Closed,
}

impl RegistrationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RegistrationMode::Open => "open",
            RegistrationMode::Invite => "invite",
            RegistrationMode::Closed => "closed",
        }
    }

    pub fn parse(value: &str) -> Option<RegistrationMode> {
        match value {
            "open" => Some(RegistrationMode::Open),
            "invite" => Some(RegistrationMode::Invite),
            "closed" => Some(RegistrationMode::Closed),
            _ => None,
        }
    }
}

/// Les réglages connus. Le nom (`name`) est la clé de la table `settings`.
///
/// `Ord` : la clé sert d'index dans un `BTreeMap` (ordre stable).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SettingKey {
    HubName,
    PublicUrl,
    RegistrationMode,
    /// `smtps://utilisateur:motdepasse@hote:465` : contient le mot de passe, donc secret.
    SmtpUrl,
    MailFrom,
    DiscordClientId,
    DiscordClientSecret,
    /// Application Twitch qui donne accès à IGDB (illustrations des jeux, ADR 0047).
    IgdbClientId,
    IgdbClientSecret,
    /// Page publique `/telecharger` active (`true` / `false`).
    PublicDownload,
    /// Parties hébergées par le hub (ADR 0052) : `true` / `false`, éteint par défaut.
    Hosting,
}

impl SettingKey {
    /// Tous les réglages, dans l'ordre d'affichage.
    pub const ALL: [SettingKey; 11] = [
        SettingKey::HubName,
        SettingKey::PublicUrl,
        SettingKey::RegistrationMode,
        SettingKey::SmtpUrl,
        SettingKey::MailFrom,
        SettingKey::DiscordClientId,
        SettingKey::DiscordClientSecret,
        SettingKey::IgdbClientId,
        SettingKey::IgdbClientSecret,
        SettingKey::PublicDownload,
        SettingKey::Hosting,
    ];

    /// Nom en base (et dans les formulaires de la console).
    pub fn name(self) -> &'static str {
        match self {
            SettingKey::HubName => "hub_name",
            SettingKey::PublicUrl => "public_url",
            SettingKey::RegistrationMode => "registration_mode",
            SettingKey::SmtpUrl => "smtp_url",
            SettingKey::MailFrom => "mail_from",
            SettingKey::DiscordClientId => "discord_client_id",
            SettingKey::DiscordClientSecret => "discord_client_secret",
            SettingKey::IgdbClientId => "igdb_client_id",
            SettingKey::IgdbClientSecret => "igdb_client_secret",
            SettingKey::PublicDownload => "public_download",
            SettingKey::Hosting => "hosting",
        }
    }

    /// Variable d'environnement équivalente (ADR 0033).
    pub fn env_var(self) -> &'static str {
        match self {
            SettingKey::HubName => "RAFTUP_HUB_NAME",
            SettingKey::PublicUrl => "RAFTUP_HUB_PUBLIC_URL",
            SettingKey::RegistrationMode => "RAFTUP_HUB_REGISTRATION",
            SettingKey::SmtpUrl => "RAFTUP_HUB_SMTP_URL",
            SettingKey::MailFrom => "RAFTUP_HUB_MAIL_FROM",
            SettingKey::DiscordClientId => "RAFTUP_HUB_DISCORD_CLIENT_ID",
            SettingKey::DiscordClientSecret => "RAFTUP_HUB_DISCORD_CLIENT_SECRET",
            SettingKey::IgdbClientId => "RAFTUP_HUB_IGDB_CLIENT_ID",
            SettingKey::IgdbClientSecret => "RAFTUP_HUB_IGDB_CLIENT_SECRET",
            SettingKey::PublicDownload => "RAFTUP_HUB_PUBLIC_DOWNLOAD",
            SettingKey::Hosting => "RAFTUP_HUB_HOSTING",
        }
    }

    pub fn from_name(name: &str) -> Option<SettingKey> {
        SettingKey::ALL.into_iter().find(|key| key.name() == name)
    }

    /// Valeur jamais réaffichée.
    pub fn is_secret(self) -> bool {
        matches!(
            self,
            SettingKey::SmtpUrl | SettingKey::DiscordClientSecret | SettingKey::IgdbClientSecret
        )
    }

    /// Vérifie une valeur et renvoie sa forme rangée (espaces retirés…).
    /// Le message d'erreur ne recopie jamais la valeur (elle peut être un secret).
    pub fn validate(self, raw: &str) -> Result<String, HubError> {
        let value = raw.trim();
        if value.is_empty() {
            return Err(HubError::InvalidSetting("valeur vide"));
        }
        if value.chars().count() > MAX_VALUE_CHARS || value.chars().any(char::is_control) {
            return Err(HubError::InvalidSetting(
                "valeur trop longue ou caractère de contrôle",
            ));
        }
        match self {
            SettingKey::HubName => {
                if value.chars().count() > 64 {
                    return Err(HubError::InvalidSetting(
                        "nom du hub : 64 caractères au plus",
                    ));
                }
                Ok(value.to_string())
            }
            SettingKey::PublicUrl => {
                let parsed = url::Url::parse(value)
                    .map_err(|_| HubError::InvalidSetting("adresse publique illisible"))?;
                let scheme_ok = matches!(parsed.scheme(), "http" | "https");
                if !scheme_ok || parsed.host_str().is_none() || parsed.query().is_some() {
                    return Err(HubError::InvalidSetting(
                        "adresse publique : http(s)://hôte, sans paramètres",
                    ));
                }
                // Rangée sans `/` final : les liens s'écrivent `<adresse>/v1/…`.
                Ok(value.trim_end_matches('/').to_string())
            }
            SettingKey::RegistrationMode => RegistrationMode::parse(value)
                .map(|mode| mode.as_str().to_string())
                .ok_or(HubError::InvalidSetting(
                    "mode d'inscription : open, invite ou closed",
                )),
            SettingKey::SmtpUrl => {
                if mail::valid_smtp_url(value) {
                    Ok(value.to_string())
                } else {
                    Err(HubError::InvalidSetting(
                        "adresse SMTP : smtps://utilisateur:motdepasse@hote:465",
                    ))
                }
            }
            SettingKey::MailFrom => {
                if mail::valid_mailbox(value) {
                    Ok(value.to_string())
                } else {
                    Err(HubError::InvalidSetting(
                        "expéditeur : Nom <adresse@exemple.org>",
                    ))
                }
            }
            SettingKey::DiscordClientId => {
                if value.len() <= 32 && value.chars().all(|c| c.is_ascii_digit()) {
                    Ok(value.to_string())
                } else {
                    Err(HubError::InvalidSetting(
                        "identifiant Discord : seulement des chiffres",
                    ))
                }
            }
            SettingKey::DiscordClientSecret => {
                if value.len() <= 256 && !value.chars().any(char::is_whitespace) {
                    Ok(value.to_string())
                } else {
                    Err(HubError::InvalidSetting("secret Discord illisible"))
                }
            }
            // Identifiant et secret Twitch : lettres et chiffres ASCII (30 caractères
            // chez Twitch) ; on laisse de la marge sans accepter d'espaces ni de `&`.
            SettingKey::IgdbClientId | SettingKey::IgdbClientSecret => {
                if value.len() <= 128 && value.chars().all(|c| c.is_ascii_alphanumeric()) {
                    Ok(value.to_string())
                } else {
                    Err(HubError::InvalidSetting(
                        "identifiant ou secret Twitch : lettres et chiffres seulement",
                    ))
                }
            }
            SettingKey::PublicDownload => match value {
                "true" | "false" => Ok(value.to_string()),
                _ => Err(HubError::InvalidSetting("page publique : true ou false")),
            },
            SettingKey::Hosting => match value {
                "true" | "false" => Ok(value.to_string()),
                _ => Err(HubError::InvalidSetting(
                    "parties hébergées : true ou false",
                )),
            },
        }
    }
}

/// Réglages venus de l'environnement (variables ou ligne de commande), lus une fois
/// au démarrage. Vide par défaut (tests).
#[derive(Clone, Default)]
pub struct EnvSettings {
    values: BTreeMap<SettingKey, String>,
}

impl EnvSettings {
    /// Lit les variables `RAFTUP_HUB_…` du process. Une valeur invalide est
    /// ignorée avec un avertissement (sans la recopier).
    pub fn from_env() -> EnvSettings {
        EnvSettings::from_lookup(|name| std::env::var(name).ok())
    }

    /// Comme [`EnvSettings::from_env`], avec une fonction de lecture choisie (tests).
    ///
    /// `impl Fn(&str) -> Option<String>` : n'importe quelle fonction ou fermeture qui
    /// prend un nom de variable et rend peut-être sa valeur.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> EnvSettings {
        let mut env = EnvSettings::default();
        for key in SettingKey::ALL {
            let Some(raw) = lookup(key.env_var()) else {
                continue;
            };
            if raw.trim().is_empty() {
                continue;
            }
            if let Err(error) = env.set(key, &raw) {
                tracing::warn!(variable = key.env_var(), %error, "variable ignorée");
            }
        }
        env
    }

    /// Fixe une valeur (vérifiée), par exemple depuis `--public-url`.
    pub fn set(&mut self, key: SettingKey, raw: &str) -> Result<(), HubError> {
        let value = key.validate(raw)?;
        self.values.insert(key, value);
        Ok(())
    }

    fn get(&self, key: SettingKey) -> Option<&str> {
        self.values.get(&key).map(String::as_str)
    }
}

/// `Debug` écrit à la main : seulement les noms des réglages présents, jamais les
/// valeurs (secrets).
impl fmt::Debug for EnvSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.values.keys().map(|key| key.name()))
            .finish()
    }
}

/// D'où vient la valeur d'un réglage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingSource {
    Database,
    Environment,
    Default,
}

/// Réglages résolus, avec les secrets : pour l'usage interne du hub seulement.
#[derive(Clone, PartialEq, Eq)]
pub struct Settings {
    pub hub_name: String,
    /// Sans `/` final.
    pub public_url: String,
    pub registration_mode: RegistrationMode,
    pub smtp_url: Option<String>,
    pub mail_from: Option<String>,
    pub discord_client_id: Option<String>,
    pub discord_client_secret: Option<String>,
    pub igdb_client_id: Option<String>,
    pub igdb_client_secret: Option<String>,
    pub public_download: bool,
    /// Parties hébergées par le hub permises (ADR 0052).
    pub hosting: bool,
    /// Source de chaque réglage (pour la console).
    sources: BTreeMap<SettingKey, SettingSource>,
}

/// `Debug` sans les secrets.
impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Settings")
            .field("hub_name", &self.hub_name)
            .field("public_url", &self.public_url)
            .field("registration_mode", &self.registration_mode)
            .field("smtp_configured", &self.smtp_url.is_some())
            .field("discord_client_id", &self.discord_client_id)
            .field("igdb_client_id", &self.igdb_client_id)
            .field("public_download", &self.public_download)
            .field("hosting", &self.hosting)
            .finish_non_exhaustive()
    }
}

impl Settings {
    /// Résout chaque réglage : base, sinon environnement, sinon défaut.
    ///
    /// `default_public_url` : adresse par défaut (celle de [`crate::HubConfig`]).
    /// Une valeur invalide en base (écrite à la main ?) est sautée, avec un
    /// avertissement : on passe à la source suivante.
    pub fn resolve(
        database: &BTreeMap<String, String>,
        env: &EnvSettings,
        default_public_url: &str,
    ) -> Settings {
        let mut sources = BTreeMap::new();
        // Fermeture qui renvoie la valeur retenue pour une clé, et note sa source.
        let mut pick = |key: SettingKey| -> Option<String> {
            if let Some(raw) = database.get(key.name()) {
                match key.validate(raw) {
                    Ok(value) => {
                        sources.insert(key, SettingSource::Database);
                        return Some(value);
                    }
                    Err(error) => {
                        tracing::warn!(setting = key.name(), %error, "réglage en base ignoré")
                    }
                }
            }
            if let Some(value) = env.get(key) {
                sources.insert(key, SettingSource::Environment);
                return Some(value.to_string());
            }
            sources.insert(key, SettingSource::Default);
            None
        };

        let hub_name = pick(SettingKey::HubName).unwrap_or_else(|| DEFAULT_HUB_NAME.to_string());
        let public_url = pick(SettingKey::PublicUrl)
            .unwrap_or_else(|| default_public_url.trim_end_matches('/').to_string());
        let registration_mode = pick(SettingKey::RegistrationMode)
            .and_then(|v| RegistrationMode::parse(&v))
            .unwrap_or_default();
        let smtp_url = pick(SettingKey::SmtpUrl);
        let mail_from = pick(SettingKey::MailFrom);
        let discord_client_id = pick(SettingKey::DiscordClientId);
        let discord_client_secret = pick(SettingKey::DiscordClientSecret);
        let igdb_client_id = pick(SettingKey::IgdbClientId);
        let igdb_client_secret = pick(SettingKey::IgdbClientSecret);
        // Page publique active par défaut (ADR 0036 : « désactivable »).
        let public_download = pick(SettingKey::PublicDownload).is_none_or(|v| v == "true");
        // Parties hébergées éteintes par défaut : héberger des parties consomme du
        // CPU, de la mémoire et une plage de ports à ouvrir (ADR 0052).
        let hosting = pick(SettingKey::Hosting).is_some_and(|v| v == "true");

        Settings {
            hub_name,
            public_url,
            registration_mode,
            smtp_url,
            mail_from,
            discord_client_id,
            discord_client_secret,
            igdb_client_id,
            igdb_client_secret,
            public_download,
            hosting,
            sources,
        }
    }

    /// Source d'un réglage.
    pub fn source(&self, key: SettingKey) -> SettingSource {
        self.sources
            .get(&key)
            .copied()
            .unwrap_or(SettingSource::Default)
    }

    /// Service SMTP, si l'adresse et l'expéditeur sont réglés. Un réglage refusé par
    /// `lettre` donne `None` et un avertissement (sans l'adresse, qui a un mot de passe).
    pub fn smtp_mailer(&self) -> Option<SmtpMailer> {
        let (Some(url), Some(from)) = (&self.smtp_url, &self.mail_from) else {
            return None;
        };
        match SmtpMailer::from_url(url, from) {
            Ok(mailer) => Some(mailer),
            Err(error) => {
                tracing::warn!(%error, "réglage SMTP refusé : aucun e-mail ne partira");
                None
            }
        }
    }

    /// Application Discord, si l'identifiant et le secret sont réglés.
    pub fn discord_config(&self) -> Option<DiscordConfig> {
        let (Some(client_id), Some(client_secret)) =
            (&self.discord_client_id, &self.discord_client_secret)
        else {
            return None;
        };
        Some(DiscordConfig {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            redirect_uri: format!("{}/v1/oauth/discord/callback", self.public_url),
        })
    }

    /// Application Twitch pour IGDB (ADR 0047), si l'identifiant et le secret sont réglés.
    pub fn igdb_credentials(&self) -> Option<crate::art::IgdbCredentials> {
        let (Some(client_id), Some(client_secret)) =
            (&self.igdb_client_id, &self.igdb_client_secret)
        else {
            return None;
        };
        Some(crate::art::IgdbCredentials {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
        })
    }

    /// Vue sans secret, pour la console.
    pub fn public(&self) -> PublicSettings {
        let entries = SettingKey::ALL
            .into_iter()
            .map(|key| {
                let value = match key {
                    SettingKey::HubName => Some(self.hub_name.clone()),
                    SettingKey::PublicUrl => Some(self.public_url.clone()),
                    SettingKey::RegistrationMode => {
                        Some(self.registration_mode.as_str().to_string())
                    }
                    SettingKey::SmtpUrl
                    | SettingKey::DiscordClientSecret
                    | SettingKey::IgdbClientSecret => None,
                    SettingKey::MailFrom => self.mail_from.clone(),
                    SettingKey::DiscordClientId => self.discord_client_id.clone(),
                    SettingKey::IgdbClientId => self.igdb_client_id.clone(),
                    SettingKey::PublicDownload => Some(self.public_download.to_string()),
                    SettingKey::Hosting => Some(self.hosting.to_string()),
                };
                let configured = match key {
                    SettingKey::SmtpUrl => self.smtp_url.is_some(),
                    SettingKey::DiscordClientSecret => self.discord_client_secret.is_some(),
                    SettingKey::IgdbClientSecret => self.igdb_client_secret.is_some(),
                    _ => self.source(key) != SettingSource::Default,
                };
                PublicSetting {
                    name: key.name(),
                    secret: key.is_secret(),
                    value,
                    configured,
                    source: self.source(key),
                }
            })
            .collect();
        PublicSettings {
            hub_name: self.hub_name.clone(),
            public_url: self.public_url.clone(),
            registration_mode: self.registration_mode,
            smtp_configured: self.smtp_url.is_some() && self.mail_from.is_some(),
            smtp_host: self.smtp_url.as_deref().and_then(smtp_host),
            discord_configured: self.discord_config().is_some(),
            public_download: self.public_download,
            entries,
        }
    }
}

/// Nom d'hôte d'une adresse SMTP, sans l'utilisateur ni le mot de passe.
fn smtp_host(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
}

/// Un réglage vu par la console : jamais la valeur d'un secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PublicSetting {
    pub name: &'static str,
    pub secret: bool,
    /// `None` pour un secret, ou pour un réglage vide.
    pub value: Option<String>,
    /// Réglé ailleurs que par le défaut (pour un secret : présent).
    pub configured: bool,
    pub source: SettingSource,
}

/// Réglages sans secret : ce que la console peut afficher.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PublicSettings {
    pub hub_name: String,
    pub public_url: String,
    pub registration_mode: RegistrationMode,
    /// Adresse SMTP **et** expéditeur réglés.
    pub smtp_configured: bool,
    /// Hôte du serveur SMTP (sans identifiants), s'il est réglé.
    pub smtp_host: Option<String>,
    pub discord_configured: bool,
    pub public_download: bool,
    /// Chaque réglage, dans l'ordre d'affichage.
    pub entries: Vec<PublicSetting>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMTP: &str = "smtps://moi:motdepasse-secret@smtp.exemple.org:465";

    fn db(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn priorite_base_puis_environnement_puis_defaut() {
        let env = EnvSettings::from_lookup(|name| match name {
            "RAFTUP_HUB_NAME" => Some("Hub de l'environnement".to_string()),
            "RAFTUP_HUB_REGISTRATION" => Some("invite".to_string()),
            _ => None,
        });
        let database = db(&[("registration_mode", "closed")]);
        let settings = Settings::resolve(&database, &env, "http://127.0.0.1:8787/");
        assert_eq!(settings.registration_mode, RegistrationMode::Closed);
        assert_eq!(
            settings.source(SettingKey::RegistrationMode),
            SettingSource::Database
        );
        assert_eq!(settings.hub_name, "Hub de l'environnement");
        assert_eq!(
            settings.source(SettingKey::HubName),
            SettingSource::Environment
        );
        assert_eq!(settings.public_url, "http://127.0.0.1:8787");
        assert_eq!(
            settings.source(SettingKey::PublicUrl),
            SettingSource::Default
        );
        assert!(settings.public_download);
        assert!(!settings.hosting);
    }

    #[test]
    fn hebergement_regle_par_l_environnement() {
        let env = EnvSettings::from_lookup(|name| {
            (name == "RAFTUP_HUB_HOSTING").then(|| "true".to_string())
        });
        let settings = Settings::resolve(&BTreeMap::new(), &env, "http://h");
        assert!(settings.hosting);
        assert!(SettingKey::Hosting.validate("oui").is_err());
    }

    #[test]
    fn valeur_invalide_en_base_sautee() {
        let env = EnvSettings::default();
        let database = db(&[("registration_mode", "nimporte"), ("public_download", "x")]);
        let settings = Settings::resolve(&database, &env, "http://h");
        assert_eq!(settings.registration_mode, RegistrationMode::Open);
        assert!(settings.public_download);
    }

    #[test]
    fn variable_invalide_ignoree() {
        let env = EnvSettings::from_lookup(|name| {
            (name == "RAFTUP_HUB_PUBLIC_URL").then(|| "ftp://x".to_string())
        });
        assert!(env.get(SettingKey::PublicUrl).is_none());
    }

    #[test]
    fn secrets_jamais_dans_la_vue_publique_ni_debug() {
        let database = db(&[
            ("smtp_url", SMTP),
            ("mail_from", "Rafthub <no-reply@exemple.org>"),
            ("discord_client_id", "123456"),
            ("discord_client_secret", "secret-discord-xyz"),
        ]);
        let settings = Settings::resolve(&database, &EnvSettings::default(), "http://h");
        let public = settings.public();
        let json = serde_json::to_string(&public).unwrap_or_default();
        assert!(!json.contains("motdepasse-secret"));
        assert!(!json.contains("secret-discord-xyz"));
        assert!(public.smtp_configured);
        assert!(public.discord_configured);
        assert_eq!(public.smtp_host.as_deref(), Some("smtp.exemple.org"));
        let secret_entry = public
            .entries
            .iter()
            .find(|e| e.name == "discord_client_secret");
        assert!(secret_entry.is_some_and(|e| e.configured && e.secret && e.value.is_none()));
        let debug = format!("{settings:?}");
        assert!(!debug.contains("motdepasse-secret") && !debug.contains("secret-discord-xyz"));
        let env = EnvSettings::from_lookup(|name| {
            (name == "RAFTUP_HUB_SMTP_URL").then(|| SMTP.to_string())
        });
        assert!(!format!("{env:?}").contains("motdepasse"));
    }

    #[test]
    fn services_construits_depuis_les_reglages() {
        let empty = Settings::resolve(&BTreeMap::new(), &EnvSettings::default(), "http://h");
        assert!(empty.smtp_mailer().is_none());
        assert!(empty.discord_config().is_none());
        let database = db(&[
            ("smtp_url", SMTP),
            ("mail_from", "Rafthub <no-reply@exemple.org>"),
            ("discord_client_id", "42"),
            ("discord_client_secret", "s"),
            ("public_url", "https://hub.exemple.org/"),
        ]);
        let settings = Settings::resolve(&database, &EnvSettings::default(), "http://h");
        assert!(settings.smtp_mailer().is_some());
        let discord = settings.discord_config();
        assert_eq!(
            discord.map(|d| d.redirect_uri),
            Some("https://hub.exemple.org/v1/oauth/discord/callback".to_string())
        );
    }

    #[test]
    fn validations() {
        assert!(SettingKey::PublicUrl
            .validate("https://hub.exemple.org")
            .is_ok());
        assert!(SettingKey::PublicUrl
            .validate("javascript:alert(1)")
            .is_err());
        assert!(SettingKey::RegistrationMode.validate(" invite ").is_ok());
        assert!(SettingKey::RegistrationMode.validate("ouvert").is_err());
        assert!(SettingKey::SmtpUrl.validate("pas une url").is_err());
        assert!(SettingKey::MailFrom.validate("pas d'arobase").is_err());
        assert!(SettingKey::DiscordClientId.validate("12a").is_err());
        assert!(SettingKey::PublicDownload.validate("oui").is_err());
        assert!(SettingKey::HubName.validate("   ").is_err());
        // Le message d'erreur ne recopie pas la valeur.
        let error = SettingKey::SmtpUrl
            .validate("motdepasse-secret")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(!error.contains("motdepasse-secret"));
        assert_eq!(SettingKey::from_name("smtp_url"), Some(SettingKey::SmtpUrl));
    }
}
