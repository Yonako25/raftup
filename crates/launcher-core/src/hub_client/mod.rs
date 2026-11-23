//! Client du hub `raftup-hub` : comptes, amis, présence (ADR 0002, 0033, 0035).
//!
//! Le hub est en AGPL (ADR 0003). Ce module ne dépend **pas** de sa crate : il parle
//! seulement à son API HTTP `/v1`, avec des types redéfinis ici ([`types`]).
//!
//! - [`HubUrl`] : adresse du hub, validée (HTTPS, ou HTTP seulement vers ce PC) ;
//! - [`HubClient`] : un appel par route de l'API, **bloquant** (`ureq`) : l'interface
//!   l'appelle depuis un thread à part, jamais depuis le thread du dessin ;
//! - [`token`] : où ranger le jeton de session (trousseau du système, ou mémoire) ;
//! - [`discord`] : connexion Discord (serveur local du retour, `start`, `finish`) ;
//! - [`games`] : parties hébergées par le hub (`/v1/games`, ADR 0052) ;
//! - [`HubClientError`] : erreurs typées, avec le code stable du hub quand il répond.
//!
//! Ni le mot de passe ni le jeton n'apparaissent dans un `Debug` ou un message d'erreur.

pub mod discord;
pub mod games;
pub mod token;
pub mod types;

use std::time::Duration;

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::de::DeserializeOwned;
use serde_json::json;

pub use discord::{DiscordOutcome, DiscordReturn, DiscordStart, DiscordTicket, DISCORD_WAIT};
pub use games::{CreatedGame, GameStatus, GameUpload, HostedGame, MAX_SEED_BYTES};
pub use token::{KeyringStore, MemoryStore, TokenStore, TokenStoreError};
pub use types::{
    Account, ApiInfo, ErrorCode, Friend, FriendRequests, HubInfo, Me, Password, Player, Presence,
    RequestOutcome, Session, SessionToken, Status, MAX_GAME_CHARS,
};

/// Adresse du hub par défaut (ADR 0035, 0040).
///
/// - Launcher des releases officielles : l'adresse du Rafthub officiel, donnée à la
///   compilation par la variable `RAFTUP_OFFICIAL_HUB_URL` (le workflow de release la
///   lit dans les variables du dépôt GitHub). Seul le nom de domaine est écrit, jamais
///   l'IP du serveur.
/// - Sinon (compilation locale) : le hub lancé sur ce PC.
///
/// Un `rafthub.toml` posé à côté de l'exe (Rafthub auto-hébergé) passe avant.
///
/// `option_env!` lit la variable **au moment de la compilation** (pas au lancement) ;
/// le `match` est permis dans une constante.
pub const DEFAULT_HUB_URL: &str = match option_env!("RAFTUP_OFFICIAL_HUB_URL") {
    Some(url) => url,
    None => "http://127.0.0.1:8787",
};

/// Vrai si ce launcher a été compilé avec l'adresse du Rafthub officiel
/// (`RAFTUP_OFFICIAL_HUB_URL`) : `DEFAULT_HUB_URL` est alors le hub officiel.
pub const HAS_OFFICIAL_HUB: bool = option_env!("RAFTUP_OFFICIAL_HUB_URL").is_some();

/// Version de l'API que ce client sait parler.
pub const SUPPORTED_API_VERSION: u32 = 1;

/// Taille maximale d'une réponse lue : les réponses du hub sont petites.
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

/// Durée maximale d'un appel, connexion comprise.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Délai de l'envoi d'une seed (`POST /v1/games`) : jusqu'à ~85 Mo en base64, à faire
/// monter sur une connexion de particulier.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Caractères encodés dans un pseudo placé dans le chemin : tout sauf lettres et chiffres
/// ASCII. `/`, `?`, `#`, `.` deviennent `%2F`, `%3F`… et ne changent jamais la route.
const PATH_SEGMENT: &AsciiSet = NON_ALPHANUMERIC;

