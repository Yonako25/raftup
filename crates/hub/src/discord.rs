//! Connexion par Discord (OAuth 2.0 avec PKCE), liée à un compte du hub (ADR 0002, 0033).
//!
//! Parcours, vu du launcher :
//! 1. `POST /v1/oauth/discord/start` : le hub renvoie l'adresse de Discord à ouvrir
//!    dans le navigateur, un `state` et un `secret` que seul le launcher connaît.
//! 2. Le joueur accepte chez Discord, qui renvoie le navigateur vers le hub
//!    (`/v1/oauth/discord/callback`). Le hub échange le code contre l'identité
//!    Discord, puis renvoie le navigateur vers le launcher (port local ou lien
//!    `raftup://`) avec un `ticket`.
//! 3. `POST /v1/oauth/discord/finish` (`state`, `secret`, `ticket`) : session ouverte,
//!    compte lié, ou pseudo à choisir (`/v1/oauth/discord/register`).
//!
//! Il faut le `secret` (gardé par le launcher) **et** le `ticket` (arrivé dans le
//! navigateur qui a dit oui à Discord, sur la même machine) : quelqu'un qui enverrait
//! son lien d'autorisation à une victime n'obtiendrait pas sa session.
//!
//! Le hub ne parle à Discord qu'à travers le trait [`DiscordApi`] : [`HttpDiscord`]
//! pour de vrai, [`FakeDiscord`] dans les tests.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Identité Discord, réduite au minimum (portée OAuth `identify`, sans l'e-mail).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscordUser {
    /// Identifiant Discord (« snowflake »), stable même si le joueur change de nom.
    pub id: String,
    pub username: String,
    /// Nom affiché choisi par le joueur, s'il en a un.
    pub global_name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscordError {
    #[error("Discord a refusé le code d'autorisation")]
    Refused,
    #[error("Discord ne répond pas ou répond mal")]
    Unavailable,
}

/// Ce que le hub demande à Discord.
///
/// Synchrone (le client HTTP bloque le thread) : le hub l'appelle sur un thread à
/// part (`spawn_blocking`). `Send + Sync` : partagé entre les threads du serveur.
pub trait DiscordApi: Send + Sync {
    /// Adresse de la page d'autorisation de Discord.
    fn authorize_url(&self, state: &str, code_challenge: &str) -> String;
    /// Échange le code reçu contre l'identité du joueur.
    fn identify(&self, code: &str, code_verifier: &str) -> Result<DiscordUser, DiscordError>;
}

/// Réglages de l'application Discord (portail développeur de Discord).
#[derive(Clone)]
pub struct DiscordConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Adresse de retour enregistrée chez Discord :
    /// `<adresse publique du hub>/v1/oauth/discord/callback`.
    pub redirect_uri: String,
}

/// `Debug` écrit à la main : le secret de l'application n'apparaît jamais.
impl std::fmt::Debug for DiscordConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordConfig")
            .field("client_id", &self.client_id)
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

const DISCORD_AUTHORIZE: &str = "https://discord.com/oauth2/authorize";
const DISCORD_TOKEN: &str = "https://discord.com/api/v10/oauth2/token";
const DISCORD_ME: &str = "https://discord.com/api/v10/users/@me";

/// Vrai client, en HTTPS (crate `ureq`).
pub struct HttpDiscord {
    config: DiscordConfig,
    agent: ureq::Agent,
}

