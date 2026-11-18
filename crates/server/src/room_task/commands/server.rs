//! Commandes serveur de `!admin` : `ServerCommandProcessor` de `MultiServer.py`
//! (Archipelago 0.6.8). Après `!admin login`, `!admin /send Alice Épée` envoie
//! la ligne `/send Alice Épée` à ce processeur ; les réponses partent vers
//! l'administrateur seul, avec le genre `AdminCommandResult`.
//!
//! Écarts déclarés (ADR 0013) : une erreur d'arguments donne un texte court à
//! la place de la trace Python ; les commandes non reprises (voir
//! `UNSUPPORTED`) sont refusées avec un message clair.

use super::*;
use crate::command_line::{py_isnumeric, raw_argument, split_command};
use crate::fuzzy::intended_text;
use crate::game::slot_set;

/// Commandes du serveur officiel, dans son ordre (celui du dictionnaire
/// `commands` : classes parentes d'abord, puis `ServerCommandProcessor`).
const SERVER_COMMANDS: [&str; 20] = [
    "help",
    "license",
    "options",
    "save",
    "players",
    "status",
    "exit",
    "alias",
    "collect",
    "countdown",
    "release",
    "allow_release",
    "forbid_release",
    "send_multiple",
    "send",
    "send_location",
    "hint",
    "hint_location",
    "option",
    "datastore",
];

/// Commandes connues mais pas encore reprises par Raftup.
const UNSUPPORTED: [&str; 7] = [
    "help",
    "license",
    "options",
    "exit",
    "alias",
    "option",
    "datastore",
];

/// Commandes qui reçoivent le texte brut après le premier mot (`@mark_raw`).
fn is_raw(command: &str) -> bool {
    matches!(
        command,
        "alias" | "collect" | "release" | "allow_release" | "forbid_release"
    )
}

/// Résultat d'une recherche : `Err(Some(texte))` est une réponse à renvoyer
/// telle quelle, `Err(None)` un cas où Python planterait.
type Found<T> = Result<T, Option<String>>;

/// Numéro écrit en chiffres (`int(texte)` après `isnumeric()`). Un nombre trop
/// grand ne correspond à rien : `i64::MAX`. `None` pour des chiffres non ASCII,
/// que Python accepte à `isnumeric()` mais pas toujours à `int()`.
fn numeric_id(text: &str) -> Option<i64> {
    if !text.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(text.parse::<i64>().unwrap_or(i64::MAX))
}

impl RoomTask {
    /// Réponse à l'administrateur (`ServerCommandProcessor.output`).
    fn admin_output(&self, id: ClientId, text: impl Into<String>) {
        let mut message = text_message(text.into());
        message.kind = Some("AdminCommandResult".to_string());
        self.notify(id, message);
    }

    fn admin_invalid(&self, id: ClientId, command: &str) {
        self.admin_output(id, format!("Invalid arguments for /{command}."));
    }

    /// `ctx.commandprocessor(raw)` : exécute une ligne de commande serveur.
    pub(super) fn run_server_command(&mut self, id: ClientId, raw: &str) {
        let words = split_command(raw);
        // Ligne vide ou mot vide : Python plante (IndexError) ; nous ne répondons rien.
        let Some(first) = words.first().filter(|w| !w.is_empty()) else {
            return;
        };
        let Some(name) = first.strip_prefix('/') else {
            // `default` : du texte sans `/` est diffusé comme message du serveur.
            let mut message = text_message(format!("[Server]: {raw}"));
            message.kind = Some("ServerChat".to_string());
            message.message = Some(raw.to_string());
            self.broadcast_text_all(message);
            return;
        };
        let command = name.to_lowercase();
        if !SERVER_COMMANDS.contains(&command.as_str()) {
            self.admin_output(
                id,
                format!(
                    "Could not find command {name}. Known commands: {}",
                    SERVER_COMMANDS.join(", ")
                ),
            );
            return;
        }
        if UNSUPPORTED.contains(&command.as_str()) {
            self.admin_output(
                id,
                format!("Server command /{command} is not available in Raftup yet."),
            );
            return;
        }
        if is_raw(&command) {
            match raw_argument(raw) {
                Some(argument) => self.run_raw_server_command(id, &command, &argument),
                // Argument obligatoire : Python lève un `TypeError`.
                None => self.admin_invalid(id, &command),
            }
        } else {
            let args: Vec<&str> = words[1..].iter().map(String::as_str).collect();
            self.run_word_server_command(id, &command, &args);
        }
    }

