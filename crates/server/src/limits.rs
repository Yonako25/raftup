//! Limites de chaque connexion (NF03, NF04, NF05, ADR 0007).
//!
//! Les valeurs par défaut sont celles de la spec. Les tests les raccourcissent
//! (par exemple un ping toutes les 200 ms) pour ne pas attendre 40 secondes.
//!
//! Les compteurs prennent l'heure en paramètre (`now`) au lieu de la lire
//! eux-mêmes : on peut ainsi les tester sans attendre.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Toutes les limites du serveur.
///
/// `Clone, Copy` : la structure ne contient que des nombres, on peut la copier
/// librement dans chaque tâche de connexion, sans `Arc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Taille maximale d'un message WebSocket, en octets (NF03).
    pub max_message_size: usize,
    /// Longueur maximale d'un message de chat, en caractères (NF03).
    pub max_chat_length: usize,
    /// Paquets acceptés par seconde et par connexion (NF04).
    pub packets_per_second: u32,
    /// Messages de chat acceptés pendant `chat_window` (NF04).
    pub chat_messages: usize,
    /// Fenêtre du compteur de chat (NF04).
    pub chat_window: Duration,
    /// Intervalle entre deux pings (NF05).
    pub ping_interval: Duration,
    /// Fermeture d'une connexion dont on n'a rien reçu pendant ce temps (NF05).
    pub silence_timeout: Duration,
    /// Taille de la file connexions → salle, en événements.
    pub room_queue: usize,
    /// Taille de la file salle → client, en trames.
    pub client_queue: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_message_size: 1024 * 1024,
            max_chat_length: 2000,
            packets_per_second: 50,
            chat_messages: 5,
            chat_window: Duration::from_secs(10),
            ping_interval: Duration::from_secs(20),
            silence_timeout: Duration::from_secs(40),
            room_queue: 1024,
            client_queue: 256,
        }
    }
}

/// Compteur de débit « seau à jetons » (NF04).
///
/// Le seau contient au plus `rate` jetons et se remplit de `rate` jetons par
/// seconde. Chaque paquet consomme un jeton : on accepte une rafale de `rate`
/// paquets, puis `rate` paquets par seconde en moyenne.
#[derive(Debug)]
pub(crate) struct PacketRate {
    rate: f64,
    tokens: f64,
    last: Instant,
}

impl PacketRate {
    pub(crate) fn new(rate: u32, now: Instant) -> PacketRate {
        let rate = f64::from(rate);
        PacketRate {
            rate,
            tokens: rate,
            last: now,
        }
    }

    /// Consomme `count` jetons ; `false` s'il n'y en a pas assez.
    pub(crate) fn allow(&mut self, count: usize, now: Instant) -> bool {
        // `saturating_duration_since` : 0 si `now` est avant `last`, sans paniquer.
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate).min(self.rate);
        // Une trame a au plus quelques milliers de paquets : la conversion est exacte.
        let needed = count as f64;
        if needed > self.tokens {
            return false;
        }
        self.tokens -= needed;
        true
    }
}

/// Compteur de messages de chat sur une fenêtre glissante (NF04).
#[derive(Debug, Default)]
pub(crate) struct ChatRate {
    /// Heure des derniers messages acceptés, du plus ancien au plus récent.
    accepted: VecDeque<Instant>,
}

impl ChatRate {
    /// Vrai si le message est accepté (et alors compté).
    pub(crate) fn allow(&mut self, limits: &Limits, now: Instant) -> bool {
        // On oublie les messages sortis de la fenêtre.
        while let Some(&oldest) = self.accepted.front() {
            if now.saturating_duration_since(oldest) >= limits.chat_window {
                self.accepted.pop_front();
            } else {
                break;
            }
        }
        if self.accepted.len() >= limits.chat_messages {
            return false;
        }
        self.accepted.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rafale_puis_refus() {
        let start = Instant::now();
        let mut rate = PacketRate::new(50, start);
        assert!(rate.allow(50, start));
        assert!(!rate.allow(1, start));
    }

    #[test]
    fn le_seau_se_remplit() {
        let start = Instant::now();
        let mut rate = PacketRate::new(50, start);
        assert!(rate.allow(50, start));
        // Une demi-seconde plus tard : 25 jetons de plus.
        let later = start + Duration::from_millis(500);
        assert!(rate.allow(25, later));
        assert!(!rate.allow(1, later));
    }

    #[test]
    fn le_seau_ne_deborde_pas() {
        let start = Instant::now();
        let mut rate = PacketRate::new(50, start);
        // Après une minute de calme, toujours 50 jetons au plus.
        let later = start + Duration::from_secs(60);
        assert!(!rate.allow(51, later));
        assert!(rate.allow(50, later));
    }

    #[test]
    fn chat_cinq_par_fenetre() {
        let limits = Limits::default();
        let start = Instant::now();
        let mut chat = ChatRate::default();
        for _ in 0..5 {
            assert!(chat.allow(&limits, start));
        }
        assert!(!chat.allow(&limits, start + Duration::from_secs(9)));
        // Dix secondes après les premiers, la fenêtre est libre.
        assert!(chat.allow(&limits, start + Duration::from_secs(10)));
    }

    #[test]
    fn chat_refuse_non_compte() {
        let limits = Limits::default();
        let start = Instant::now();
        let mut chat = ChatRate::default();
        for _ in 0..5 {
            assert!(chat.allow(&limits, start));
        }
        // Les refus ne prolongent pas l'attente.
        for second in 1..10 {
            assert!(!chat.allow(&limits, start + Duration::from_secs(second)));
        }
        assert!(chat.allow(&limits, start + Duration::from_secs(10)));
    }
}
