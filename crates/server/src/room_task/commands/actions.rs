//! Commandes du chat qui changent l'état de la partie : `!release`, `!collect`
//! (`_cmd_release`, `_cmd_collect` de `ClientMessageProcessor`, MultiServer.py).
//!
//! Le travail lui-même (`release_player`, `collect_player`) est dans `play` :
//! il sert aussi au relâchement automatique quand un joueur atteint son but.

use super::*;
use crate::fuzzy::intended_text;

/// Ce que permet un mode `release_mode` / `collect_mode` pour un joueur.
#[derive(Debug, PartialEq, Eq)]
enum Permission {
    Allowed,
    Disabled,
    NeedsGoal,
}

/// Lecture d'un mode comme le serveur officiel : `"enabled" in mode`, puis
/// `"disabled" in mode`, sinon « auto » ou « goal ». L'ordre compte :
/// `auto-enabled` contient `enabled`, donc il autorise toujours.
fn permission(mode: &str, has_goal: bool) -> Permission {
    if mode.contains("enabled") {
        Permission::Allowed
    } else if mode.contains("disabled") {
        Permission::Disabled
    } else if has_goal {
        Permission::Allowed
    } else {
        Permission::NeedsGoal
    }
}

/// Durée de `!countdown`, lue comme `int(seconds, 10)` en Python : un texte qui
/// n'est pas un nombre donne 10 ; plus d'une heure est refusé.
fn parse_countdown(seconds: &str) -> Result<i64, String> {
    let text = seconds.trim();
    let invalid = |shown: &str| format!("{shown} is invalid. Maximum is 1 hour.");
    match text.parse::<i64>() {
        Ok(timer) if timer > 60 * 60 => Err(invalid(&timer.to_string())),
        Ok(timer) => Ok(timer),
        Err(_) => {
            // Trop grand pour un i64 mais bien un nombre : Python l'aurait lu,
            // puis refusé (plus d'une heure).
            let digits = text.strip_prefix('+').unwrap_or(text);
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                Err(invalid(digits.trim_start_matches('0')))
            } else {
                Ok(10)
            }
        }
    }
}

impl RoomTask {
    /// Vrai si le slot a déjà annoncé avoir atteint son but.
    fn has_goal(&self, team: TeamId, slot: SlotId) -> bool {
        self.game.status(team, slot) == Some(ClientStatus::GOAL)
    }

