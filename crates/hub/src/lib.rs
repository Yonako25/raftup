//! `raftup-hub` : comptes, amis et présence des joueurs de Raftup (ADR 0002, 0033).
//!
//! Licence : **AGPL-3.0-or-later** (ADR 0003), contrairement au reste du dépôt.
//! Aucune crate MIT/Apache ne dépend de celle-ci.
//!
//! L'API HTTP est versionnée (`/v1/...`) et parle JSON. Voir [`api`] pour les routes.
//! Pas encore fait (ADR 0033) : vérification de l'adresse e-mail, mot de passe oublié,
//! connexion Discord, blocages, changement de pseudo, export et suppression du compte.

pub mod api;
pub mod error;
pub mod identity;
pub mod memory;
pub mod password;
pub mod session;
pub mod store;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use api::router;
pub use error::HubError;
pub use password::{HashParams, Password};
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
    friend_limiter: RateLimiter,
    /// Hash d'un mot de passe bidon : vérifié quand l'adresse n'existe pas, pour que
    /// la réponse prenne le même temps (on ne devine pas les adresses inscrites).
    dummy_hash: String,
}

impl AppState {
    pub async fn new(store: Store, config: HubConfig) -> Result<AppState, HubError> {
        let params = config.hash_params;
        let dummy_hash = blocking(move || {
            password::hash_password(&Password::new("raftup-hub-dummy-password"), &params)
        })
        .await?;
        Ok(AppState {
            inner: Arc::new(Inner {
                presence: PresenceBoard::new(config.presence_ttl),
                login_limiter: RateLimiter::new(config.login_max_failures, config.login_window),
                friend_limiter: RateLimiter::new(
                    config.friend_requests_per_minute,
                    Duration::from_secs(60),
                ),
                store,
                config,
                dummy_hash,
            }),
        })
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
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
