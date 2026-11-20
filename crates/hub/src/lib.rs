//! Rafthub (binaire `rafthub`) : comptes, amis et présence des joueurs de Raftup
//! (ADR 0002, 0033), et sa console d'admin web sous `/admin` (ADR 0036).
//!
//! Licence : **AGPL-3.0-or-later** (ADR 0003), contrairement au reste du dépôt.
//! Aucune crate MIT/Apache ne dépend de celle-ci.
//!
//! L'API HTTP est versionnée (`/v1/...`) et parle JSON. Voir [`api`] pour les routes,
//! et [`web`] pour les pages de la console d'admin.
//! Pas encore fait (ADR 0033) : changement de pseudo, refus de toutes les demandes.

pub mod api;
pub mod console;
pub mod discord;
pub mod distribution;
pub mod error;
pub mod identity;
pub mod mail;
pub mod memory;
pub mod password;
pub mod server_config;
pub mod session;
pub mod settings;
pub mod store;
pub mod web;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use api::router;
pub use discord::{DiscordApi, DiscordConfig, DiscordUser, FakeDiscord, HttpDiscord};
pub use error::HubError;
pub use mail::{DisabledMailer, Email, Mailer, MemoryMailer, SmtpMailer};
pub use password::{HashParams, Password};
pub use settings::{EnvSettings, PublicSettings, RegistrationMode, SettingKey, Settings};
pub use store::Store;

use memory::{PresenceBoard, RateLimiter};

/// Réglages du hub. `Default` donne les valeurs de production.
#[derive(Clone, Debug)]
pub struct HubConfig {
    /// Coût du hachage des mots de passe.
    pub hash_params: HashParams,
    /// Durée de vie d'une session (30 jours par défaut).
    pub session_lifetime: Duration,
    /// Délai après lequel une présence non renouvelée redevient « hors ligne ».
    pub presence_ttl: Duration,
    /// Échecs de connexion permis par adresse e-mail dans `login_window`.
    pub login_max_failures: u32,
    pub login_window: Duration,
    /// Demandes d'ami permises par compte et par minute.
    pub friend_requests_per_minute: u32,
    /// Adresse publique **par défaut** du hub, pour les liens des e-mails. Le
    /// réglage `public_url` (base, puis environnement) passe avant (ADR 0036).
    pub public_url: String,
    /// Réglages venus de l'environnement et de la ligne de commande (ADR 0036) :
    /// utilisés quand la base n'a pas de valeur. Vide par défaut.
    pub settings_env: EnvSettings,
    /// Durée de validité d'un lien de vérification d'adresse.
    pub email_verify_lifetime: Duration,
    /// Durée de validité d'un code « mot de passe oublié ».
    pub password_reset_lifetime: Duration,
    /// E-mails permis par adresse (ou par compte) dans `mail_window`.
    pub mails_per_window: u32,
    pub mail_window: Duration,
    /// Délai pour finir une connexion Discord commencée.
    pub oauth_ttl: Duration,
    /// Connexions Discord en attente gardées au plus en mémoire.
    pub oauth_max_pending: usize,
    /// Durée de vie d'une session de la console d'admin (12 h, ADR 0036).
    pub admin_session_lifetime: Duration,
    /// Distribution du launcher : cache de l'exe officiel et d'où il vient (ADR 0036).
    pub distribution: distribution::DistributionConfig,
    /// Code d'installation à usage unique, tant qu'aucun admin n'existe : il ouvre
    /// la page `/admin/installation` qui crée le premier admin (ADR 0037). `None` :
    /// page fermée.
    pub setup_code: Option<String>,
}

