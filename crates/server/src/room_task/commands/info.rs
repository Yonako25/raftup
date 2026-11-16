//! Commandes d'information : `!help`, `!license`, `!options`, `!players`,
//! `!status`, `!remaining`, `!missing`, `!checked`.
//! Les textes sont ceux de `MultiServer.py` (Archipelago 0.6.8).

use super::*;

/// Texte de `!help`. Le serveur officiel le fabrique avec `inspect` à partir de
/// la signature et de la docstring de chaque commande ; ici il est écrit en dur.
/// À vérifier par capture (écrit à la main d'après le code, sans l'avoir lancé).
const HELP_TEXT: &str = concat!(
    "!help \n    Returns the help listing\n",
    "!license \n    Returns the licensing information\n",
    "!options \n    List all current options. Warning: lists password.\n",
    "!admin [command] \n    Allow remote administration of the multiworld server\n",
    "    Usage: \"!admin login <password>\" in order to log in to the remote interface.\n",
    "    Once logged in, you can then use \"!admin <command>\" to issue commands.\n",
    "    If you need further help once logged in.  use \"!admin /help\" \n",
    "!players \n    Get information about connected and missing players.\n",
    "!status [tag] \n    Get information about your team.\n",
    "    Optionally mention a Tag name and get information on who has that Tag.\n",
    "    For example: DeathLink or EnergyLink.\n",
    "!release \n    Sends remaining items in your world to their recipients.\n",
    "!collect \n    Send your remaining items to yourself\n",
    "!countdown seconds=10 \n    Start a countdown in seconds\n",
    "!remaining \n    List remaining items in your game, but not their location or recipient\n",
    "!missing [filter_text] \n    List all missing location checks from the server's perspective.\n",
    "    Can be given text, which will be used as filter.\n",
    "!checked [filter_text] \n    List all done location checks from the server's perspective.\n",
    "    Can be given text, which will be used as filter.\n",
    "!alias [alias_name] \n    Set your alias to the passed name.\n",
    "!getitem item_name \n    Cheat in an item, if it is enabled on this server\n",
    "!hint [item_name] \n    Use !hint {item_name},\n",
    "    for example !hint Lamp to get a spoiler peek for that item.\n",
    "    If hint costs are on, this will only give you one new result,\n",
    "    you can rerun the command to get more in that case.\n",
    "!hint_location [location] \n    Use !hint_location {location_name},\n",
    "    for example !hint_location atomic-bomb to get a spoiler peek for that location.\n",
);

/// Texte de `!license`. DÉCISION : le serveur officiel affiche le fichier LICENSE
/// d'Archipelago ; nous affichons la notice de Raftup (écart déclaré).
const LICENSE_TEXT: &str =
    "Raftup is dual-licensed under the MIT License or the Apache License 2.0, \
at your option. Copyright (c) 2026 The Raftup contributors. \
See the LICENSE-MIT and LICENSE-APACHE files distributed with Raftup for the full texts.";

impl RoomTask {
    pub(super) fn cmd_help(&self, id: ClientId) {
        self.output(id, HELP_TEXT);
    }

    pub(super) fn cmd_license(&self, id: ClientId) {
        self.output(id, LICENSE_TEXT);
    }

    /// `!options` : une trame par ligne, comme le serveur officiel.
    /// Les mots de passe sont masqués (écart déclaré pour `password`).
    pub(super) fn cmd_options(&self, id: ClientId) {
        let options = &self.room.data().server_options;
        let password = match self.room.password() {
            Some(_) => MASK.to_string(),
            None => "None".to_string(),
        };
        let python_bool = |value: bool| if value { "True" } else { "False" };
        let lines = [
            ("hint_cost", options.hint_cost.to_string()),
            (
                "location_check_points",
                options.location_check_points.to_string(),
            ),
            // Toujours masqué, comme le serveur officiel pour un client.
            ("server_password", MASK.to_string()),
            ("password", password),
            ("release_mode", options.release_mode.clone()),
            ("remaining_mode", options.remaining_mode.clone()),
            ("collect_mode", options.collect_mode.clone()),
            ("countdown_mode", options.countdown_mode.clone()),
            (
                "item_cheat",
                python_bool(!options.disable_item_cheat).to_string(),
            ),
            ("compatibility", options.compatibility.to_string()),
        ];
        self.output(id, "Current options:");
        for (name, value) in lines {
            self.output(id, format!("Option {name} is set to {value}"));
        }
    }