    /// Commandes à arguments découpés mot à mot.
    fn run_word_server_command(&mut self, id: ClientId, command: &str, args: &[&str]) {
        // Nombre d'arguments que la méthode Python accepte (minimum, maximum).
        let (min, max) = match command {
            "save" | "players" => (0, 0),
            "status" | "countdown" => (0, 1),
            "send_multiple" => (2, usize::MAX),
            _ => (1, usize::MAX),
        };
        if args.len() < min || args.len() > max {
            self.admin_invalid(id, command);
            return;
        }
        match command {
            "save" => self.srv_save(id),
            "players" => {
                let (text, _) = self.players_text();
                self.admin_output(id, text);
            }
            "status" => self.srv_status(id, args.first().copied().unwrap_or("")),
            "countdown" => self.srv_countdown(id, args.first().copied().unwrap_or("10")),
            "send" => self.srv_send(id, "send", "1", args[0], &args[1..]),
            "send_multiple" => self.srv_send(id, "send_multiple", args[0], args[1], &args[2..]),
            "send_location" => self.srv_send_location(id, args[0], &args[1..]),
            "hint" => self.srv_hint(id, args[0], &args[1..], false),
            "hint_location" => self.srv_hint(id, args[0], &args[1..], true),
            _ => debug!(id, command, "commande serveur sans code"),
        }
    }

    /// Commandes qui lisent le texte brut (le nom du joueur, espaces compris).
    fn run_raw_server_command(&mut self, id: ClientId, command: &str, argument: &str) {
        let Some((team, slot, name)) = self.resolve_player(argument) else {
            let text = match command {
                "collect" => format!("Could not find player {argument} to collect"),
                "release" => format!("Could not find player {argument} to release"),
                "allow_release" => {
                    format!("Could not find player {argument} to allow the !release command for.")
                }
                _ => {
                    format!("Could not find player {argument} to forbid the !release command for.")
                }
            };
            self.admin_output(id, text);
            return;
        };
        match command {
            "collect" => self.collect_player(team, slot, false),
            "release" => self.release_player(team, slot),
            "allow_release" => {
                self.allow_releases.insert((team, slot));
                self.admin_output(
                    id,
                    format!(
                        "Player {name} is now allowed to use the !release command at any time."
                    ),
                );
            }
            _ => {
                self.allow_releases.remove(&(team, slot));
                self.admin_output(
                    id,
                    format!(
                        "Player {name} has to follow the server restrictions on use of the \
                         !release command."
                    ),
                );
            }
        }
    }

    /// Tous les (équipe, slot) de la partie avec leur nom, triés par (équipe, slot).
    fn player_entries(&self) -> Vec<(TeamId, SlotId, String)> {
        let keys: BTreeSet<(TeamId, SlotId)> =
            self.room.data().connect_names.values().copied().collect();
        keys.into_iter()
            .map(|(team, slot)| (team, slot, self.slot_name(slot)))
            .collect()
    }