impl Default for HubConfig {
    fn default() -> Self {
        HubConfig {
            hash_params: HashParams::production(),
            session_lifetime: Duration::from_secs(30 * 24 * 3600),
            presence_ttl: Duration::from_secs(120),
            login_max_failures: 10,
            login_window: Duration::from_secs(15 * 60),
            friend_requests_per_minute: 20,
            public_url: "http://127.0.0.1:8787".to_string(),
            email_verify_lifetime: Duration::from_secs(24 * 3600),
            password_reset_lifetime: Duration::from_secs(3600),
            mails_per_window: 3,
            mail_window: Duration::from_secs(3600),
            oauth_ttl: Duration::from_secs(10 * 60),
            oauth_max_pending: 10_000,
            admin_session_lifetime: Duration::from_secs(12 * 3600),
            settings_env: EnvSettings::default(),
            distribution: distribution::DistributionConfig::default(),
            setup_code: None,
        }
    }
}

impl HubConfig {
    /// Réglages pour les tests : hachage minimal (rapide en debug), le reste inchangé.
    pub fn for_tests() -> Self {
        HubConfig {
            hash_params: HashParams::fast_for_tests(),
            ..HubConfig::default()
        }
    }
}

/// D'où viennent le service d'envoi d'e-mails et le client Discord.
enum Services {
    /// Choisis une fois pour toutes (tests : `MemoryMailer`, `FakeDiscord`).
    ///
    /// `Arc<dyn Mailer>` : un objet partagé dont on ne connaît que le trait ; le vrai
    /// type (SMTP, mémoire…) est choisi au lancement. Sans Discord (`None`), les
    /// routes Discord répondent 503.
    Fixed {
        mailer: Arc<dyn Mailer>,
        discord: Option<Arc<dyn DiscordApi>>,
    },
    /// Construits à chaque usage depuis les [`Settings`] (base > environnement >
    /// défaut) : un changement fait dans la console vaut tout de suite (ADR 0036).
    FromSettings,
}

/// État partagé par toutes les requêtes.
///
/// `Arc` (compteur de références partagé entre threads) : axum clone l'état pour
/// chaque requête ; cloner un `Arc` ne copie pas les données, seulement le pointeur.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    config: HubConfig,
    presence: PresenceBoard,
    login_limiter: RateLimiter,
    /// Échecs de connexion à la console d'admin, par adresse e-mail. À part de
    /// `login_limiter` : bloquer l'API d'un joueur ne bloque pas la console.
    admin_login_limiter: RateLimiter,
    friend_limiter: RateLimiter,
    /// Limite les e-mails envoyés à une même adresse (ou pour un même compte).
    mail_limiter: RateLimiter,
    /// Envoi des e-mails et connexion Discord : fixés au lancement, ou relus depuis
    /// les réglages à chaque usage.
    services: Services,
    /// Connexions Discord commencées et pas encore finies.
    oauth_pending: discord::PendingLogins,
    /// Hash d'un mot de passe bidon : vérifié quand l'adresse n'existe pas, pour que
    /// la réponse prenne le même temps (on ne devine pas les adresses inscrites).
    dummy_hash: String,
    /// Exe officiel du launcher en cache, et zip relié à ce hub.
    distribution: distribution::Distribution,
}

impl AppState {
    /// État sans envoi d'e-mail ([`DisabledMailer`]).
    pub async fn new(store: Store, config: HubConfig) -> Result<AppState, HubError> {
        AppState::with_mailer(store, config, Arc::new(DisabledMailer)).await
    }

    /// État avec un service d'envoi d'e-mails choisi, sans Discord.
    pub async fn with_mailer(
        store: Store,
        config: HubConfig,
        mailer: Arc<dyn Mailer>,
    ) -> Result<AppState, HubError> {
        AppState::with_services(store, config, mailer, None).await
    }

    /// État avec un envoi d'e-mails et un Discord fixés (tests), indépendants des
    /// réglages SMTP et Discord.
    pub async fn with_services(
        store: Store,
        config: HubConfig,
        mailer: Arc<dyn Mailer>,
        discord: Option<Arc<dyn DiscordApi>>,
    ) -> Result<AppState, HubError> {
        AppState::build(store, config, Services::Fixed { mailer, discord }).await
    }