/// Ce qui fait échouer un appel au hub.
#[derive(Debug, thiserror::Error)]
pub enum HubClientError {
    /// Adresse du hub refusée (la raison dit pourquoi).
    #[error("adresse du hub refusée : {0}")]
    InvalidUrl(&'static str),
    /// Pseudo vide : rien à envoyer.
    #[error("pseudo vide")]
    EmptyUsername,
    /// Aucune session : il faut d'abord se connecter.
    #[error("pas connecté au hub")]
    NotLoggedIn,
    /// Le hub ne répond pas (hors ligne, adresse fausse, délai dépassé…).
    #[error("hub injoignable : {0}")]
    Network(String),
    /// Le hub a répondu par une erreur de son API.
    #[error("{message}")]
    Api {
        status: u16,
        code: ErrorCode,
        message: String,
    },
    /// Réponse que ce client ne comprend pas (pas un hub, version trop récente…).
    #[error("réponse inattendue du hub (code {status}) : {detail}")]
    UnexpectedResponse { status: u16, detail: String },
    /// Le petit serveur local qui attend le retour de Discord n'a pas pu s'ouvrir.
    #[error("retour de Discord impossible à recevoir : {0}")]
    DiscordReturn(String),
    /// Le joueur n'a pas répondu chez Discord à temps.
    #[error("délai dépassé : aucune réponse de Discord")]
    DiscordTimeout,
    /// Le joueur a annulé la connexion Discord dans le launcher.
    #[error("connexion Discord annulée")]
    DiscordCancelled,
    /// Le trousseau du système a refusé de lire ou de ranger le jeton.
    #[error(transparent)]
    TokenStore(#[from] TokenStoreError),
    /// Fichier à envoyer illisible (seed ou sauvegarde).
    #[error("{what} illisible : {detail}")]
    File { what: &'static str, detail: String },
    /// Fichier à envoyer plus gros que ce que le hub accepte ([`MAX_SEED_BYTES`]).
    #[error("{what} trop grosse ({bytes} octets)")]
    TooLarge { what: &'static str, bytes: u64 },
    /// Identifiant de partie mal formé (32 chiffres hexadécimaux attendus).
    #[error("identifiant de partie refusé")]
    BadGameId,
}

impl HubClientError {
    /// Le code stable du hub, s'il a répondu par une erreur de son API.
    pub fn code(&self) -> Option<&ErrorCode> {
        match self {
            HubClientError::Api { code, .. } => Some(code),
            _ => None,
        }
    }

    /// Vrai si la session est absente, expirée ou révoquée : il faut se reconnecter.
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, HubClientError::NotLoggedIn) || self.code() == Some(&ErrorCode::Unauthorized)
    }
}

/// Adresse validée d'un hub, sans `/` final (`https://hub.example.org`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HubUrl(String);

impl HubUrl {
    /// Valide l'adresse.
    ///
    /// - `https://` vers n'importe quel hôte ;
    /// - `http://` seulement vers ce PC (`localhost`, `127.0.0.1`, `[::1]`) : ailleurs, le
    ///   mot de passe et le jeton passeraient en clair sur le réseau ;
    /// - un chemin est permis (hub derrière un proxy, `https://exemple.org/hub`), mais ni
    ///   `?`, ni `#`, ni identifiants (`user@hôte`), ni espaces.
    pub fn parse(text: &str) -> Result<HubUrl, HubClientError> {
        let text = text.trim().trim_end_matches('/');
        if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(HubClientError::InvalidUrl("espace ou caractère invisible"));
        }
        if text.contains(['?', '#']) {
            return Err(HubClientError::InvalidUrl(
                "« ? » et « # » ne sont pas permis",
            ));
        }
        let (secure, rest) = if let Some(rest) = text.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = text.strip_prefix("http://") {
            (false, rest)
        } else {
            return Err(HubClientError::InvalidUrl(
                "il faut une adresse qui commence par https://",
            ));
        };
        // L'hôte va jusqu'au premier `/` (le reste est le chemin).
        let authority = rest.split('/').next().unwrap_or_default();
        if authority.is_empty() {
            return Err(HubClientError::InvalidUrl("hôte manquant"));
        }
        if authority.contains('@') {
            return Err(HubClientError::InvalidUrl("identifiants dans l'adresse"));
        }
        if !secure && !is_loopback(authority) {
            return Err(HubClientError::InvalidUrl(
                "http:// seulement vers ce PC ; ailleurs, il faut https://",
            ));
        }
        Ok(HubUrl(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// L'hôte (avec son port éventuel), sans schéma ni chemin : `hub.example.org`.
    /// C'est ce que l'interface affiche pour dire sur quel Rafthub on est.
    pub fn host(&self) -> &str {
        // `parse` a vérifié le schéma : l'un des deux préfixes est toujours là.
        let rest = self
            .0
            .strip_prefix("https://")
            .or_else(|| self.0.strip_prefix("http://"))
            .unwrap_or(&self.0);
        rest.split('/').next().unwrap_or(rest)
    }

    /// Adresse complète d'une route (`path` commence par `/v1`).
    pub(crate) fn join(&self, path: &str) -> String {
        format!("{}{path}", self.0)
    }
}

impl std::fmt::Display for HubUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Vrai si `authority` (hôte et port éventuel) désigne ce PC.
fn is_loopback(authority: &str) -> bool {
    // `[::1]:8787` : l'hôte IPv6 est entre crochets, le port après.
    let host = if let Some(rest) = authority.strip_prefix('[') {
        match rest.split_once(']') {
            Some((host, after)) if after.is_empty() || after.starts_with(':') => host,
            _ => return false,
        }
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

/// Méthode HTTP d'un appel.
#[derive(Debug, Clone, Copy)]
enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// Client de l'API `/v1` d'un hub. Garde le jeton de la session ouverte.
///
/// `Debug` est dérivé sans risque : celui de [`SessionToken`] masque le jeton.
#[derive(Debug)]
pub struct HubClient {
    url: HubUrl,
    agent: ureq::Agent,
    token: Option<SessionToken>,
}

impl HubClient {
    /// Client sans session.
    pub fn new(url: HubUrl) -> HubClient {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            // On ne suit jamais une redirection : le jeton ne part pas ailleurs.
            .max_redirects(0)
            // Les codes 4xx / 5xx sont lus par nous (corps d'erreur JSON du hub).
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        HubClient {
            url,
            agent,
            token: None,
        }
    }

    /// Client qui reprend un jeton déjà connu (lu dans le trousseau).
    pub fn with_token(url: HubUrl, token: SessionToken) -> HubClient {
        let mut client = HubClient::new(url);
        client.token = Some(token);
        client
    }

    pub fn url(&self) -> &HubUrl {
        &self.url
    }

    pub fn token(&self) -> Option<&SessionToken> {
        self.token.as_ref()
    }

    pub fn is_logged_in(&self) -> bool {
        self.token.is_some()
    }

    // ---------- Comptes et sessions ----------

    /// `GET /v1` : vérifie que l'adresse est bien un hub, d'une version connue.
    pub fn api_info(&self) -> Result<ApiInfo, HubClientError> {
        let info: ApiInfo = self.call(Method::Get, "/v1", None, false)?;
        if info.service != "raftup-hub" {
            return Err(HubClientError::UnexpectedResponse {
                status: 200,
                detail: "ce serveur n'est pas un hub Raftup".to_string(),
            });
        }
        if info.api_version != SUPPORTED_API_VERSION {
            return Err(HubClientError::UnexpectedResponse {
                status: 200,
                detail: format!(
                    "API du hub en version {}, ce launcher parle la version {}",
                    info.api_version, SUPPORTED_API_VERSION
                ),
            });
        }
        Ok(info)
    }

    /// `GET /v1/info` : nom du hub, moyens de connexion proposés, mode d'inscription.
    ///
    /// Un hub plus ancien, sans cette route (404), est traité comme un hub qui ne
    /// propose que le mot de passe.
    pub fn hub_info(&self) -> Result<HubInfo, HubClientError> {
        match self.call::<HubInfo>(Method::Get, "/v1/info", None, false) {
            Ok(info) if info.service != "raftup-hub" => Err(unexpected(
                200,
                "ce serveur n'est pas un hub Raftup".to_string(),
            )),
            Ok(info) => Ok(info),
            Err(HubClientError::UnexpectedResponse { status: 404, .. }) => {
                Ok(HubInfo::password_only())
            }
            Err(other) => Err(other),
        }
    }

    // ---------- Connexion Discord (voir le module `discord`) ----------

    /// `POST /v1/oauth/discord/start` : commence une connexion Discord.
    ///
    /// `return_port` : port de [`DiscordReturn`] ; sans lui, le hub renvoie le
    /// navigateur vers `raftup://oauth/discord`. Le jeton n'est **pas** envoyé : on
    /// se connecte (pour lier Discord au compte connecté : [`Self::discord_link_start`]).
    pub fn discord_start(&self, return_port: Option<u16>) -> Result<DiscordStart, HubClientError> {
        self.discord_start_with(return_port, false)
    }

    /// `POST /v1/oauth/discord/start` **avec le jeton** : lie Discord au compte
    /// connecté. La suite est la même (attente du retour, puis [`Self::discord_finish`],
    /// qui rend [`DiscordOutcome::Linked`]). Un compte Discord déjà lié à un autre
    /// compte est refusé à la fin par [`ErrorCode::DiscordTaken`].
    pub fn discord_link_start(
        &self,
        return_port: Option<u16>,
    ) -> Result<DiscordStart, HubClientError> {
        self.discord_start_with(return_port, true)
    }

    /// Corps commun de `discord_start` et `discord_link_start` ; `auth` : envoyer le jeton.
    fn discord_start_with(
        &self,
        return_port: Option<u16>,
        auth: bool,
    ) -> Result<DiscordStart, HubClientError> {
        let body = match return_port {
            Some(port) => json!({ "return_port": port }),
            None => json!({}),
        };
        let json: discord::DiscordStartJson =
            self.call(Method::Post, "/v1/oauth/discord/start", Some(body), auth)?;
        Ok(DiscordStart {
            authorize_url: json.authorize_url,
            state: json.state,
            secret: json.secret,
        })
    }

    /// `POST /v1/oauth/discord/finish` : finit la connexion. Si le compte existe, la
    /// session est gardée dans le client, comme après [`Self::login`].
    pub fn discord_finish(
        &mut self,
        ticket: &DiscordTicket,
    ) -> Result<DiscordOutcome, HubClientError> {
        let body = json!({
            "state": ticket.state,
            "secret": ticket.secret,
            "ticket": ticket.ticket,
        });
        let json: discord::DiscordFinishJson =
            self.call(Method::Post, "/v1/oauth/discord/finish", Some(body), false)?;
        match json.status.as_str() {
            "logged_in" => self
                .keep_discord_session(json)
                .map(DiscordOutcome::LoggedIn),
            "linked" => Ok(DiscordOutcome::Linked),
            "needs_username" => Ok(DiscordOutcome::NeedsUsername {
                suggested_username: json.suggested_username,
                invite_required: json.invite_required,
            }),
            other => Err(unexpected(200, format!("statut Discord inconnu : {other}"))),
        }
    }

    /// `POST /v1/oauth/discord/register` : crée le compte d'un nouveau joueur Discord
    /// avec le pseudo choisi, et garde la session. Pseudo pris ou code d'invitation
    /// faux : le même `ticket` peut resservir avec une autre saisie.
    pub fn discord_register(
        &mut self,
        ticket: &DiscordTicket,
        username: &str,
        invite_code: Option<&str>,
    ) -> Result<Session, HubClientError> {
        let mut body = json!({
            "state": ticket.state,
            "secret": ticket.secret,
            "ticket": ticket.ticket,
            "username": non_empty(username)?,
        });
        if let Some(code) = invite_code.map(str::trim).filter(|c| !c.is_empty()) {
            body["invite_code"] = json!(code);
        }
        let json: discord::DiscordFinishJson = self.call(
            Method::Post,
            "/v1/oauth/discord/register",
            Some(body),
            false,
        )?;
        if json.status != "logged_in" {
            return Err(unexpected(
                200,
                format!("statut Discord inconnu : {}", json.status),
            ));
        }
        self.keep_discord_session(json)
    }

    /// `DELETE /v1/me/identities/discord` : délie Discord du compte connecté. Refusé
    /// si c'est le dernier moyen de se connecter ([`ErrorCode::LastIdentity`]) ou si
    /// aucun compte Discord n'est lié ([`ErrorCode::IdentityNotFound`]).
    pub fn discord_unlink(&self) -> Result<(), HubClientError> {
        self.call_empty(Method::Delete, "/v1/me/identities/discord", None)
    }

    /// Lit la session d'une réponse `logged_in` et garde le jeton.
    fn keep_discord_session(
        &mut self,
        json: discord::DiscordFinishJson,
    ) -> Result<Session, HubClientError> {
        // `zip` : les quatre champs doivent être là ensemble, sinon la réponse est fausse.
        let fields = json
            .token
            .zip(json.user_id)
            .zip(json.username)
            .zip(json.expires_at);
        let Some((((token, user_id), username), expires_at)) = fields else {
            return Err(unexpected(200, "session incomplète".to_string()));
        };
        let session = Session {
            token: SessionToken::new(token),
            user_id,
            username,
            expires_at,
        };
        self.token = Some(session.token.clone());
        Ok(session)
    }

    /// `POST /v1/accounts` : crée un compte. Ne connecte pas : appeler [`Self::login`].
    ///
    /// `invite_code` : code d'invitation, pour un hub en mode « sur invitation »
    /// (ADR 0036). `None` ou un texte vide : le champ n'est pas envoyé. Le hub ignore
    /// la casse et les tirets ; ici on retire seulement les espaces autour.
    pub fn register(
        &self,
        email: &str,
        password: &Password,
        username: &str,
        invite_code: Option<&str>,
    ) -> Result<Account, HubClientError> {
        let mut body = json!({
            "email": email,
            "password": password.expose(),
            "username": username,
        });
        if let Some(code) = invite_code.map(str::trim).filter(|c| !c.is_empty()) {
            body["invite_code"] = json!(code);
        }
        self.call(Method::Post, "/v1/accounts", Some(body), false)
    }

    /// `POST /v1/sessions` : se connecte et garde le jeton dans le client.
    pub fn login(&mut self, email: &str, password: &Password) -> Result<Session, HubClientError> {
        let body = json!({ "email": email, "password": password.expose() });
        let json: types::SessionJson =
            self.call(Method::Post, "/v1/sessions", Some(body), false)?;
        let session = Session {
            token: SessionToken::new(json.token),
            user_id: json.user_id,
            username: json.username,
            expires_at: json.expires_at,
        };
        self.token = Some(session.token.clone());
        Ok(session)
    }

    /// `DELETE /v1/sessions/current` : révoque la session sur le hub, puis l'oublie.
    ///
    /// Le jeton est oublié **même si** le hub ne répond pas : se déconnecter marche
    /// toujours de ce côté. Une session déjà expirée n'est pas une erreur.
    pub fn logout(&mut self) -> Result<(), HubClientError> {
        let result = self.call_empty(Method::Delete, "/v1/sessions/current", None);
        self.token = None;
        match result {
            Err(error) if error.is_unauthorized() => Ok(()),
            other => other,
        }
    }

    /// `GET /v1/me` : le compte connecté.
    pub fn me(&self) -> Result<Me, HubClientError> {
        self.call(Method::Get, "/v1/me", None, true)
    }

    /// `PATCH /v1/me` : change son pseudo. Mêmes règles qu'à l'inscription ; le hub
    /// refuse un deuxième changement en moins de 24 heures
    /// ([`ErrorCode::UsernameChangeTooSoon`]). Renvoie le compte avec le nouveau pseudo.
    pub fn change_username(&self, username: &str) -> Result<Account, HubClientError> {
        let username = non_empty(username)?;
        let body = json!({ "username": username });
        self.call(Method::Patch, "/v1/me", Some(body), true)
    }

    // ---------- Jeton rangé ----------

    /// Range le jeton de la session dans `store`.
    pub fn remember(&self, store: &dyn TokenStore) -> Result<(), HubClientError> {
        let token = self.token.as_ref().ok_or(HubClientError::NotLoggedIn)?;
        store.save(&self.url, token)?;
        Ok(())
    }

    /// Reprend la session rangée pour ce hub : `Ok(None)` s'il n'y en a pas, ou si le
    /// hub l'a refusée (expirée, révoquée) ; dans ce cas, le jeton périmé est oublié.
    pub fn restore(
        url: HubUrl,
        store: &dyn TokenStore,
    ) -> Result<Option<(HubClient, Me)>, HubClientError> {
        let Some(token) = store.load(&url)? else {
            return Ok(None);
        };
        let client = HubClient::with_token(url, token);
        match client.me() {
            Ok(me) => Ok(Some((client, me))),
            Err(error) if error.is_unauthorized() => {
                store.clear(&client.url)?;
                Ok(None)
            }
            Err(other) => Err(other),
        }
    }

    // ---------- Présence et amis ----------

    /// `PUT /v1/presence` : sa présence. Le hub l'oublie au bout de 2 minutes sans
    /// renouvellement : l'appeler régulièrement.
    pub fn set_presence(&self, presence: &Presence) -> Result<(), HubClientError> {
        let body = serde_json::to_value(presence).map_err(|e| unexpected(0, e.to_string()))?;
        self.call_empty(Method::Put, "/v1/presence", Some(body))
    }

    /// `GET /v1/friends` : amis et leur présence.
    pub fn friends(&self) -> Result<Vec<Friend>, HubClientError> {
        self.call(Method::Get, "/v1/friends", None, true)
    }

    /// `GET /v1/friends/requests` : demandes reçues et envoyées.
    pub fn requests(&self) -> Result<FriendRequests, HubClientError> {
        self.call(Method::Get, "/v1/friends/requests", None, true)
    }

    /// `POST /v1/friends/requests` : demande en ami par pseudo.
    pub fn send_request(&self, username: &str) -> Result<RequestOutcome, HubClientError> {
        let username = non_empty(username)?;
        let body = json!({ "username": username });
        let json: types::RequestOutcomeJson =
            self.call(Method::Post, "/v1/friends/requests", Some(body), true)?;
        match json.status.as_str() {
            "pending" => Ok(RequestOutcome::Pending),
            "accepted" => Ok(RequestOutcome::Accepted),
            other => Err(unexpected(
                200,
                format!("statut de demande inconnu : {other}"),
            )),
        }
    }

    /// `POST /v1/friends/requests/{pseudo}/accept`.
    pub fn accept_request(&self, username: &str) -> Result<(), HubClientError> {
        let path = format!("/v1/friends/requests/{}/accept", segment(username)?);
        self.call_empty(Method::Post, &path, None)
    }

    /// `POST /v1/friends/requests/{pseudo}/decline`.
    pub fn decline_request(&self, username: &str) -> Result<(), HubClientError> {
        let path = format!("/v1/friends/requests/{}/decline", segment(username)?);
        self.call_empty(Method::Post, &path, None)
    }

    /// `DELETE /v1/friends/requests/{pseudo}` : annule sa propre demande.
    pub fn cancel_request(&self, username: &str) -> Result<(), HubClientError> {
        let path = format!("/v1/friends/requests/{}", segment(username)?);
        self.call_empty(Method::Delete, &path, None)
    }

    /// `DELETE /v1/friends/{pseudo}` : retire un ami.
    pub fn remove_friend(&self, username: &str) -> Result<(), HubClientError> {
        let path = format!("/v1/friends/{}", segment(username)?);
        self.call_empty(Method::Delete, &path, None)
    }

    // ---------- Parties hébergées (ADR 0052) ----------

    /// `POST /v1/games` : envoie la seed (et la sauvegarde) ; le hub démarre la partie.
    ///
    /// Un refus de taille du hub (413) arrive parfois sans corps JSON (limite de la
    /// requête) : il devient quand même le code `seed_too_large`.
    pub fn create_game(&self, upload: &GameUpload) -> Result<CreatedGame, HubClientError> {
        let result = self.send_with(
            Method::Post,
            "/v1/games",
            Some(upload.to_json()),
            true,
            UPLOAD_TIMEOUT,
        );
        let (status, text) = match result {
            Err(HubClientError::UnexpectedResponse { status: 413, .. }) => {
                return Err(HubClientError::Api {
                    status: 413,
                    code: ErrorCode::SeedTooLarge,
                    message: "seed trop grosse".to_string(),
                })
            }
            other => other?,
        };
        serde_json::from_str(&text).map_err(|e| unexpected(status, e.to_string()))
    }

    /// `GET /v1/games` : les parties hébergées par ce compte.
    pub fn games(&self) -> Result<Vec<HostedGame>, HubClientError> {
        let list: games::GameList = self.call(Method::Get, "/v1/games", None, true)?;
        Ok(list.games)
    }

    /// `GET /v1/games/{id}` : état d'une partie (joueurs connectés, avancement).
    pub fn game_status(&self, id: &str) -> Result<GameStatus, HubClientError> {
        let path = game_path(id)?;
        self.call(Method::Get, &path, None, true)
    }

    /// `DELETE /v1/games/{id}` : arrête et supprime la partie (hôte seulement).
    pub fn delete_game(&self, id: &str) -> Result<(), HubClientError> {
        let path = game_path(id)?;
        self.call_empty(Method::Delete, &path, None)
    }

    // ---------- Appels HTTP ----------

    /// Appel dont la réponse est un objet JSON lu en `T`.
    ///
    /// `DeserializeOwned` : `T` se construit depuis le texte sans l'emprunter, donc le
    /// texte peut être libéré à la fin de la fonction.
    fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        auth: bool,
    ) -> Result<T, HubClientError> {
        let (status, text) = self.send(method, path, body, auth)?;
        serde_json::from_str(&text).map_err(|e| unexpected(status, e.to_string()))
    }

    /// Appel authentifié sans corps de réponse utile (204).
    fn call_empty(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<(), HubClientError> {
        self.send(method, path, body, true).map(|_| ())
    }

    /// Envoie la requête et rend le code et le corps d'une réponse réussie (2xx) ;
    /// une réponse d'erreur devient [`HubClientError::Api`].
    fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        auth: bool,
    ) -> Result<(u16, String), HubClientError> {
        self.send_with(method, path, body, auth, TIMEOUT)
    }

    /// Comme [`Self::send`], avec un délai au choix (envoi d'une seed).
    fn send_with(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        auth: bool,
        timeout: Duration,
    ) -> Result<(u16, String), HubClientError> {
        let url = self.url.join(path);
        let bearer = if auth {
            let token = self.token.as_ref().ok_or(HubClientError::NotLoggedIn)?;
            Some(format!("Bearer {}", token.expose()))
        } else {
            None
        };
        let body = body.map(|value| value.to_string());

        // ureq a deux sortes de requêtes : sans corps (GET, DELETE) et avec corps (POST,
        // PUT, PATCH). Ce sont deux types différents, d'où les deux branches.
        let result = match method {
            Method::Get | Method::Delete => {
                let mut request = match method {
                    Method::Get => self.agent.get(&url),
                    _ => self.agent.delete(&url),
                };
                if let Some(bearer) = &bearer {
                    request = request.header("Authorization", bearer);
                }
                // `config()` règle le délai de cette seule requête.
                request
                    .config()
                    .timeout_global(Some(timeout))
                    .build()
                    .call()
            }
            Method::Post | Method::Put | Method::Patch => {
                let mut request = match method {
                    Method::Post => self.agent.post(&url),
                    Method::Patch => self.agent.patch(&url),
                    _ => self.agent.put(&url),
                };
                if let Some(bearer) = &bearer {
                    request = request.header("Authorization", bearer);
                }
                let request = request.config().timeout_global(Some(timeout)).build();
                match body {
                    Some(body) => request
                        .header("Content-Type", "application/json")
                        .send(body.as_str()),
                    None => request.send_empty(),
                }
            }
        };
        // Le message d'ureq ne contient ni le jeton (en-tête) ni le corps.
        let mut response = result.map_err(|e| HubClientError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_string()
            // Pas de corps (204) : texte vide, ce n'est pas une erreur.
            .unwrap_or_default();

        if (200..300).contains(&status) {
            return Ok((status, text));
        }
        match serde_json::from_str::<types::ErrorJson>(&text) {
            Ok(error) => Err(HubClientError::Api {
                status,
                code: ErrorCode::parse(&error.error),
                message: if error.message.is_empty() {
                    error.error
                } else {
                    error.message
                },
            }),
            Err(_) => Err(unexpected(status, "pas de corps d'erreur JSON".to_string())),
        }
    }
}

fn unexpected(status: u16, detail: String) -> HubClientError {
    HubClientError::UnexpectedResponse { status, detail }
}

/// Pseudo nettoyé des espaces autour ; vide : erreur.
fn non_empty(username: &str) -> Result<&str, HubClientError> {
    let username = username.trim();
    if username.is_empty() {
        return Err(HubClientError::EmptyUsername);
    }
    Ok(username)
}

/// Chemin `/v1/games/{id}`, après vérification de l'identifiant.
fn game_path(id: &str) -> Result<String, HubClientError> {
    if !games::valid_id(id) {
        return Err(HubClientError::BadGameId);
    }
    Ok(format!("/v1/games/{id}"))
}

/// Pseudo encodé pour un morceau de chemin.
fn segment(username: &str) -> Result<String, HubClientError> {
    Ok(utf8_percent_encode(non_empty(username)?, PATH_SEGMENT).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adresses_acceptees() {
        for (text, expected) in [
            ("https://hub.example.org", "https://hub.example.org"),
            ("  https://hub.example.org/  ", "https://hub.example.org"),
            ("https://example.org/hub/", "https://example.org/hub"),
            ("http://127.0.0.1:8787", "http://127.0.0.1:8787"),
            ("http://localhost:8787/", "http://localhost:8787"),
            ("http://LOCALHOST", "http://LOCALHOST"),
            ("http://[::1]:8787", "http://[::1]:8787"),
        ] {
            assert_eq!(HubUrl::parse(text).unwrap().as_str(), expected, "{text}");
        }
    }

    #[test]
    fn hote_affiche() {
        for (text, host) in [
            ("https://hub.example.org", "hub.example.org"),
            ("https://example.org/hub", "example.org"),
            ("http://127.0.0.1:8787", "127.0.0.1:8787"),
            ("http://[::1]:8787/x", "[::1]:8787"),
        ] {
            assert_eq!(HubUrl::parse(text).unwrap().host(), host, "{text}");
        }
    }

    #[test]
    fn adresses_refusees() {
        for text in [
            "",
            "hub.example.org",
            "ftp://hub.example.org",
            "http://hub.example.org",
            "http://192.168.1.10:8787",
            "http://localhost.evil.org",
            "http://127.0.0.1.evil.org",
            "http://[::1].evil.org",
            "https://",
            "https://user:pass@hub.example.org",
            "http://evil.org@127.0.0.1",
            "https://hub.example.org/?x=1",
            "https://hub.example.org/#a",
            "https://hub example.org",
        ] {
            assert!(
                HubUrl::parse(text).is_err(),
                "{text} aurait dû être refusée"
            );
        }
    }

    #[test]
    fn adresse_par_defaut_valide() {
        assert!(HubUrl::parse(DEFAULT_HUB_URL).is_ok());
    }

    #[test]
    fn pseudo_encode_dans_le_chemin() {
        assert_eq!(segment("Mimi").unwrap(), "Mimi");
        assert_eq!(segment(" a.b_c ").unwrap(), "a%2Eb%5Fc");
        assert_eq!(segment("../x").unwrap(), "%2E%2E%2Fx");
        assert_eq!(segment("é").unwrap(), "%C3%A9");
        assert!(matches!(segment("  "), Err(HubClientError::EmptyUsername)));
    }

    #[test]
    fn mot_de_passe_masque() {
        let password = Password::new("hunter22");
        assert_eq!(format!("{password:?}"), "Password(***)");
        let client = HubClient::with_token(
            HubUrl::parse(DEFAULT_HUB_URL).unwrap(),
            SessionToken::new("jeton-secret"),
        );
        assert!(!format!("{client:?}").contains("jeton-secret"));
    }

    #[test]
    fn sans_session_pas_d_appel() {
        let client = HubClient::new(HubUrl::parse(DEFAULT_HUB_URL).unwrap());
        assert!(matches!(client.friends(), Err(HubClientError::NotLoggedIn)));
        assert!(client.friends().unwrap_err().is_unauthorized());
    }

    #[test]
    fn codes_inconnus_gardes() {
        assert_eq!(ErrorCode::parse("username_taken"), ErrorCode::UsernameTaken);
        assert_eq!(
            ErrorCode::parse("nouveau_code"),
            ErrorCode::Other("nouveau_code".to_string())
        );
    }
}