    /// `resolve_player` : le nom exact, sinon le seul nom qui ne diffère que
    /// par la casse (deux candidats : ambigu, donc rien).
    fn resolve_player(&self, input: &str) -> Option<(TeamId, SlotId, String)> {
        let entries = self.player_entries();
        if let Some(found) = entries.iter().find(|(_, _, name)| name == input) {
            return Some(found.clone());
        }
        let lowered = input.to_lowercase();
        let mut matches = entries
            .into_iter()
            .filter(|(_, _, name)| name.to_lowercase() == lowered);
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first)
    }

    /// Nom de joueur approché (`get_intended_text` sur `player_names`), puis
    /// son (équipe, slot) : la table officielle ne connaît que l'équipe 0.
    fn find_player(&self, input: &str) -> Found<(TeamId, SlotId)> {
        let slots = &self.room.data().slots;
        let names: Vec<&str> = slots.values().map(|s| s.name.as_str()).collect();
        let (name, usable, response) = intended_text(input, &names);
        if !usable {
            return Err(Some(response));
        }
        let slot = slots
            .iter()
            .find(|(_, info)| info.name == name)
            .map(|(slot, _)| *slot);
        slot.map(|slot| (0, slot)).ok_or(None)
    }

    /// `/save` : l'état est écrit à la fin de chaque événement ; on force
    /// seulement une écriture, et on répond comme l'officiel.
    fn srv_save(&mut self, id: ClientId) {
        if self.save.is_some() {
            self.dirty = true;
            self.admin_output(id, "Game saved");
        } else {
            self.admin_output(id, "Saving is disabled.");
        }
    }

    /// `/status [tag]` : une réponse par équipe.
    fn srv_status(&self, id: ClientId, tag: &str) {
        let mut teams: BTreeSet<TeamId> = self
            .room
            .data()
            .connect_names
            .values()
            .map(|(team, _)| *team)
            .collect();
        teams.insert(0);
        for team in teams {
            self.admin_output(id, self.status_text(team, tag));
        }
    }

    /// `/countdown [secondes]` : comme `!countdown`, sans tenir compte du mode.
    fn srv_countdown(&mut self, id: ClientId, seconds: &str) {
        match super::actions::parse_countdown(seconds) {
            Ok(timer) => self.start_countdown(timer),
            // Python lève une erreur : message court à la place de la trace.
            Err(text) => self.admin_output(id, text),
        }
    }

    /// `/send` et `/send_multiple` : donne `amount` fois un objet à un joueur.
    fn srv_send(&mut self, id: ClientId, command: &str, amount: &str, player: &str, item: &[&str]) {
        match self.send_items(amount, player, &item.join(" ")) {
            Ok(()) => {}
            Err(Some(text)) => self.admin_output(id, text),
            Err(None) => self.admin_invalid(id, command),
        }
    }

    fn send_items(&mut self, amount: &str, player: &str, item: &str) -> Found<()> {
        let (team, slot) = self.find_player(player)?;
        let data = self.room.data();
        let game = self.slot_game(slot);
        // Jeu absent du datapackage : le serveur officiel plante.
        let package = data.datapackage.get(&game).ok_or(None)?;
        let names: Vec<&str> = package.item_name_to_id.keys().map(String::as_str).collect();
        let (name, usable, response) = intended_text(item, &names);
        let item_id = package.item_name_to_id.get(&name).copied();
        let Some(item_id) = item_id.filter(|_| usable) else {
            return Err(Some(response));
        };
        // `int(amount)` : n'importe quel texte qui n'est pas un entier plante.
        let count: i64 = amount.trim().parse().map_err(|_| None)?;
        if count > 100 {
            return Err(Some(format!("{count} is invalid. Maximum is 100.")));
        }
        let new_item = NetworkItem {
            item: item_id,
            location: -1,
            player: 0,
            flags: ItemFlags(0),
        };
        // `send_items_to` : un groupe donne l'objet à chacun de ses membres.
        for target in slot_set(data, slot) {
            for _ in 0..count.max(0) {
                self.game.add_cheat_item(team, target, new_item);
            }
        }
        self.send_new_items();
        let how_many = if count == 1 {
            String::new()
        } else {
            format!("{count} of ")
        };
        let text = format!(
            "Cheat console: sending {how_many}\"{name}\" to {}",
            self.aliased_name(team, slot)
        );
        self.broadcast_text_all(text_message(text));
        Ok(())
    }

    /// `/send_location <joueur> <lieu>` : le lieu est visité comme si le joueur l'avait fait.
    fn srv_send_location(&mut self, id: ClientId, player: &str, location: &[&str]) {
        match self.find_location_to_send(player, &location.join(" ")) {
            Ok((team, slot, location)) => self.register_location_checks(team, slot, &[location]),
            Err(Some(text)) => self.admin_output(id, text),
            Err(None) => self.admin_invalid(id, "send_location"),
        }
    }

    fn find_location_to_send(
        &self,
        player: &str,
        full_name: &str,
    ) -> Found<(TeamId, SlotId, LocationId)> {
        let (team, slot) = self.find_player(player)?;
        if py_isnumeric(full_name) {
            let location = numeric_id(full_name).ok_or(None)?;
            return Ok((team, slot, location));
        }
        let game = self.slot_game(slot);
        let Some(package) = self.room.data().datapackage.get(&game) else {
            return Err(Some(
                "Can't look up location for unknown game. Send by ID instead.".to_string(),
            ));
        };
        let names: Vec<&str> = package
            .location_name_to_id
            .keys()
            .map(String::as_str)
            .collect();
        let (name, usable, response) = intended_text(full_name, &names);
        let location = package.location_name_to_id.get(&name).copied();
        match location.filter(|_| usable) {
            Some(location) => Ok((team, slot, location)),
            None => Err(Some(response)),
        }
    }

    /// `/hint` et `/hint_location` : indice donné à l'équipe du joueur, sans coût.
    fn srv_hint(&mut self, id: ClientId, player: &str, rest: &[&str], for_location: bool) {
        let command = if for_location {
            "hint_location"
        } else {
            "hint"
        };
        let (team, hints) = match self.find_hints(player, &rest.join(" "), for_location) {
            Ok(found) => found,
            Err(Some(text)) => {
                self.admin_output(id, text);
                return;
            }
            Err(None) => {
                self.admin_invalid(id, command);
                return;
            }
        };
        if hints.is_empty() {
            self.admin_output(id, "No hints found.");
            return;
        }
        let effects = self
            .game
            .notify_hints(self.room.data(), team, &hints, false, false, None);
        self.apply_hint_effects(team, effects);
    }

    /// Les indices demandés : par numéro, par nom, ou par groupe de noms.
    fn find_hints(
        &self,
        player: &str,
        full_name: &str,
        for_location: bool,
    ) -> Found<(TeamId, Vec<Hint>)> {
        let (team, slot) = self.find_player(player)?;
        let data = self.room.data();
        let status = Some(HintStatus::UNSPECIFIED);
        if py_isnumeric(full_name) {
            let number = numeric_id(full_name).ok_or(None)?;
            let hints = if for_location {
                self.game
                    .collect_hint_location_id(data, team, slot, number, status)
            } else {
                self.game.collect_hints(data, team, slot, number, None)
            };
            return Ok((team, hints));
        }
        let game = self.slot_game(slot);
        let Some(package) = data.datapackage.get(&game) else {
            let what = if for_location { "location" } else { "item" };
            return Err(Some(format!(
                "Can't look up {what} for unknown game. Hint for ID instead."
            )));
        };
        let (names, groups) = if for_location {
            (&package.location_name_to_id, &package.location_name_groups)
        } else {
            (&package.item_name_to_id, &package.item_name_groups)
        };
        // Candidats : les noms puis les groupes (l'officiel en fait un ensemble).
        let mut candidates: Vec<&str> = names.keys().map(String::as_str).collect();
        for group in groups.keys() {
            if !names.contains_key(group) {
                candidates.push(group);
            }
        }
        let (name, usable, response) = intended_text(full_name, &candidates);
        if !usable {
            return Err(Some(response));
        }
        let mut hints = Vec::new();
        if let Some(members) = groups.get(&name) {
            // Seuls les membres qui ont un numéro comptent.
            for member in members {
                let Some(number) = names.get(member) else {
                    continue;
                };
                if for_location {
                    hints.extend(
                        self.game
                            .collect_hint_location_id(data, team, slot, *number, status),
                    );
                } else {
                    hints.extend(self.game.collect_hints(data, team, slot, *number, None));
                }
            }
        } else if let Some(number) = names.get(&name) {
            if for_location {
                hints = self
                    .game
                    .collect_hint_location_id(data, team, slot, *number, status);
            } else {
                hints = self.game.collect_hints(data, team, slot, *number, None);
            }
        }
        Ok((team, hints))
    }
}
