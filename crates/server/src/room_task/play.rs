//! Déroulement de la partie : lieux visités, statut du client, objectif atteint
//! (`collect` et `release` automatiques) et envoi des nouveaux objets.

use super::*;

impl RoomTask {
    pub(super) fn on_location_checks(&mut self, id: ClientId, checks: &LocationChecks) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        if player.no_locations {
            let text = "Trackers can't register new Location Checks";
            self.send(id, vec![invalid_packet("cmd", "LocationChecks", text)]);
            return;
        }
        let before = self.game.checked_count(player.team, player.slot);
        self.register_location_checks(player.team, player.slot, &checks.locations);
        // Un nouveau lieu visité par le joueur lui-même : il joue (ADR 0031).
        if self.game.checked_count(player.team, player.slot) > before {
            self.on_player_played(player.team, player.slot);
        }
    }

    /// Enregistre des lieux visités et prévient tout le monde
    /// (`register_location_checks` du serveur officiel).
    pub(super) fn register_location_checks(
        &mut self,
        team: TeamId,
        slot: SlotId,
        locations: &[LocationId],
    ) {
        let (new_locations, sent) =
            self.game
                .register_checks(self.room.data(), team, slot, locations);
        if new_locations.is_empty() {
            return;
        }
        info!(team, slot, count = new_locations.len(), "lieux visités");
        for s in &sent {
            let event = self.item_event(s);
            self.emit(event);
        }

        // 1. Messages « X a envoyé Y à Z » à toute l'équipe, par paquets de 140.
        for chunk in sent.chunks(ITEM_SEND_CHUNK) {
            let messages: Vec<ServerPacket> = chunk
                .iter()
                .map(|s| ServerPacket::PrintJson(item_send_message(s)))
                .collect();
            self.broadcast_team_text(team, messages);
        }
        // 2. Les nouveaux objets à chaque client qui doit les recevoir.
        self.send_new_items();
        // 3. Les lieux visités et les points d'indice aux clients de ce slot.
        let update = RoomUpdate {
            hint_points: Some(self.hint_points(team, slot)),
            checked_locations: Some(new_locations),
            ..RoomUpdate::default()
        };
        self.broadcast_slot(team, slot, vec![ServerPacket::RoomUpdate(update)]);
        // 4. Les indices dont le lieu vient d'être visité passent à « trouvé ».
        let changed = self.game.recheck_hints(self.room.data(), team, slot);
        for (hint_team, hint_slot) in changed {
            let value = hints_value(&self.game.hints(hint_team, hint_slot));
            self.notify_read_key(format!("_read_hints_{hint_team}_{hint_slot}"), value);
        }
    }

    /// Envoie aux clients d'un slot la liste complète de ses lieux visités.
    pub(super) fn update_checked_locations(&mut self, team: TeamId, slot: SlotId) {
        let update = RoomUpdate {
            checked_locations: Some(self.game.checked(team, slot)),
            ..RoomUpdate::default()
        };
        self.broadcast_slot(team, slot, vec![ServerPacket::RoomUpdate(update)]);
    }

    pub(super) fn on_status_update(&mut self, id: ClientId, status: ClientStatus) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        if player.no_locations && status == ClientStatus::GOAL {
            let text = "Trackers can't register Goal Complete";
            self.send(id, vec![invalid_packet("cmd", "StatusUpdate", text)]);
            return;
        }
        self.update_client_status(player.team, player.slot, status);
    }

    /// Change le statut d'un slot (`update_client_status` du serveur officiel).
    /// L'objectif atteint est définitif : ensuite, plus rien ne change.
    pub(super) fn update_client_status(
        &mut self,
        team: TeamId,
        slot: SlotId,
        status: ClientStatus,
    ) {
        if self.game.status(team, slot) == Some(ClientStatus::GOAL) {
            return;
        }
        if status == ClientStatus::GOAL {
            self.on_goal_achieved(team, slot);
            // Tous les autres slots de l'équipe (groupes compris) ont fini ?
            // Un slot jamais connecté n'a pas de statut : il n'a donc pas fini.
            let slots: Vec<SlotId> = self.room.data().slots.keys().copied().collect();
            let all_done = slots
                .iter()
                .filter(|other| **other != slot)
                .all(|other| self.game.status(team, *other) == Some(ClientStatus::GOAL));
            if all_done {
                let text = format!(
                    "Team #{} has completed all of their games! Congratulations!",
                    team + 1
                );
                self.broadcast_text_all(text_message(text));
            }
        }
        self.game.set_status(team, slot, status);
        let key = format!("_read_client_status_{team}_{slot}");
        self.notify_read_key(key, Value::from(status.0));
    }

    /// Annonce l'objectif atteint, puis `collect` et `release` automatiques
    /// si les options de la salle le prévoient (`auto` ou `auto-enabled`).
    pub(super) fn on_goal_achieved(&mut self, team: TeamId, slot: SlotId) {
        let text = format!(
            "{} (Team #{}) has completed their goal.",
            self.aliased_name(team, slot),
            team + 1
        );
        info!(team, slot, "objectif atteint");
        self.feed(team, slot, multidata::raftup::FeedKind::Goal);
        self.emit(GameEvent::Goal {
            time: now(),
            slot,
            name: self.slot_name(slot),
        });
        let mut message = text_message(text);
        message.kind = Some("Goal".to_string());
        message.team = Some(team);
        message.slot = Some(slot);
        self.broadcast_text_all(message);

        let options = &self.room.data().server_options;
        let auto_collect = options.collect_mode.contains("auto");
        let auto_release = options.release_mode.contains("auto");
        if auto_collect {
            self.collect_player(team, slot, false);
        }
        if auto_release {
            self.release_player(team, slot);
        }
    }

    /// Envoie aux autres tous les objets encore cachés dans le monde de ce slot.
    pub(super) fn release_player(&mut self, team: TeamId, slot: SlotId) {
        self.feed(team, slot, multidata::raftup::FeedKind::Release);
        self.release_world(team, slot);
    }

    /// Le travail de `release_player`, sans entrée au fil d'activité (la
    /// release par vote écrit la sienne, ADR 0031).
    pub(super) fn release_world(&mut self, team: TeamId, slot: SlotId) {
        let text = format!(
            "{} (Team #{}) has released all remaining items from their world.",
            self.slot_name(slot),
            team + 1
        );
        self.broadcast_text_all(slot_message(text, "Release", team, slot));
        let all_locations: Vec<LocationId> = match self.room.data().locations.get(&slot) {
            Some(locations) => locations.keys().copied().collect(),
            None => Vec::new(),
        };
        self.register_location_checks(team, slot, &all_locations);
        self.update_checked_locations(team, slot);
    }

    /// Récupère tous les objets de ce slot encore cachés dans les autres mondes.
    /// Quand tous les membres d'un groupe l'ont fait, le groupe aussi.
    pub(super) fn collect_player(&mut self, team: TeamId, slot: SlotId, is_group: bool) {
        let text = format!(
            "{} (Team #{}) has collected their items from other worlds.",
            self.slot_name(slot),
            team + 1
        );
        self.broadcast_text_all(slot_message(text, "Collect", team, slot));
        for (source, locations) in locations_for_player(self.room.data(), slot) {
            self.register_location_checks(team, source, &locations);
            self.update_checked_locations(team, source);
        }
        if !is_group {
            let groups: Vec<SlotId> = self
                .room
                .data()
                .slots
                .iter()
                .filter(|(_, info)| info.slot_type == SlotType::GROUP)
                .filter(|(_, info)| info.group_members.contains(&slot))
                .map(|(group, _)| *group)
                .collect();
            for group in groups {
                if self
                    .game
                    .mark_group_collected(self.room.data(), group, slot)
                {
                    self.collect_player(team, group, true);
                }
            }
        }
    }

    /// Envoie à chaque joueur les objets qu'il n'a pas encore reçus (`send_new_items`).
    pub(super) fn send_new_items(&mut self) {
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        for id in ids {
            let Some(mut player) = self.player(id).cloned() else {
                continue;
            };
            if player.items.no_items {
                continue;
            }
            let all_items = self.all_items(&player);
            if all_items.len() > player.send_index {
                let index = player.send_index;
                player.send_index = all_items.len();
                self.set_player(id, player);
                let packet = ReceivedItems {
                    index: u32::try_from(index).unwrap_or(u32::MAX),
                    items: all_items[index..].to_vec(),
                };
                self.send(id, vec![ServerPacket::ReceivedItems(packet)]);
            }
        }
    }
}