impl HttpDiscord {
    pub fn new(config: DiscordConfig) -> HttpDiscord {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(true)
            .http_status_as_error(true)
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .into();
        HttpDiscord { config, agent }
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct MeResponse {
    id: String,
    username: String,
    #[serde(default)]
    global_name: Option<String>,
}

impl DiscordApi for HttpDiscord {
    fn authorize_url(&self, state: &str, code_challenge: &str) -> String {
        authorize_url(&self.config, state, code_challenge)
    }

    fn identify(&self, code: &str, code_verifier: &str) -> Result<DiscordUser, DiscordError> {
        // 1. Le code (et le vérificateur PKCE) contre un jeton d'accès.
        let response = self
            .agent
            .post(DISCORD_TOKEN)
            .send_form([
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", self.config.redirect_uri.as_str()),
                ("code_verifier", code_verifier),
                ("client_id", self.config.client_id.as_str()),
                ("client_secret", self.config.client_secret.as_str()),
            ])
            .map_err(|error| match error {
                // 400/401 : code faux, expiré ou déjà servi.
                ureq::Error::StatusCode(400 | 401) => DiscordError::Refused,
                _ => DiscordError::Unavailable,
            })?;
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|_| DiscordError::Unavailable)?;
        let token: TokenResponse =
            serde_json::from_str(&text).map_err(|_| DiscordError::Unavailable)?;

        // 2. Le jeton d'accès contre l'identité. Il ne sert qu'à cette lecture et
        // n'est jamais gardé.
        let response = self
            .agent
            .get(DISCORD_ME)
            .header("Authorization", format!("Bearer {}", token.access_token))
            .call()
            .map_err(|_| DiscordError::Unavailable)?;
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|_| DiscordError::Unavailable)?;
        let me: MeResponse = serde_json::from_str(&text).map_err(|_| DiscordError::Unavailable)?;
        Ok(DiscordUser {
            id: me.id,
            username: me.username,
            global_name: me.global_name.filter(|name| !name.trim().is_empty()),
        })
    }
}

/// Adresse d'autorisation, avec les paramètres encodés (`url` s'en charge).
pub fn authorize_url(config: &DiscordConfig, state: &str, code_challenge: &str) -> String {
    match url::Url::parse_with_params(
        DISCORD_AUTHORIZE,
        [
            ("response_type", "code"),
            ("client_id", config.client_id.as_str()),
            ("scope", "identify"),
            ("redirect_uri", config.redirect_uri.as_str()),
            ("state", state),
            ("code_challenge", code_challenge),
            ("code_challenge_method", "S256"),
        ],
    ) {
        Ok(url) => url.into(),
        // L'adresse de base est une constante valide : ce cas n'arrive pas.
        Err(_) => DISCORD_AUTHORIZE.to_string(),
    }
}

/// PKCE (RFC 7636), méthode `S256` : le défi est l'empreinte SHA-256 du
/// vérificateur, en base64 « URL » sans `=`.
pub fn code_challenge(code_verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()))
}

/// Faux Discord pour les tests : les codes acceptés sont donnés à l'avance.
#[derive(Default)]
pub struct FakeDiscord {
    codes: Mutex<BTreeMap<String, DiscordUser>>,
    last_verifier: Mutex<Option<String>>,
}

impl FakeDiscord {
    pub fn new() -> FakeDiscord {
        FakeDiscord::default()
    }

    /// Le prochain échange avec `code` renverra `user` (une seule fois).
    pub fn accept(&self, code: &str, user: DiscordUser) {
        lock(&self.codes).insert(code.to_string(), user);
    }

    /// Vérificateur PKCE reçu au dernier échange.
    pub fn last_verifier(&self) -> Option<String> {
        lock(&self.last_verifier).clone()
    }
}

impl DiscordApi for FakeDiscord {
    fn authorize_url(&self, state: &str, code_challenge: &str) -> String {
        let config = DiscordConfig {
            client_id: "faux".to_string(),
            client_secret: String::new(),
            redirect_uri: "http://hub.test/v1/oauth/discord/callback".to_string(),
        };
        authorize_url(&config, state, code_challenge)
    }

    fn identify(&self, code: &str, code_verifier: &str) -> Result<DiscordUser, DiscordError> {
        *lock(&self.last_verifier) = Some(code_verifier.to_string());
        lock(&self.codes).remove(code).ok_or(DiscordError::Refused)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---------- Connexions en attente (mémoire) ----------

/// Où renvoyer le navigateur après Discord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturnTo {
    /// Petit serveur local du launcher : `http://127.0.0.1:<port>/discord`.
    Loopback(u16),
    /// Lien profond `raftup://oauth/discord`.
    DeepLink,
}

impl ReturnTo {
    /// Adresse de retour avec `state` et `ticket`.
    pub fn url(self, state: &str, ticket: &str) -> String {
        let base = match self {
            ReturnTo::Loopback(port) => format!("http://127.0.0.1:{port}/discord"),
            ReturnTo::DeepLink => "raftup://oauth/discord".to_string(),
        };
        // `state` et `ticket` sont en hexadécimal : rien à encoder.
        format!("{base}?state={state}&ticket={ticket}")
    }
}

/// Résultat de l'aller-retour chez Discord.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Identified(DiscordUser),
    /// Le joueur a refusé chez Discord, ou le code n'a pas été accepté.
    Refused,
    /// Discord n'a pas répondu.
    Unavailable,
}

