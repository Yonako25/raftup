//! Suivi de la partie depuis l'extérieur (ADR 0010) : photo de l'état,
//! fil d'événements et message de l'hôte.

use super::*;

impl RoomTask {
    /// Message de l'hôte à tous, comme la console du serveur officiel :
    /// `[Server]: texte`, de type `ServerChat`.
    pub(super) fn announce(&mut self, text: String) {
        info!("message de l'hôte : {text}");
        let mut message = text_message(format!("[Server]: {text}"));
        message.kind = Some("ServerChat".to_string());
        message.message = Some(text.clone());
        self.broadcast_text_all(message);
        self.emit(GameEvent::ServerChat { time: now(), text });
    }

    /// Publie un événement ; sans abonné, il est simplement perdu.
    pub(super) fn emit(&self, event: GameEvent) {
        let _ = self.monitor.send(event);
    }

    /// Événement « objet envoyé », avec les noms lisibles.
    pub(super) fn item_event(&self, sent: &Sent) -> GameEvent {
        let finder = sent.item.player;
        GameEvent::ItemSent {
            time: now(),
            finder: self.slot_name(finder),
            receiver: self.slot_name(sent.receiver),
            item: self
                .names
                .item(&self.slot_game(sent.receiver), sent.item.item),
            location: self
                .names
                .location(&self.slot_game(finder), sent.item.location),
            flags: u32::from(sent.item.flags.0),
        }
    }

    /// Photo de l'état de chaque slot.
    pub(super) fn status(&self) -> ServerStatus {
        let data = self.room.data();
        let mut slots = Vec::new();
        for (slot, info) in &data.slots {
            let team = 0;
            let players: Vec<&Player> = self
                .clients
                .values()
                .filter_map(|c| c.player.as_ref())
                .filter(|p| (p.team, p.slot) == (team, *slot))
                .collect();
            let tags = players
                .iter()
                .find(|p| !p.no_locations)
                .map(|p| p.tags.clone())
                .unwrap_or_default();
            slots.push(SlotStatus {
                team,
                slot: *slot,
                name: info.name.clone(),
                game: info.game.clone(),
                is_group: info.slot_type != SlotType::PLAYER,
                connections: players.len(),
                tags,
                checked: self.game.checked_count(team, *slot),
                total: data.locations.get(slot).map_or(0, |l| l.len()),
                status: u32::from(self.game.status(team, *slot).map_or(0, |s| s.0)),
                last_activity: self.activity.get(&(team, *slot)).copied(),
            });
        }
        ServerStatus {
            slots,
            connections: self.clients.len(),
        }
    }
}
