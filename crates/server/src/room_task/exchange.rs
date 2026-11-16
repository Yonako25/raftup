//! Échanges entre clients : `LocationScouts`, `Say` (chat), `Bounce` et paquets
//! aux champs incorrects.

use super::*;

/// Façon de combiner les critères d'un `Bounce` (champ `operator`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BounceOperator {
    /// Par défaut : la bonne équipe, et au moins un des autres critères.
    Legacy,
    /// Au moins un critère, équipe comprise.
    Or,
    /// Tous les critères.
    And,
}

/// Destinataires d'un `Bounce` (`BounceTarget` du serveur officiel).
/// `None` = critère absent.
pub(super) struct BounceTarget {
    pub(super) teams: Option<Vec<TeamId>>,
    pub(super) games: Option<Vec<String>>,
    pub(super) tags: Option<Vec<String>>,
    pub(super) slots: Option<Vec<SlotId>>,
}

impl BounceTarget {
    pub(super) fn matches(&self, operator: BounceOperator, player: &Player, game: &str) -> bool {
        let team = self.teams.as_ref().map(|t| t.contains(&player.team));
        // Les critères autres que l'équipe, seulement ceux qui sont présents.
        let others = [
            self.games.as_ref().map(|g| g.iter().any(|x| x == game)),
            self.tags
                .as_ref()
                .map(|t| t.iter().any(|x| player.tags.contains(x))),
            self.slots.as_ref().map(|s| s.contains(&player.slot)),
        ];
        let others: Vec<bool> = others.into_iter().flatten().collect();
        match operator {
            // Sans autre critère, `any` est faux : personne ne reçoit le message.
            BounceOperator::Legacy => team.unwrap_or(true) && others.iter().any(|m| *m),
            BounceOperator::Or => team.into_iter().chain(others).any(|m| m),
            BounceOperator::And => team.into_iter().chain(others).all(|m| m),
        }
    }
}

/// Premier champ de liste mal formé d'un `Bounce`, dans l'ordre où le serveur
/// officiel les vérifie : une liste de nombres (`teams`, `slots`) ou de textes.
pub(super) fn bad_bounce_field(raw: &Value) -> Option<&'static str> {
    let fields = [
        ("teams", true),
        ("games", false),
        ("tags", false),
        ("slots", true),
    ];
    fields.into_iter().find_map(|(name, numbers)| {
        let value = raw.get(name)?;
        let ok = value.as_array().is_some_and(|items| {
            items.iter().all(|item| {
                if numbers {
                    item.is_i64() || item.is_u64()
                } else {
                    item.is_string()
                }
            })
        });
        (!ok).then_some(name)
    })
}

impl RoomTask {
    /// Paquet connu aux champs incorrects : la réponse du serveur officiel,
    /// quand il en a une. Avant la connexion, seul `Connect` répond.
    pub(super) fn on_bad_arguments(&mut self, id: ClientId, cmd: &str, raw: Option<&Value>) {
        let text = match cmd {
            "Connect" => "Connect".to_string(),
            _ if self.player(id).is_none() => return,
            // Le serveur officiel refuse un outil de suivi avant de lire les lieux.
            "LocationChecks" if self.player(id).is_some_and(|p| p.no_locations) => {
                let text = "Trackers can't register new Location Checks";
                self.send(id, vec![invalid_packet("cmd", cmd, text)]);
                return;
            }
            "LocationScouts" => "Locations has to be a list of integers".to_string(),
            "Say" => "Say".to_string(),
            "Get" => "Retrieve".to_string(),
            "Set" => "Set".to_string(),
            "SetNotify" => "SetNotify".to_string(),
            "Bounce" => {
                // Sans champ de liste fautif (par exemple `data` qui n'est pas
                // un objet), le serveur officiel accepterait : écart connu, rare.
                match raw.and_then(bad_bounce_field) {
                    Some(field) => format!(
                        "Bounce: \"{field}\" list provided did not have the correct format."
                    ),
                    None => "Bounce: invalid packet.".to_string(),
                }
            }
            "CreateHints" => return self.on_bad_create_hints(id, raw),
            "UpdateHint" => return self.on_bad_update_hint(id, raw),
            // Les autres seront traités avec leurs paquets.
            _ => return,
        };
        self.send(id, vec![invalid_packet("arguments", cmd, &text)]);
    }