/// Une connexion commencée et pas encore finie.
#[derive(Clone, Debug)]
pub struct Pending {
    /// Empreinte du `secret` donné au launcher.
    pub secret_hash: String,
    /// Vérificateur PKCE : reste sur le hub, seul son défi part chez Discord.
    pub code_verifier: String,
    /// Compte à lier (le joueur était connecté au départ), sinon connexion.
    pub link_user: Option<i64>,
    pub return_to: ReturnTo,
    pub created: Instant,
    /// Rempli au retour de Discord, avec l'empreinte du `ticket`.
    pub outcome: Option<(Outcome, String)>,
}

/// Connexions en attente, rangées par `state`. Elles expirent après `ttl`.
pub struct PendingLogins {
    ttl: Duration,
    max: usize,
    entries: Mutex<BTreeMap<String, Pending>>,
}

impl PendingLogins {
    pub fn new(ttl: Duration, max: usize) -> Self {
        PendingLogins {
            ttl,
            max,
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Ajoute une connexion ; `false` s'il y en a déjà trop (après ménage).
    pub fn insert(&self, state: String, pending: Pending) -> bool {
        let mut entries = lock(&self.entries);
        if entries.len() >= self.max {
            let ttl = self.ttl;
            entries.retain(|_, p| p.created.elapsed() < ttl);
        }
        if entries.len() >= self.max {
            return false;
        }
        entries.insert(state, pending);
        true
    }

    /// Copie d'une connexion encore valide.
    pub fn get(&self, state: &str) -> Option<Pending> {
        let mut entries = lock(&self.entries);
        match entries.get(state) {
            Some(p) if p.created.elapsed() < self.ttl => Some(p.clone()),
            Some(_) => {
                entries.remove(state);
                None
            }
            None => None,
        }
    }

    /// Inscrit le retour de Discord, une seule fois. `false` si la connexion est
    /// inconnue, expirée ou déjà revenue.
    pub fn set_outcome(&self, state: &str, outcome: Outcome, ticket_hash: String) -> bool {
        let mut entries = lock(&self.entries);
        match entries.get_mut(state) {
            Some(p) if p.created.elapsed() < self.ttl && p.outcome.is_none() => {
                p.outcome = Some((outcome, ticket_hash));
                true
            }
            _ => false,
        }
    }

    pub fn remove(&self, state: &str) {
        lock(&self.entries).remove(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defi_pkce_de_la_rfc_7636() {
        // Exemple de l'annexe B de la RFC 7636.
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn adresse_d_autorisation_encodee() {
        let config = DiscordConfig {
            client_id: "123".to_string(),
            client_secret: "chut".to_string(),
            redirect_uri: "https://hub.example.org/v1/oauth/discord/callback".to_string(),
        };
        let url = authorize_url(&config, "etat", "defi");
        assert!(url.starts_with("https://discord.com/oauth2/authorize?"));
        assert!(url.contains("client_id=123"));
        assert!(url.contains("scope=identify"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fhub.example.org%2Fv1%2Foauth"));
        assert!(!url.contains("chut"));
        assert!(!format!("{config:?}").contains("chut"));
    }

    #[test]
    fn connexions_en_attente_expirent_et_reviennent_une_fois() {
        let logins = PendingLogins::new(Duration::from_secs(60), 2);
        let pending = Pending {
            secret_hash: "s".to_string(),
            code_verifier: "v".to_string(),
            link_user: None,
            return_to: ReturnTo::DeepLink,
            created: Instant::now(),
            outcome: None,
        };
        assert!(logins.insert("a".to_string(), pending.clone()));
        assert!(logins.insert("b".to_string(), pending.clone()));
        assert!(!logins.insert("c".to_string(), pending.clone()));
        assert!(logins.set_outcome("a", Outcome::Refused, "t".to_string()));
        assert!(!logins.set_outcome("a", Outcome::Refused, "t".to_string()));
        assert!(!logins.set_outcome("inconnu", Outcome::Refused, "t".to_string()));

        let expired = PendingLogins::new(Duration::ZERO, 2);
        assert!(expired.insert("a".to_string(), pending));
        assert!(expired.get("a").is_none());
    }
}
