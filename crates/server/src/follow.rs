//! État du suivi asynchrone (ADR 0031) : absences, dernier départ de chaque
//! slot, votes de release et fil d'activité.
//!
//! Ce module ne fait que ranger des données et calculer des textes : il ne
//! connaît ni les clients ni le réseau. La tâche salle (`room_task/follow.rs`)
//! s'en sert pour répondre aux commandes et écrire le résumé de retour.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use ap_protocol::{SlotId, TeamId};
use multidata::raftup::{
    FeedEntry, FeedKind, ReleaseVotes, SlotAway, SlotSeen, Snapshot, FEED_MAX,
};

/// Longueur maximale du message d'absence, en caractères.
pub(crate) const AWAY_MESSAGE_MAX: usize = 200;

/// Un joueur absent.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Away {
    /// Début de l'absence (secondes depuis 1970).
    pub since: f64,
    pub message: String,
}

/// Dernier départ d'un slot : l'heure, et le nombre d'objets reçus à ce moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Seen {
    pub items: usize,
    pub time: f64,
}

/// Tout l'état du suivi asynchrone d'une salle.
#[derive(Debug, Default)]
pub(crate) struct FollowState {
    pub away: BTreeMap<(TeamId, SlotId), Away>,
    pub seen: BTreeMap<(TeamId, SlotId), Seen>,
    /// Pour chaque (équipe, slot visé), les slots qui ont voté.
    pub votes: BTreeMap<(TeamId, SlotId), BTreeSet<SlotId>>,
    /// Les dernières entrées du fil, de la plus ancienne à la plus récente.
    pub feed: VecDeque<FeedEntry>,
    /// Numéro de la prochaine entrée du fil.
    next_seq: u64,
}

impl FollowState {
    /// État repris d'une sauvegarde (vide pour une sauvegarde version 1).
    pub fn from_snapshot(snapshot: &Snapshot) -> FollowState {
        let away = snapshot
            .away
            .iter()
            .map(|a| {
                let away = Away {
                    since: a.since,
                    message: a.message.clone(),
                };
                ((a.team, a.slot), away)
            })
            .collect();
        let seen = snapshot
            .seen
            .iter()
            .map(|s| {
                // `try_from` : un nombre trop grand pour cette machine est
                // ramené au maximum au lieu de paniquer.
                let items = usize::try_from(s.items).unwrap_or(usize::MAX);
                (
                    (s.team, s.slot),
                    Seen {
                        items,
                        time: s.time,
                    },
                )
            })
            .collect();
        let votes = snapshot
            .release_votes
            .iter()
            .filter(|v| !v.voters.is_empty())
            .map(|v| ((v.team, v.target), v.voters.iter().copied().collect()))
            .collect();
        let feed: VecDeque<FeedEntry> = snapshot.feed.iter().cloned().collect();
        let next_seq = feed.iter().map(|e| e.seq + 1).max().unwrap_or(0);
        FollowState {
            away,
            seen,
            votes,
            feed,
            next_seq,
        }
    }

    /// Recopie l'état dans un instantané de sauvegarde.
    pub fn fill_snapshot(&self, snapshot: &mut Snapshot) {
        snapshot.away = self
            .away
            .iter()
            .map(|((team, slot), a)| SlotAway {
                team: *team,
                slot: *slot,
                since: a.since,
                message: a.message.clone(),
            })
            .collect();
        snapshot.seen = self
            .seen
            .iter()
            .map(|((team, slot), s)| SlotSeen {
                team: *team,
                slot: *slot,
                items: u64::try_from(s.items).unwrap_or(u64::MAX),
                time: s.time,
            })
            .collect();
        snapshot.release_votes = self
            .votes
            .iter()
            .map(|((team, target), voters)| ReleaseVotes {
                team: *team,
                target: *target,
                voters: voters.iter().copied().collect(),
            })
            .collect();
        snapshot.feed = self.feed.iter().cloned().collect();
    }

    /// Ajoute une entrée au fil ; les plus anciennes tombent au-delà de `FEED_MAX`.
    pub fn push(&mut self, time: f64, team: TeamId, slot: SlotId, event: FeedKind) {
        self.feed.push_back(FeedEntry {
            seq: self.next_seq,
            time,
            team,
            slot,
            event,
        });
        self.next_seq += 1;
        while self.feed.len() > FEED_MAX {
            self.feed.pop_front();
        }
    }

    /// Entrées du fil arrivées après `time`, sauf celles du slot lui-même.
    pub fn feed_since(&self, time: f64, team: TeamId, slot: SlotId) -> Vec<&FeedEntry> {
        self.feed
            .iter()
            .filter(|e| e.time > time && e.team == team && e.slot != slot)
            .collect()
    }

    /// Nombre de votes contre ce slot.
    pub fn vote_count(&self, team: TeamId, target: SlotId) -> usize {
        self.votes.get(&(team, target)).map_or(0, BTreeSet::len)
    }
}