    /// Contenu des lieux demandés, sans les visiter.
    pub(super) fn on_location_scouts(&mut self, id: ClientId, scouts: &LocationScouts) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        let create_as_hint = scouts.create_as_hint.unwrap_or(0);
        let slot_locations = self.room.data().locations.get(&player.slot);
        let mut locations = Vec::new();
        let mut hints = Vec::new();
        for location in &scouts.locations {
            let Some(placed) = slot_locations.and_then(|l| l.get(location)) else {
                // Lieu inconnu : le serveur officiel 0.6.8 plante et ferme la connexion.
                self.close(id, "LocationScouts sur un lieu inconnu");
                return;
            };
            if create_as_hint != 0 {
                // Statut « non précisé » : le défaut Python de `collect_hint_location_id`.
                hints.extend(self.game.collect_hint_location_id(
                    self.room.data(),
                    player.team,
                    player.slot,
                    *location,
                    Some(HintStatus::UNSPECIFIED),
                ));
            }
            locations.push(NetworkItem {
                item: placed.item,
                location: *location,
                player: placed.player,
                flags: placed.flags,
            });
        }
        // `notify_hints` change l'état tout de suite, comme en Python.
        let effects = self.game.notify_hints(
            self.room.data(),
            player.team,
            &hints,
            create_as_hint == 2,
            true,
            None,
        );
        // Ordre à vérifier par capture : le serveur officiel lance les envois
        // des indices en tâches (`async_start`) puis attend l'envoi de
        // `LocationInfo`, dont l'écriture part avant que les tâches ne tournent.
        // On envoie donc `LocationInfo` d'abord, puis les `SetReply` /
        // `RoomUpdate` / `PrintJSON` des indices.
        let info = LocationInfo { locations };
        self.send(id, vec![ServerPacket::LocationInfo(info)]);
        self.apply_hint_effects(player.team, effects);
    }

    /// Message de chat, envoyé à tous. Les commandes `!` arrivent en J2.
    pub(super) fn on_say(&mut self, id: ClientId, text: &str) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        if !is_printable(text) {
            self.send(id, vec![invalid_packet("arguments", "Say", "Say")]);
            return;
        }
        // NF03 : longueur en caractères Unicode, comme `len()` en Python.
        let max_length = self.limits.max_chat_length;
        if text.chars().count() > max_length {
            let notice = format!("Message not sent: longer than {max_length} characters.");
            self.notify(id, text_message(notice));
            return;
        }
        // NF04 : seuls les messages acceptés comptent.
        let limits = self.limits;
        let allowed = match self.clients.get_mut(&id) {
            Some(client) => client.chat.allow(&limits, Instant::now()),
            None => return,
        };
        if !allowed {
            let notice = format!(
                "Message not sent: at most {} messages every {} seconds.",
                limits.chat_messages,
                limits.chat_window.as_secs()
            );
            self.notify(id, text_message(notice));
            return;
        }
        // Comme le serveur officiel : même une commande est montrée à tous,
        // sauf `!admin`, qui peut contenir un mot de passe. On reconnaît la
        // commande comme `on_command_line` (premier mot, casse ignorée) : sinon
        // `!Admin login secret` partirait en clair.
        if !commands::is_admin_command(text) {
            self.emit(GameEvent::Chat {
                time: now(),
                slot: player.slot,
                name: self.slot_name(player.slot),
                text: text.to_string(),
            });
            let line = format!("{}: {text}", self.aliased_name(player.team, player.slot));
            let mut message = slot_message(line, "Chat", player.team, player.slot);
            message.message = Some(text.to_string());
            self.broadcast_text_all(message);
        }
        // Les commandes `!` s'exécutent après la diffusion, comme sur le serveur officiel.
        self.on_command_line(id, text);
    }

    /// Relaie un message libre (par exemple un DeathLink) aux clients visés.
    pub(super) fn on_bounce(&mut self, id: ClientId, bounce: Bounce) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        let operator = match bounce.extra.get("operator") {
            None => "legacy",
            Some(Value::String(text)) => text.as_str(),
            Some(_) => "",
        };
        let operator = match operator {
            "legacy" => BounceOperator::Legacy,
            "or" => BounceOperator::Or,
            "and" => BounceOperator::And,
            // Le serveur officiel 0.6.8 plante ici (variable inconnue) et ferme.
            _ => return self.close(id, "Bounce : opérateur inconnu"),
        };
        let target = BounceTarget {
            // Équipe par défaut : celle de l'expéditeur.
            teams: Some(bounce.teams.clone().unwrap_or_else(|| vec![player.team])),
            games: bounce.games.clone(),
            tags: bounce.tags.clone(),
            slots: bounce.slots.clone(),
        };

        let bounced = Bounced {
            teams: bounce.teams,
            games: bounce.games,
            slots: bounce.slots,
            tags: bounce.tags,
            data: bounce.data,
            extra: bounce.extra,
        };
        let Some(frame) = encode(&[ServerPacket::Bounced(bounced)]) else {
            return;
        };
        let data = self.room.data();
        for client in self.clients.values() {
            let Some(other) = &client.player else {
                continue;
            };
            let game = data.slots.get(&other.slot).map_or("", |s| s.game.as_str());
            if !client.closing && target.matches(operator, other, game) {
                client.push(Outgoing::Frame(frame.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinataires_bounce() {
        let player = Player {
            team: 0,
            slot: 2,
            version: SERVER_VERSION,
            tags: vec!["DeathLink".to_string()],
            items: ItemsChoice::decode(7).unwrap_or(ItemsChoice {
                no_items: false,
                remote_items: true,
                remote_start_inventory: true,
            }),
            send_index: 0,
            no_locations: false,
            no_text: false,
        };
        let target = |tags: Option<Vec<&str>>, slots: Option<Vec<SlotId>>| BounceTarget {
            teams: Some(vec![0]),
            games: None,
            tags: tags.map(|t| t.into_iter().map(String::from).collect()),
            slots,
        };
        use BounceOperator::*;
        // Legacy sans autre critère que l'équipe : personne.
        assert!(!target(None, None).matches(Legacy, &player, "Jeu"));
        assert!(target(Some(vec!["DeathLink"]), None).matches(Legacy, &player, "Jeu"));
        assert!(target(Some(vec!["Autre"]), Some(vec![2])).matches(Legacy, &player, "Jeu"));
        assert!(!target(Some(vec!["Autre"]), Some(vec![2])).matches(And, &player, "Jeu"));
        assert!(target(None, None).matches(Or, &player, "Jeu"));
        assert!(target(Some(vec!["DeathLink"]), Some(vec![2])).matches(And, &player, "Jeu"));
    }
}