    /// `!players` (`get_players_string`). Avec moins de 10 noms, tout le monde
    /// voit la réponse ; sinon seulement le demandeur.
    pub(super) fn cmd_players(&self, id: ClientId) {
        // Tous les (équipe, slot) de la partie, triés.
        let names: BTreeSet<(TeamId, SlotId)> =
            self.room.data().connect_names.values().copied().collect();
        let connected: BTreeSet<(TeamId, SlotId)> = self
            .clients
            .values()
            .filter_map(|c| c.player.as_ref())
            .map(|p| (p.team, p.slot))
            .collect();
        let mut text = String::new();
        let mut total = 0;
        let mut current_team = None;
        for (team, slot) in &names {
            let is_player = self
                .room
                .data()
                .slots
                .get(slot)
                .is_some_and(|s| s.slot_type == SlotType::PLAYER);
            if !is_player {
                continue;
            }
            total += 1;
            let name = self.slot_name(*slot);
            if current_team != Some(*team) {
                text.push_str(&format!(":: Team #{}: ", team + 1));
                current_team = Some(*team);
            }
            if connected.contains(&(*team, *slot)) {
                text.push_str(&format!("{name} "));
            } else {
                text.push_str(&format!("({name}) "));
            }
        }
        // `text[:-1]` : on retire le dernier caractère (l'espace final).
        text.pop();
        let text = format!("{} players of {total} connected {text}", connected.len());
        if names.len() < 10 {
            self.broadcast_text_all(command_result(text));
        } else {
            self.output(id, text);
        }
    }

    /// `!status [tag]` (`get_status_string`) : l'état de l'équipe du demandeur.
    pub(super) fn cmd_status(&self, id: ClientId, tag: &str) {
        let Some(player) = self.player(id) else {
            return;
        };
        let team = player.team;
        let data = self.room.data();
        let mut text = format!("Player Status on team {team}:");
        for (slot, locations) in &data.locations {
            let mut connected = 0;
            let mut tagged = 0;
            for client in self.clients.values().filter_map(|c| c.player.as_ref()) {
                if client.team == team && client.slot == *slot {
                    connected += 1;
                    if client.tags.iter().any(|t| t == tag) {
                        tagged += 1;
                    }
                }
            }
            let plural = if connected == 1 { "" } else { "s" };
            let tag_text = if connected > 0 && !tag.is_empty() {
                format!(" {tagged} of which are tagged {tag}")
            } else {
                String::new()
            };
            let status_text = match self.game.status(team, *slot) {
                Some(ClientStatus::GOAL) => " and has finished.",
                Some(ClientStatus::READY) => " and is ready.",
                _ => ".",
            };
            text.push_str(&format!(
                "\n{} has {connected} connection{plural}{tag_text}{status_text} ({}/{})",
                self.slot_name(*slot),
                self.game.checked_count(team, *slot),
                locations.len()
            ));
        }
        self.output(id, text);
    }

