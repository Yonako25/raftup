//! Commandes du chat : un message qui commence par `!` (`ClientMessageProcessor`
//! et `CommandProcessor.__call__` du serveur officiel, `MultiServer.py`).
//!
//! Ce module fait le cadre : découpage de la ligne, choix de la commande,
//! réponses `CommandResult`. Les commandes d'information sont dans `info`.

use super::*;
use crate::command_line::{raw_argument, split_command, truncate_chars};

mod actions;
mod hint;
mod info;

/// Commandes connues, dans l'ordre du serveur officiel (celui de `commands`,
/// affiché par « Known commands »).
const COMMANDS: [&str; 16] = [
    "help",
    "license",
    "options",
    "admin",
    "players",
    "status",
    "release",
    "collect",
    "countdown",
    "remaining",
    "missing",
    "checked",
    "alias",
    "getitem",
    "hint",
    "hint_location",
];

/// Commandes qui reçoivent le texte brut après le premier mot (`@mark_raw`).
fn is_raw(command: &str) -> bool {
    matches!(
        command,
        "admin" | "missing" | "checked" | "alias" | "getitem" | "hint" | "hint_location"
    )
}

/// Étoiles qui cachent un mot de passe. Le serveur officiel en tire entre 4 et
/// 16 au hasard ; on en met toujours 8 pour que la réponse soit reproductible.
pub(super) const MASK: &str = "********";

impl RoomTask {
    /// Réponse à une commande, visible du seul demandeur (`output`).
    pub(super) fn output(&self, id: ClientId, text: impl Into<String>) {
        self.notify(id, command_result(text.into()));
    }

    /// Plusieurs réponses envoyées dans une seule trame (`output_multiple`).
    pub(super) fn output_multiple(&self, id: ClientId, texts: Vec<String>) {
        if self.player(id).is_some_and(|p| !p.no_text) {
            let packets = texts
                .into_iter()
                .map(|text| ServerPacket::PrintJson(command_result(text)))
                .collect();
            self.send(id, packets);
        }
    }

    /// Le serveur officiel plante sur des arguments qui ne conviennent pas et
    /// renvoie la trace d'erreur Python ; nous répondons un texte court (écart connu).
    pub(super) fn invalid_arguments(&self, id: ClientId, command: &str) {
        self.output(id, format!("Invalid arguments for !{command}."));
    }

    /// Exécute la ligne d'un `Say` si c'est une commande (`CommandProcessor.__call__`).
    /// Appelé après la diffusion du chat, comme le serveur officiel.
    pub(super) fn on_command_line(&mut self, id: ClientId, raw: &str) {
        if raw.is_empty() {
            return;
        }
        let words = split_command(raw);
        // Ligne faite d'espaces : le serveur officiel plante (IndexError) et
        // renvoie une trace ; nous ne répondons rien (écart connu).
        let Some(first) = words.first() else {
            return;
        };
        // Un message sans `!` en tête est du chat simple, sans réponse.
        let Some(name) = first.strip_prefix('!') else {
            return;
        };
        // `.lower()` de Python.
        let command = name.to_lowercase();
        if !COMMANDS.contains(&command.as_str()) {
            let text = format!(
                "Could not find command {name}. Known commands: {}",
                COMMANDS.join(", ")
            );
            self.output(id, text);
            return;
        }
        if is_raw(&command) {
            let argument = raw_argument(raw);
            self.run_raw_command(id, &command, argument);
        } else {
            let args: Vec<&str> = words[1..].iter().map(String::as_str).collect();
            self.run_command(id, &command, &args);
        }
    }

    /// Commandes à arguments découpés mot à mot.
    fn run_command(&mut self, id: ClientId, command: &str, args: &[&str]) {
        // Python refuse un nombre d'arguments que la méthode n'accepte pas.
        let max_args = match command {
            "status" | "countdown" => 1,
            _ => 0,
        };
        if args.len() > max_args {
            self.invalid_arguments(id, command);
            return;
        }
        match command {
            "help" => self.cmd_help(id),
            "license" => self.cmd_license(id),
            "options" => self.cmd_options(id),
            "players" => self.cmd_players(id),
            "status" => self.cmd_status(id, args.first().copied().unwrap_or("")),
            "remaining" => self.cmd_remaining(id),
            "release" => self.cmd_release(id),
            "collect" => self.cmd_collect(id),
            "countdown" => self.cmd_countdown(id, args.first().copied().unwrap_or("10")),
            _ => self.todo_command(id, command),
        }
    }

