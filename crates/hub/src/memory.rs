//! État gardé en mémoire, jamais en base : présence et limites de débit (ADR 0002).
//!
//! Les deux structures sont partagées entre toutes les requêtes. Elles sont protégées
//! par un `std::sync::Mutex` : on ne garde jamais le verrou pendant un `.await`, donc
//! un mutex ordinaire suffit (pas besoin de celui de tokio).

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Prend le verrou même s'il est « empoisonné » (un thread a paniqué en le tenant) :
/// les données restent cohérentes ici, chaque modification tient en une instruction.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Statut de présence visible par les amis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Online,
    InGame,
    Offline,
}

/// Présence telle que l'API la montre.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PresenceView {
    pub status: Status,
    /// Nom du jeu, seulement en partie.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
}

impl PresenceView {
    fn offline() -> Self {
        PresenceView {
            status: Status::Offline,
            game: None,
        }
    }
}

struct PresenceEntry {
    view: PresenceView,
    updated: Instant,
}

/// Présence de chaque compte connecté. Une présence non renouvelée pendant `ttl`
/// redevient « hors ligne » : le launcher la renvoie régulièrement.
pub struct PresenceBoard {
    ttl: Duration,
    entries: Mutex<BTreeMap<i64, PresenceEntry>>,
}

impl PresenceBoard {
    pub fn new(ttl: Duration) -> Self {
        PresenceBoard {
            ttl,
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Enregistre la présence d'un compte ; `Offline` l'efface.
    pub fn set(&self, user_id: i64, view: PresenceView) {
        let mut entries = lock(&self.entries);
        if view.status == Status::Offline {
            entries.remove(&user_id);
        } else {
            entries.insert(
                user_id,
                PresenceEntry {
                    view,
                    updated: Instant::now(),
                },
            );
        }
    }

    pub fn clear(&self, user_id: i64) {
        lock(&self.entries).remove(&user_id);
    }

    pub fn get(&self, user_id: i64) -> PresenceView {
        let mut entries = lock(&self.entries);
        match entries.get(&user_id) {
            Some(entry) if entry.updated.elapsed() <= self.ttl => entry.view.clone(),
            Some(_) => {
                // Expirée : on en profite pour libérer la place.
                entries.remove(&user_id);
                PresenceView::offline()
            }
            None => PresenceView::offline(),
        }
    }
}

/// Compteur par clé sur une fenêtre de temps fixe (ex. 10 échecs de connexion
/// par adresse e-mail en 15 minutes).
pub struct RateLimiter {
    max: u32,
    window: Duration,
    hits: Mutex<BTreeMap<String, (u32, Instant)>>,
}

/// Au-delà de ce nombre de clés, on retire les fenêtres terminées.
const MAX_TRACKED_KEYS: usize = 10_000;

impl RateLimiter {
    pub fn new(max: u32, window: Duration) -> Self {
        RateLimiter {
            max,
            window,
            hits: Mutex::new(BTreeMap::new()),
        }
    }

    /// Vrai si la clé a atteint la limite dans la fenêtre en cours (sans compter).
    pub fn is_blocked(&self, key: &str) -> bool {
        let hits = lock(&self.hits);
        match hits.get(key) {
            Some((count, start)) => start.elapsed() < self.window && *count >= self.max,
            None => false,
        }
    }

    /// Compte un essai ; renvoie `false` si la limite est dépassée.
    pub fn hit(&self, key: &str) -> bool {
        let mut hits = lock(&self.hits);
        if hits.len() >= MAX_TRACKED_KEYS {
            let window = self.window;
            hits.retain(|_, (_, start)| start.elapsed() < window);
        }
        let entry = hits.entry(key.to_string()).or_insert((0, Instant::now()));
        if entry.1.elapsed() >= self.window {
            *entry = (0, Instant::now());
        }
        entry.0 = entry.0.saturating_add(1);
        entry.0 <= self.max
    }

    pub fn reset(&self, key: &str) {
        lock(&self.hits).remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_expiree_redevient_hors_ligne() {
        let board = PresenceBoard::new(Duration::ZERO);
        board.set(
            1,
            PresenceView {
                status: Status::Online,
                game: None,
            },
        );
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(board.get(1).status, Status::Offline);
    }

    #[test]
    fn limite_puis_remise_a_zero() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.hit("a"));
        assert!(!limiter.is_blocked("a"));
        assert!(limiter.hit("a"));
        assert!(limiter.is_blocked("a"));
        assert!(!limiter.hit("a"));
        assert!(!limiter.is_blocked("b"));
        limiter.reset("a");
        assert!(!limiter.is_blocked("a"));
    }
}