    /// `!release` : envoie aux autres les objets encore cachés dans son monde.
    pub(super) fn cmd_release(&mut self, id: ClientId) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        // `allow_releases` (autorisation donnée par la console du serveur)
        // n'existe pas encore dans Raftup : la liste est toujours vide.
        let mode = self.room.data().server_options.release_mode.clone();
        match permission(&mode, self.has_goal(player.team, player.slot)) {
            Permission::Allowed => self.release_player(player.team, player.slot),
            Permission::Disabled => self.output(
                id,
                "Sorry, client item releasing has been disabled on this server. \
                 You can ask the server admin for a /release",
            ),
            Permission::NeedsGoal => self.output(
                id,
                "Sorry, client item releasing requires you to have beaten the game on this server. \
                 You can ask the server admin for a /release",
            ),
        }
    }

    /// `!collect` : récupère ses objets encore cachés dans les autres mondes.
    pub(super) fn cmd_collect(&mut self, id: ClientId) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        let mode = self.room.data().server_options.collect_mode.clone();
        match permission(&mode, self.has_goal(player.team, player.slot)) {
            Permission::Allowed => self.collect_player(player.team, player.slot, false),
            Permission::Disabled => self.output(
                id,
                "Sorry, client collecting has been disabled on this server. \
                 You can ask the server admin for a /collect",
            ),
            Permission::NeedsGoal => self.output(
                id,
                "Sorry, client collecting requires you to have beaten the game on this server. \
                 You can ask the server admin for a /collect",
            ),
        }
    }

    /// `!alias [surnom]` : choisit un surnom, ou retire le sien sans argument.
    /// Comme le serveur officiel : 16 caractères au plus, espaces retirés
    /// APRÈS la coupe (`alias_name[:16].strip()`).
    pub(super) fn cmd_alias(&mut self, id: ClientId, argument: &str) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        let (team, slot) = (player.team, player.slot);
        if !argument.is_empty() {
            let alias = truncate_chars(argument, 16).trim().to_string();
            self.output(id, format!("Hello, {alias}"));
            self.game.set_alias(team, slot, alias);
        } else if self.game.remove_alias(team, slot) {
            self.output(id, "Removed Alias");
        } else {
            // Rien à retirer : le serveur officiel ne répond rien.
            return;
        }
        self.update_aliases(team);
    }

    /// `!getitem <objet>` : triche, donne un objet de son propre jeu si la
    /// salle le permet (`item_cheat`). Le nom est cherché comme `!hint` : à
    /// l'identique, sinon le plus proche s'il est assez sûr.
    pub(super) fn cmd_getitem(&mut self, id: ClientId, argument: &str) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        if self.room.data().server_options.disable_item_cheat {
            self.output(id, "Cheating is disabled.");
            return;
        }
        let data = self.room.data();
        let game = data
            .slots
            .get(&player.slot)
            .map(|s| s.game.clone())
            .unwrap_or_default();
        // Jeu absent du datapackage : le serveur officiel plante.
        let Some(package) = data.datapackage.get(&game) else {
            self.invalid_arguments(id, "getitem");
            return;
        };
        // Les candidats gardent l'ordre du datapackage : en cas d'égalité de
        // score, le premier gagne, comme en Python.
        let names: Vec<&str> = package.item_name_to_id.keys().map(String::as_str).collect();
        let (name, usable, response) = intended_text(argument, &names);
        let item_id = package.item_name_to_id.get(&name).copied();
        let Some(item_id) = item_id.filter(|_| usable) else {
            self.output(id, response);
            return;
        };
        let item = NetworkItem {
            item: item_id,
            location: -1,
            player: player.slot,
            flags: ItemFlags(0),
        };
        self.game.add_cheat_item(player.team, player.slot, item);
        let text = format!(
            "Cheat console: sending \"{name}\" to {}",
            self.aliased_name(player.team, player.slot)
        );
        let mut message = text_message(text);
        message.kind = Some("ItemCheat".to_string());
        message.team = Some(player.team);
        message.receiving = Some(player.slot);
        message.item = Some(item);
        self.broadcast_text_all(message);
        self.send_new_items();
    }

    /// `!countdown [secondes]` : compte à rebours annoncé à tous (10 s par défaut).
    pub(super) fn cmd_countdown(&mut self, id: ClientId, seconds: &str) {
        let mode = &self.room.data().server_options.countdown_mode;
        // `len(player_names)` : tous les slots, groupes compris.
        let crowded = self.room.data().slots.len() >= 30;
        if mode == "disabled" || (mode == "auto" && crowded) {
            self.output(
                id,
                "Sorry, client countdowns have been disabled on this server. \
                 You can ask the server admin for a /countdown",
            );
            return;
        }
        let timer = match parse_countdown(seconds) {
            Ok(timer) => timer,
            // Python lève une erreur et renvoie sa trace : on renvoie seulement
            // le message (écart déclaré, ADR 0013).
            Err(text) => {
                self.output(id, text);
                return;
            }
        };
        self.start_countdown(timer);
    }

    /// `countdown()` du serveur officiel : annonce le départ ; si un compte à
    /// rebours tourne déjà, il repart simplement de `timer`.
    pub(super) fn start_countdown(&mut self, timer: i64) {
        self.broadcast_countdown(format!("[Server]: Starting countdown of {timer}s"), timer);
        self.countdown_timer = timer;
        if self.countdown_next.is_none() {
            // Première annonce tout de suite, comme la boucle Python.
            self.countdown_step();
        }
    }

    /// Un tour de la boucle Python : annonce le nombre et attend 1 s, ou
    /// annonce « GO » et s'arrête quand il ne reste rien.
    pub(crate) fn countdown_step(&mut self) {
        if self.countdown_timer > 0 {
            let n = self.countdown_timer;
            self.broadcast_countdown(format!("[Server]: {n}"), n);
            self.countdown_timer -= 1;
            self.countdown_next =
                Some(tokio::time::Instant::now() + std::time::Duration::from_secs(1));
        } else {
            self.broadcast_countdown("[Server]: GO".to_string(), 0);
            self.countdown_timer = 0;
            self.countdown_next = None;
        }
    }

    fn broadcast_countdown(&self, text: String, countdown: i64) {
        let mut message = text_message(text);
        message.kind = Some("Countdown".to_string());
        message.countdown = Some(countdown);
        self.broadcast_text_all(message);
    }

    /// Envoie la nouvelle liste des joueurs (avec les surnoms) à tous les
    /// clients de l'équipe, même ceux qui ne veulent pas de texte
    /// (`update_aliases` du serveur officiel).
    fn update_aliases(&self, team: TeamId) {
        let update = RoomUpdate {
            players: Some(self.players()),
            ..RoomUpdate::default()
        };
        self.broadcast(&[ServerPacket::RoomUpdate(update)], |p| p.team == team);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_like_python() {
        assert_eq!(permission("enabled", false), Permission::Allowed);
        assert_eq!(permission("auto-enabled", false), Permission::Allowed);
        assert_eq!(permission("disabled", true), Permission::Disabled);
        assert_eq!(permission("auto", false), Permission::NeedsGoal);
        assert_eq!(permission("auto", true), Permission::Allowed);
        assert_eq!(permission("goal", false), Permission::NeedsGoal);
        assert_eq!(permission("goal", true), Permission::Allowed);
    }
}

#[cfg(test)]
mod countdown_tests {
    use super::*;

    #[test]
    fn duration_like_python_int() {
        assert_eq!(parse_countdown("5"), Ok(5));
        assert_eq!(parse_countdown(" +7 "), Ok(7));
        assert_eq!(parse_countdown("-3"), Ok(-3));
        assert_eq!(parse_countdown("abc"), Ok(10));
        assert_eq!(parse_countdown("3600"), Ok(3600));
        assert_eq!(
            parse_countdown("3601"),
            Err("3601 is invalid. Maximum is 1 hour.".to_string())
        );
        assert_eq!(
            parse_countdown("99999999999999999999999"),
            Err("99999999999999999999999 is invalid. Maximum is 1 hour.".to_string())
        );
    }
}
