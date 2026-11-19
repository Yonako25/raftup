//! Suivi asynchrone dans la tâche salle (ADR 0031) : commandes `!away`,
//! `!back`, `!summary`, `!vote_release`, résumé de retour, réponse
//! automatique aux messages qui citent un absent, clés `_read_raftup_…`.
//!
//! Tout passe par ce qu'un client Archipelago sait déjà faire : des messages
//! `PrintJSON` et des clés du stockage. Rien ne se passe si la salle n'a pas
//! l'option (`Room::async_follow`).

use super::*;
use crate::command_line::truncate_chars;
use crate::follow::{
    duration_text, mentions, name_list, needed_votes, Away, Seen, AWAY_MESSAGE_MAX,
};
use crate::fuzzy::intended_text;
use multidata::raftup::{FeedEntry, FeedKind};

/// Objets nommés au plus dans le résumé de retour.
const SUMMARY_ITEMS: usize = 10;
/// Entrées du fil montrées au plus (résumé et `!summary`).
const SUMMARY_FEED: usize = 10;

impl RoomTask {
    /// Vrai si la salle a le suivi asynchrone.
    pub(super) fn follow_on(&self) -> bool {
        self.room.async_follow()
    }

    /// Vrai pour un slot de joueur (pas un groupe ni un spectateur).
    fn is_player_slot(&self, slot: SlotId) -> bool {
        self.room
            .data()
            .slots
            .get(&slot)
            .is_some_and(|s| s.slot_type == SlotType::PLAYER)
    }

    /// Slots de joueurs de l'équipe, dans l'ordre des numéros.
    fn player_slots(&self) -> Vec<SlotId> {
        let slots = &self.room.data().slots;
        slots
            .iter()
            .filter(|(_, info)| info.slot_type == SlotType::PLAYER)
            .map(|(slot, _)| *slot)
            .collect()
    }

    /// Ajoute une entrée au fil d'activité (seulement avec l'option).
    pub(super) fn feed(&mut self, team: TeamId, slot: SlotId, event: FeedKind) {
        if self.follow_on() {
            self.follow.push(now(), team, slot, event);
        }
    }

    /// Annonce à tous, texte simple sans `type` : tout client l'affiche.
    fn follow_broadcast(&self, text: String) {
        self.broadcast_text_all(text_message(text));
    }

    // --- Absence ---------------------------------------------------------