/// Voix nécessaires : la majorité stricte des votants possibles.
/// 0 votant possible (partie solo) : 0, c'est-à-dire « pas de vote ».
pub(crate) fn needed_votes(eligible: usize) -> usize {
    if eligible == 0 {
        0
    } else {
        eligible / 2 + 1
    }
}

/// Durée lisible en anglais, comme les autres messages du serveur :
/// `less than a minute`, `5 min`, `2 h 05 min`, `3 days 4 h`.
pub(crate) fn duration_text(seconds: f64) -> String {
    // Une horloge qui recule donne une durée négative : on affiche 0.
    // `as u64` sur un f64 : la partie entière, bornée (pas de panique).
    let total = if seconds.is_finite() && seconds > 0.0 {
        seconds as u64
    } else {
        0
    };
    let minutes = total / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if minutes == 0 {
        "less than a minute".to_string()
    } else if hours == 0 {
        format!("{minutes} min")
    } else if days == 0 {
        format!("{hours} h {:02} min", minutes % 60)
    } else {
        let unit = if days == 1 { "day" } else { "days" };
        format!("{days} {unit} {} h", hours % 24)
    }
}

/// « A, B, C and 4 more » : `max` noms au plus.
pub(crate) fn name_list(names: &[String], max: usize) -> String {
    let shown: Vec<&str> = names.iter().take(max).map(String::as_str).collect();
    let rest = names.len().saturating_sub(max);
    if rest == 0 {
        match shown.split_last() {
            Some((last, [])) => (*last).to_string(),
            Some((last, others)) => format!("{} and {last}", others.join(", ")),
            None => String::new(),
        }
    } else {
        format!("{} and {rest} more", shown.join(", "))
    }
}

/// Vrai si `text` cite `name` (casse ignorée). Un nom d'un seul caractère
/// n'est jamais reconnu : il se trouverait dans presque tous les messages.
pub(crate) fn mentions(text: &str, name: &str) -> bool {
    name.chars().count() >= 2 && text.to_lowercase().contains(&name.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn majority() {
        assert_eq!(needed_votes(0), 0);
        assert_eq!(needed_votes(1), 1);
        assert_eq!(needed_votes(2), 2);
        assert_eq!(needed_votes(3), 2);
        assert_eq!(needed_votes(4), 3);
    }

    #[test]
    fn durations() {
        assert_eq!(duration_text(-5.0), "less than a minute");
        assert_eq!(duration_text(59.0), "less than a minute");
        assert_eq!(duration_text(300.0), "5 min");
        assert_eq!(duration_text(2.0 * 3600.0 + 5.0 * 60.0), "2 h 05 min");
        assert_eq!(duration_text(86400.0 + 3600.0), "1 day 1 h");
        assert_eq!(duration_text(3.0 * 86400.0), "3 days 0 h");
        assert_eq!(duration_text(f64::NAN), "less than a minute");
    }

    #[test]
    fn lists() {
        let names: Vec<String> = ["A", "B", "C"].iter().map(|s| s.to_string()).collect();
        assert_eq!(name_list(&names[..1], 10), "A");
        assert_eq!(name_list(&names, 10), "A, B and C");
        assert_eq!(name_list(&names, 2), "A, B and 1 more");
        assert_eq!(name_list(&[], 2), "");
    }

    #[test]
    fn mention() {
        assert!(mentions("where is BOB ?", "Bob"));
        assert!(!mentions("hello", "Bob"));
        assert!(!mentions("a b c", "a"));
    }

    #[test]
    fn feed_is_bounded_and_numbered() {
        let mut state = FollowState::default();
        for i in 0..(FEED_MAX + 3) {
            state.push(i as f64, 0, 1, FeedKind::Goal);
        }
        assert_eq!(state.feed.len(), FEED_MAX);
        assert_eq!(state.feed.front().map(|e| e.seq), Some(3));
        // Reprise : la numérotation continue après la dernière entrée.
        let mut snapshot = Snapshot::default();
        state.fill_snapshot(&mut snapshot);
        let mut again = FollowState::from_snapshot(&snapshot);
        again.push(0.0, 0, 1, FeedKind::Back);
        assert_eq!(again.feed.back().map(|e| e.seq), Some(FEED_MAX as u64 + 3));
    }

    #[test]
    fn snapshot_roundtrip() {
        let mut state = FollowState::default();
        state.away.insert(
            (0, 2),
            Away {
                since: 1.0,
                message: "brb".into(),
            },
        );
        state.seen.insert(
            (0, 1),
            Seen {
                items: 4,
                time: 2.0,
            },
        );
        state.votes.insert((0, 2), [1, 3].into_iter().collect());
        state.push(3.0, 0, 2, FeedKind::Back);
        let mut snapshot = Snapshot::default();
        state.fill_snapshot(&mut snapshot);
        let again = FollowState::from_snapshot(&snapshot);
        assert_eq!(again.away, state.away);
        assert_eq!(again.seen, state.seen);
        assert_eq!(again.votes, state.votes);
        assert_eq!(again.feed, state.feed);
        assert_eq!(again.feed_since(2.5, 0, 1).len(), 1);
        assert!(again.feed_since(2.5, 0, 2).is_empty());
    }
}