    /// Commandes qui lisent le texte brut (`@mark_raw`).
    fn run_raw_command(&mut self, id: ClientId, command: &str, argument: Option<String>) {
        // `_cmd_getitem(self, item_name)` n'a pas de valeur par défaut :
        // sans argument, Python lève une erreur.
        if command == "getitem" && argument.is_none() {
            self.invalid_arguments(id, command);
            return;
        }
        let text = argument.unwrap_or_default();
        match command {
            "admin" => self.cmd_admin(id, &text),
            "missing" => self.cmd_missing(id, &text),
            "checked" => self.cmd_checked(id, &text),
            "alias" => self.cmd_alias(id, &text),
            "getitem" => self.cmd_getitem(id, &text),
            "hint" => self.cmd_hint(id, &text, false),
            "hint_location" => self.cmd_hint(id, &text, true),
            _ => self.todo_command(id, command),
        }
    }

    /// Commande de `COMMANDS` sans branche dans `run_command` /
    /// `run_raw_command`. Toutes sont branchées : n'arrive que si on ajoute une
    /// commande à la liste en oubliant son code.
    fn todo_command(&mut self, id: ClientId, command: &str) {
        debug!(id, command, "commande connue mais sans code");
    }

    /// `!admin` : diffuse la ligne avec les mots de passe masqués, puis gère
    /// `login` / `logout`. Les commandes serveur elles-mêmes arrivent en J3.
    fn cmd_admin(&mut self, id: ClientId, command: &str) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        let mut line = format!("!admin {command}");
        // Masquage décidé sur les mots découpés (casse ignorée), pas sur le
        // texte brut : `!admin "login" secret` doit aussi être masqué.
        let words: Vec<String> = split_command(command)
            .iter()
            .map(|w| w.to_lowercase())
            .collect();
        let first = words.first().map(String::as_str);
        let second = words.get(1).map(String::as_str);
        if first == Some("login") {
            line = format!("!admin login {MASK}");
        } else if first == Some("/option") && second == Some("server_password") {
            line = format!("!admin /option server_password {MASK}");
        }
        let name = self.slot_name(player.slot);
        self.emit(GameEvent::Chat {
            time: now(),
            slot: player.slot,
            name,
            text: line.clone(),
        });
        let shown = self.aliased_name(player.team, player.slot);
        let mut message =
            slot_message(format!("{shown}: {line}"), "Chat", player.team, player.slot);
        message.message = Some(line);
        self.broadcast_text_all(message);

        let password = self
            .room
            .data()
            .server_options
            .server_password
            .clone()
            .unwrap_or_default();
        if password.is_empty() {
            self.output(id, "Sorry, Remote administration is disabled");
            return;
        }
        // Un seul administrateur à la fois, comme le serveur officiel
        // (`commandprocessor.client`) : se connecter remplace le précédent.
        let is_admin = self.admin_client == Some(id);

        if command.is_empty() {
            if is_admin {
                self.output(
                    id,
                    "Usage: !admin [Server command].\nUse !admin /help for help.\n\
                     Use !admin logout to log out of the current session.",
                );
            } else {
                self.output(id, "Usage: !admin login [password]");
            }
            return;
        }
        // Comparaison exacte, casse comprise, comme en Python.
        if command.starts_with("login ") {
            if command == format!("login {password}") {
                self.output(
                    id,
                    "Login successful. You can now issue server side commands.",
                );
                self.admin_client = Some(id);
            } else {
                self.output(id, "Password incorrect.");
            }
            return;
        }
        if !is_admin {
            self.output(id, "You must first login using !admin login [password]");
            return;
        }
        if command == "logout" {
            self.output(
                id,
                "Logout successful. You can no longer issue server side commands.",
            );
            self.admin_client = None;
            return;
        }
        // Commandes serveur : reportées à J3 (ADR 0013). Réponse du genre
        // `AdminCommandResult`, comme celles de la console officielle.
        let mut message = text_message("Server commands are not available yet.".to_string());
        message.kind = Some("AdminCommandResult".to_string());
        self.notify(id, message);
    }
}

/// Vrai si la ligne de chat est une commande `!admin`, reconnue comme dans
/// `on_command_line` : premier mot découpé, casse ignorée.
pub(super) fn is_admin_command(raw: &str) -> bool {
    split_command(raw)
        .first()
        .is_some_and(|w| w.to_lowercase() == "!admin")
}

/// `PrintJSON` de genre `CommandResult`.
fn command_result(text: String) -> PrintJson {
    let mut message = text_message(text);
    message.kind = Some("CommandResult".to_string());
    message
}