    /// État de production : SMTP et Discord relus depuis les réglages (base,
    /// environnement de `config.settings_env`, défaut) à chaque usage.
    pub async fn from_settings(store: Store, config: HubConfig) -> Result<AppState, HubError> {
        AppState::build(store, config, Services::FromSettings).await
    }

    async fn build(
        store: Store,
        config: HubConfig,
        services: Services,
    ) -> Result<AppState, HubError> {
        let params = config.hash_params;
        let dummy_hash = blocking(move || {
            password::hash_password(&Password::new("raftup-hub-dummy-password"), &params)
        })
        .await?;
        Ok(AppState {
            inner: Arc::new(Inner {
                presence: PresenceBoard::new(config.presence_ttl),
                login_limiter: RateLimiter::new(config.login_max_failures, config.login_window),
                admin_login_limiter: RateLimiter::new(
                    config.login_max_failures,
                    config.login_window,
                ),
                friend_limiter: RateLimiter::new(
                    config.friend_requests_per_minute,
                    Duration::from_secs(60),
                ),
                mail_limiter: RateLimiter::new(config.mails_per_window, config.mail_window),
                services,
                oauth_pending: discord::PendingLogins::new(
                    config.oauth_ttl,
                    config.oauth_max_pending,
                ),
                distribution: distribution::Distribution::new(config.distribution.clone()),
                store,
                config,
                dummy_hash,
            }),
        })
    }

    /// Distribution du launcher (cache de l'exe officiel).
    pub fn distribution(&self) -> &distribution::Distribution {
        &self.inner.distribution
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    /// Réglages actuels (relus en base à chaque appel : quelques lignes).
    pub async fn settings(&self) -> Result<Settings, HubError> {
        let database = self.inner.store.all_settings().await?;
        Ok(Settings::resolve(
            &database,
            &self.inner.config.settings_env,
            &self.inner.config.public_url,
        ))
    }

    /// Service d'envoi à utiliser maintenant.
    pub(crate) async fn mailer(&self) -> Result<Arc<dyn Mailer>, HubError> {
        match &self.inner.services {
            Services::Fixed { mailer, .. } => Ok(Arc::clone(mailer)),
            Services::FromSettings => Ok(match self.settings().await?.smtp_mailer() {
                Some(mailer) => Arc::new(mailer),
                None => Arc::new(DisabledMailer),
            }),
        }
    }

    /// Client Discord à utiliser maintenant (`None` : Discord non réglé).
    pub(crate) async fn discord(&self) -> Result<Option<Arc<dyn DiscordApi>>, HubError> {
        match &self.inner.services {
            Services::Fixed { discord, .. } => Ok(discord.clone()),
            Services::FromSettings => Ok(self
                .settings()
                .await?
                .discord_config()
                .map(|config| Arc::new(HttpDiscord::new(config)) as Arc<dyn DiscordApi>)),
        }
    }

    /// Envoie un e-mail sans attendre : l'envoi part sur un thread à part et la
    /// réponse HTTP n'attend pas le serveur SMTP. Elle prend donc le même temps que
    /// l'adresse existe ou non. Un échec ne laisse qu'un avertissement au journal.
    async fn send_mail(&self, email: Email) -> Result<(), HubError> {
        let mailer = self.mailer().await?;
        // Le `JoinHandle` rendu est ignoré : la tâche continue seule.
        drop(tokio::task::spawn_blocking(move || {
            if let Err(error) = mailer.send(&email) {
                tracing::warn!(%error, subject = %email.subject, "e-mail non envoyé");
            }
        }));
        Ok(())
    }
}

/// Lance un calcul lent (argon2) sur un thread à part, pour ne pas bloquer les
/// autres requêtes pendant ce temps.
async fn blocking<T, F>(work: F) -> Result<T, HubError>
where
    // `Send + 'static` : la fonction part sur un autre thread et peut y vivre
    // plus longtemps que l'appelant ; elle ne doit donc rien emprunter.
    F: FnOnce() -> Result<T, HubError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| HubError::Internal(format!("tâche de hachage : {e}")))?
}

/// Heure actuelle en secondes Unix.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