    /// `!remaining` : les objets pas encore trouvés dans le monde du demandeur.
    pub(super) fn cmd_remaining(&self, id: ClientId) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        let mode = self.room.data().server_options.remaining_mode.as_str();
        match mode {
            "enabled" => {}
            "disabled" => {
                self.output(id, "Sorry, !remaining has been disabled on this server.");
                return;
            }
            // `goal` (et tout autre mode) : il faut avoir fini la partie.
            _ => {
                if self.game.status(player.team, player.slot) != Some(ClientStatus::GOAL) {
                    self.output(
                        id,
                        "Sorry, !remaining requires you to have beaten the game on this server",
                    );
                    return;
                }
            }
        }
        match self.remaining_names(player.team, player.slot) {
            Some(names) if names.is_empty() => self.output(id, "No remaining items found."),
            Some(names) => self.output(id, format!("Remaining items: {}", names.join(", "))),
            None => self.invalid_arguments(id, "remaining"),
        }
    }

    /// Noms des objets des lieux pas encore visités, triés par (destinataire, objet),
    /// avec les noms du jeu du destinataire. `None` si Python planterait.
    fn remaining_names(&self, team: TeamId, slot: SlotId) -> Option<Vec<String>> {
        let data = self.room.data();
        let locations = data.locations.get(&slot)?;
        let checked: BTreeSet<LocationId> = self.game.checked(team, slot).into_iter().collect();
        let mut rest: Vec<(SlotId, i64)> = locations
            .iter()
            .filter(|(location, _)| !checked.contains(location))
            .map(|(_, placed)| (placed.player, placed.item))
            .collect();
        rest.sort_unstable();
        rest.iter()
            .map(|(receiver, item)| {
                let game = data.slots.get(receiver)?.game.as_str();
                let package = data.datapackage.get(game)?;
                // Le dictionnaire Python id -> nom garde le dernier nom d'un même id.
                let name = package
                    .item_name_to_id
                    .iter()
                    .rev()
                    .find(|(_, id)| *id == item)
                    .map(|(name, _)| name.clone())?;
                Some(name)
            })
            .collect()
    }

    pub(super) fn cmd_missing(&self, id: ClientId, filter: &str) {
        self.list_checks(id, filter, false);
    }

    pub(super) fn cmd_checked(&self, id: ClientId, filter: &str) {
        self.list_checks(id, filter, true);
    }

    /// `!missing` et `!checked` : même code, seuls les textes changent.
    fn list_checks(&self, id: ClientId, filter: &str, done: bool) {
        let command = if done { "checked" } else { "missing" };
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        let data = self.room.data();
        let Some(slot_locations) = data.locations.get(&player.slot) else {
            return self.invalid_arguments(id, command);
        };
        let checked: BTreeSet<LocationId> = self
            .game
            .checked(player.team, player.slot)
            .into_iter()
            .collect();
        let locations: Vec<LocationId> = slot_locations
            .keys()
            .copied()
            .filter(|l| checked.contains(l) == done)
            .collect();
        if locations.is_empty() {
            let text = if done {
                "No done location checks found."
            } else {
                "No missing location checks found."
            };
            return self.output(id, text);
        }
        let game = self.slot_game(player.slot);
        let Some(package) = data.datapackage.get(&game) else {
            return self.invalid_arguments(id, command);
        };
        // Python : dictionnaire id -> nom, le dernier nom d'un même id l'emporte.
        let lookup: BTreeMap<LocationId, &str> = package
            .location_name_to_id
            .iter()
            .map(|(name, id)| (*id, name.as_str()))
            .collect();
        let mut names = Vec::new();
        for location in &locations {
            match lookup.get(location) {
                Some(name) => names.push(*name),
                None => return self.invalid_arguments(id, command),
            }
        }
        if !filter.is_empty() {
            // Un nom de groupe de lieux exact, sinon une sous-chaîne (casse comprise).
            match package.location_name_groups.get(filter) {
                Some(group) => names.retain(|name| group.iter().any(|g| g.as_str() == *name)),
                None => names.retain(|name| name.contains(filter)),
            }
        }
        let label = if done { "Checked" } else { "Missing" };
        let mut texts: Vec<String> = names.iter().map(|n| format!("{label}: {n}")).collect();
        let kind = if done { "done" } else { "missing" };
        if filter.is_empty() {
            texts.push(format!("Found {} {kind} location checks", locations.len()));
        } else {
            texts.push(format!(
                "Found {} {kind} location checks, displaying {} of them.",
                locations.len(),
                names.len()
            ));
        }
        self.output_multiple(id, texts);
    }
}