    /// `!away [message]` : se déclarer absent.
    pub(super) fn cmd_away(&mut self, id: ClientId, message: &str) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        let message = truncate_chars(message.trim(), AWAY_MESSAGE_MAX)
            .trim()
            .to_string();
        let away = Away {
            since: now(),
            message: message.clone(),
        };
        self.follow.away.insert((player.team, player.slot), away);
        self.feed(
            player.team,
            player.slot,
            FeedKind::Away {
                message: message.clone(),
            },
        );
        self.output(
            id,
            "You are now marked as away. Use !back when you are back.",
        );
        let shown = self.aliased_name(player.team, player.slot);
        let text = if message.is_empty() {
            format!("{shown} is away.")
        } else {
            format!("{shown} is away: {message}")
        };
        self.follow_broadcast(text);
        self.emit(GameEvent::Away {
            time: now(),
            slot: player.slot,
            name: self.slot_name(player.slot),
            message,
        });
        self.notify_follow_keys(player.team);
    }

    /// `!back` : lever son absence.
    pub(super) fn cmd_back(&mut self, id: ClientId) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        if !self.clear_away(player.team, player.slot) {
            self.output(id, "You are not marked as away.");
        }
    }

    /// Lève l'absence d'un slot et l'annonce. Faux s'il n'était pas absent.
    fn clear_away(&mut self, team: TeamId, slot: SlotId) -> bool {
        if self.follow.away.remove(&(team, slot)).is_none() {
            return false;
        }
        self.feed(team, slot, FeedKind::Back);
        let shown = self.aliased_name(team, slot);
        self.follow_broadcast(format!("{shown} is back."));
        self.emit(GameEvent::Back {
            time: now(),
            slot,
            name: self.slot_name(slot),
        });
        self.notify_follow_keys(team);
        true
    }

    /// Un message de chat cite un absent : son auteur, et lui seul, reçoit le
    /// message d'absence. Appelé pour le chat simple, pas pour les commandes.
    pub(super) fn reply_for_away(&self, id: ClientId, text: &str) {
        let Some(player) = self.player(id) else {
            return;
        };
        if !self.follow_on() || text.trim_start().starts_with('!') {
            return;
        }
        for ((team, slot), away) in &self.follow.away {
            if (*team, *slot) == (player.team, player.slot) || *team != player.team {
                continue;
            }
            let name = self.slot_name(*slot);
            let alias = self.game.alias(*team, *slot).unwrap_or_default();
            if !mentions(text, &name) && !mentions(text, alias) {
                continue;
            }
            let since = duration_text(now() - away.since);
            let shown = self.aliased_name(*team, *slot);
            let reply = if away.message.is_empty() {
                format!("{shown} is away (for {since}).")
            } else {
                format!("{shown} is away (for {since}): {}", away.message)
            };
            self.notify(id, text_message(reply));
        }
    }

    /// Le slot vient de visiter un nouveau lieu lui-même : il joue de nouveau.
    /// Son absence est levée et les votes contre lui sont annulés.
    pub(super) fn on_player_played(&mut self, team: TeamId, slot: SlotId) {
        if !self.follow_on() {
            return;
        }
        self.clear_away(team, slot);
        if self.follow.votes.remove(&(team, slot)).is_some() {
            let shown = self.aliased_name(team, slot);
            self.follow_broadcast(format!(
                "The vote to release {shown}'s world is cancelled: {shown} is playing again."
            ));
            self.notify_follow_keys(team);
        }
    }

    // --- Départs et résumé de retour --------------------------------------

    /// Le dernier client d'un slot est parti : on note l'heure et les objets reçus.
    pub(super) fn note_seen(&mut self, team: TeamId, slot: SlotId) {
        if !self.follow_on() {
            return;
        }
        let items = self.game.received(team, slot, true).len();
        self.follow
            .seen
            .insert((team, slot), Seen { items, time: now() });
    }

    /// Arrêt propre : les slots encore connectés sont notés « vus ». Renvoie
    /// vrai si l'état a changé (il faut alors écrire la sauvegarde).
    pub(super) fn mark_connected_seen(&mut self) -> bool {
        if !self.follow_on() {
            return false;
        }
        let connected: BTreeSet<(TeamId, SlotId)> = self
            .clients
            .values()
            .filter_map(|c| c.player.as_ref())
            .map(|p| (p.team, p.slot))
            .collect();
        for (team, slot) in &connected {
            self.note_seen(*team, *slot);
        }
        if !connected.is_empty() {
            self.dirty = true;
        }
        !connected.is_empty()
    }

    /// Résumé envoyé au joueur qui revient (après le message `Tutorial`).
    /// Rien à la toute première connexion du slot.
    pub(super) fn send_return_summary(&self, id: ClientId, player: &Player) {
        if !self.follow_on() {
            return;
        }
        let (team, slot) = (player.team, player.slot);
        let Some(seen) = self.follow.seen.get(&(team, slot)).copied() else {
            return;
        };
        // Une autre connexion de ce slot était déjà là : le joueur n'était pas
        // vraiment parti, rien à résumer.
        let already_here = self.clients.iter().any(|(other, c)| {
            *other != id
                && c.player
                    .as_ref()
                    .is_some_and(|p| (p.team, p.slot) == (team, slot))
        });
        if already_here {
            return;
        }
        let mut lines = vec![format!(
            "Welcome back, {}! You were away for {}.",
            self.aliased_name(team, slot),
            duration_text(now() - seen.time)
        )];
        // Les objets reçus depuis le départ, dans l'ordre d'arrivée.
        let received = self.game.received(team, slot, true);
        let game = self.slot_game(slot);
        let names: Vec<String> = received
            .get(seen.items..)
            .unwrap_or_default()
            .iter()
            .map(|item| self.names.python_item(&game, item.item))
            .collect();
        if names.is_empty() {
            lines.push("You received no new items.".to_string());
        } else {
            let plural = if names.len() == 1 { "item" } else { "items" };
            lines.push(format!(
                "You received {} new {plural}: {}.",
                names.len(),
                name_list(&names, SUMMARY_ITEMS)
            ));
        }
        let entries = self.follow.feed_since(seen.time, team, slot);
        let skip = entries.len().saturating_sub(SUMMARY_FEED);
        for entry in entries.into_iter().skip(skip) {
            lines.push(self.feed_text(entry));
        }
        lines.extend(self.follow_status_lines(team, Some(slot)));
        let packets = lines
            .into_iter()
            .map(|text| ServerPacket::PrintJson(text_message(text)))
            .collect();
        if !player.no_text {
            self.send(id, packets);
        }
    }

    /// Lignes sur les absents et les votes en cours. `me` : le joueur qui
    /// lit, pour lui parler de son propre état.
    fn follow_status_lines(&self, team: TeamId, me: Option<SlotId>) -> Vec<String> {
        let mut lines = Vec::new();
        let others: Vec<String> = self
            .follow
            .away
            .iter()
            .filter(|((t, s), _)| *t == team && Some(*s) != me)
            .map(|((t, s), away)| {
                let since = duration_text(now() - away.since);
                let shown = self.aliased_name(*t, *s);
                if away.message.is_empty() {
                    format!("{shown} (for {since})")
                } else {
                    format!("{shown} (for {since}: {})", away.message)
                }
            })
            .collect();
        if !others.is_empty() {
            lines.push(format!("Currently away: {}.", others.join(", ")));
        }
        let needed = needed_votes(self.player_slots().len().saturating_sub(1));
        for ((t, target), voters) in &self.follow.votes {
            if *t != team {
                continue;
            }
            if Some(*target) == me {
                lines.push(format!(
                    "{} of {needed} needed votes to release your world. \
                     Check a location to cancel the vote.",
                    voters.len()
                ));
            } else {
                lines.push(format!(
                    "Vote to release {}'s world: {}/{needed}.",
                    self.aliased_name(*t, *target),
                    voters.len()
                ));
            }
        }
        if let Some(slot) = me {
            if self.follow.away.contains_key(&(team, slot)) {
                lines.push("You are still marked as away. Use !back to clear it.".to_string());
            }
        }
        lines
    }

    /// Une entrée du fil d'activité, en une phrase.
    fn feed_text(&self, entry: &FeedEntry) -> String {
        let who = self.aliased_name(entry.team, entry.slot);
        match &entry.event {
            FeedKind::Goal => format!("{who} completed their goal."),
            FeedKind::Release => format!("{who} released their world."),
            FeedKind::Away { message } if message.is_empty() => format!("{who} went away."),
            FeedKind::Away { message } => format!("{who} went away: {message}"),
            FeedKind::Back => format!("{who} came back."),
            FeedKind::ReleaseVote {
                voter,
                votes,
                needed,
            } => format!(
                "{} voted to release {who}'s world ({votes}/{needed}).",
                self.aliased_name(entry.team, *voter)
            ),
            FeedKind::ReleasedByVote => format!("{who}'s world was released by vote."),
        }
    }

    /// `!summary` : absents, votes et dernières entrées du fil.
    pub(super) fn cmd_summary(&self, id: ClientId) {
        let Some(player) = self.player(id) else {
            return;
        };
        let team = player.team;
        let mut lines: Vec<String> = Vec::new();
        let entries: Vec<&FeedEntry> = self.follow.feed.iter().filter(|e| e.team == team).collect();
        let skip = entries.len().saturating_sub(SUMMARY_FEED);
        for entry in entries.into_iter().skip(skip) {
            let ago = duration_text(now() - entry.time);
            lines.push(format!("[{ago} ago] {}", self.feed_text(entry)));
        }
        lines.extend(self.follow_status_lines(team, Some(player.slot)));
        if lines.is_empty() {
            lines.push("Nothing to report.".to_string());
        }
        self.output_multiple(id, lines);
    }

    // --- Release par vote --------------------------------------------------

    /// `!vote_release [joueur]` : voter pour relâcher le monde d'un joueur ;
    /// sans argument, la liste des votes en cours.
    pub(super) fn cmd_vote_release(&mut self, id: ClientId, argument: &str) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        let team = player.team;
        if argument.trim().is_empty() {
            let lines: Vec<String> = self
                .follow_status_lines(team, Some(player.slot))
                .into_iter()
                .filter(|l| l.contains("release"))
                .collect();
            if lines.is_empty() {
                self.output(id, "No vote in progress. Usage: !vote_release <player>");
            } else {
                self.output_multiple(id, lines);
            }
            return;
        }
        if !self.is_player_slot(player.slot) {
            self.output(id, "Only players can vote.");
            return;
        }
        let players = self.player_slots();
        let names: Vec<String> = players.iter().map(|s| self.slot_name(*s)).collect();
        let candidates: Vec<&str> = names.iter().map(String::as_str).collect();
        let (name, usable, response) = intended_text(argument.trim(), &candidates);
        if !usable {
            self.output(id, response);
            return;
        }
        let Some(target) = players.iter().copied().find(|s| self.slot_name(*s) == name) else {
            self.output(id, response);
            return;
        };
        let shown = self.aliased_name(team, target);
        if target == player.slot {
            self.output(
                id,
                "You can't vote for your own world. Use !release instead.",
            );
            return;
        }
        if self.game.status(team, target) == Some(ClientStatus::GOAL) {
            self.output(id, format!("{shown} has already completed their goal."));
            return;
        }
        let missing = self.game.missing(self.room.data(), team, target);
        if missing.is_none_or(|m| m.is_empty()) {
            self.output(id, format!("{shown}'s world has nothing left to release."));
            return;
        }
        // Votent les autres joueurs : la majorité stricte d'entre eux.
        let needed = needed_votes(players.len().saturating_sub(1));
        let voters = self.follow.votes.entry((team, target)).or_default();
        if !voters.insert(player.slot) {
            let count = voters.len();
            self.output(
                id,
                format!("You already voted to release {shown}'s world ({count}/{needed})."),
            );
            return;
        }
        let votes = voters.len();
        let passed = votes >= needed;
        // `u32::try_from` : un nombre de joueurs qui ne tient pas sur 32 bits
        // n'existe pas, mais on borne au lieu de paniquer.
        let as_u32 = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        self.feed(
            team,
            target,
            FeedKind::ReleaseVote {
                voter: player.slot,
                votes: as_u32(votes),
                needed: as_u32(needed),
            },
        );
        let voter_shown = self.aliased_name(team, player.slot);
        self.follow_broadcast(format!(
            "{voter_shown} voted to release {shown}'s world ({votes}/{needed})."
        ));
        self.emit(GameEvent::ReleaseVote {
            time: now(),
            target,
            target_name: self.slot_name(target),
            voter_name: self.slot_name(player.slot),
            votes: as_u32(votes),
            needed: as_u32(needed),
            passed,
        });
        if passed {
            self.follow.votes.remove(&(team, target));
            self.feed(team, target, FeedKind::ReleasedByVote);
            self.follow_broadcast(format!("The vote passed: releasing {shown}'s world."));
            self.release_world(team, target);
        }
        self.notify_follow_keys(team);
    }

    // --- Repli par le stockage ---------------------------------------------

    /// Clés `_read_raftup_away_<équipe>` et `_read_raftup_release_votes_<équipe>`.
    /// `None` (donc `null`) sans l'option, comme une clé inconnue.
    pub(super) fn read_follow_key(&self, name: &str) -> Option<Value> {
        if !self.follow_on() {
            return None;
        }
        if let Some(rest) = name.strip_prefix("raftup_away_") {
            let team = parse_exact(rest)?;
            let mut map = Map::new();
            for ((t, slot), away) in &self.follow.away {
                if *t == team {
                    map.insert(
                        slot.to_string(),
                        serde_json::json!({"since": away.since, "message": away.message}),
                    );
                }
            }
            return Some(Value::Object(map));
        }
        let rest = name.strip_prefix("raftup_release_votes_")?;
        let team = parse_exact(rest)?;
        let mut map = Map::new();
        for ((t, target), voters) in &self.follow.votes {
            if *t == team {
                let list: Vec<Value> = voters.iter().map(|v| Value::from(*v)).collect();
                map.insert(target.to_string(), Value::Array(list));
            }
        }
        Some(Value::Object(map))
    }

    /// Prévient les abonnés (`SetNotify`) des deux clés de l'équipe.
    fn notify_follow_keys(&self, team: TeamId) {
        for prefix in ["raftup_away_", "raftup_release_votes_"] {
            let name = format!("{prefix}{team}");
            if let Some(value) = self.read_follow_key(&name) {
                self.notify_read_key(format!("_read_{name}"), value);
            }
        }
    }
}
