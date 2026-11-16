//! Stockage de données : `Get`, `Set`, `SetNotify` et clés spéciales `_read_…`.

use super::*;

/// Liste d'indices en JSON, avec `"class": "Hint"` comme le serveur officiel.
pub(super) fn hints_value(hints: &[Hint]) -> Value {
    serde_json::to_value(hints).unwrap_or(Value::Null)
}

impl RoomTask {
    /// Valeurs des clés demandées ; `null` pour une clé inconnue.
    pub(super) fn on_get(&mut self, id: ClientId, get: Get) {
        let mut keys = Map::new();
        for key in &get.keys {
            let value = match key.strip_prefix("_read_") {
                Some(name) => self.read_data(name),
                None => self.store.get(key).cloned(),
            };
            // Clé en double : la première place est gardée, comme en Python.
            keys.insert(key.clone(), value.unwrap_or(Value::Null));
        }
        let retrieved = Retrieved {
            keys,
            extra: get.extra,
        };
        self.send(id, vec![ServerPacket::Retrieved(retrieved)]);
    }

    /// Clés en lecture seule `_read_…`, calculées par le serveur (`read_data`).
    pub(super) fn read_data(&mut self, name: &str) -> Option<Value> {
        let data = self.room.data();
        if name == "race_mode" {
            return Some(Value::from(i64::from(data.race_mode)));
        }
        if let Some(rest) = name.strip_prefix("hints_") {
            let (team, slot) = parse_team_slot(rest)?;
            if team != 0 || !data.slots.contains_key(&slot) {
                return None;
            }
            self.game.recheck_hints(self.room.data(), team, slot);
            return Some(hints_value(self.game.hints(team, slot)));
        }
        if let Some(rest) = name.strip_prefix("client_status_") {
            let (team, slot) = parse_team_slot(rest)?;
            if team != 0 || !data.slots.contains_key(&slot) {
                return None;
            }
            let status = self
                .game
                .status(team, slot)
                .unwrap_or(ClientStatus::UNKNOWN);
            return Some(Value::from(status.0));
        }
        if let Some(rest) = name.strip_prefix("slot_data_") {
            return data.slot_data.get(&parse_exact(rest)?).cloned();
        }
        let (groups, kind) = if let Some(game) = name.strip_prefix("item_name_groups_") {
            (game, true)
        } else {
            (name.strip_prefix("location_name_groups_")?, false)
        };
        // Seulement les jeux de la partie, comme le serveur officiel.
        if !self.room.played_games().iter().any(|g| g == groups) {
            return None;
        }
        let package = data.datapackage.get(groups)?;
        let map = if kind {
            &package.item_name_groups
        } else {
            &package.location_name_groups
        };
        serde_json::to_value(map).ok()
    }

    /// Modifie une valeur du stockage, puis prévient les clients abonnés.
    pub(super) fn on_set(&mut self, id: ClientId, set: Set) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        if set.key.starts_with("_read_") {
            self.send(id, vec![invalid_packet("arguments", "Set", "Set")]);
            return;
        }
        let default = set.default.clone().unwrap_or(Value::from(0));
        let stored = self.store.get(&set.key).cloned();
        // Vrai tant que la valeur est le même objet Python que `default`
        // (voir `datastore::apply_tracked`).
        let mut is_default_object = stored.is_none() && set.default.is_some();
        // Ce que deviendra `default` dans le `SetReply` : modifié sur place tant
        // que la valeur reste le même objet, puis figé.
        let mut default_shown = set.default.clone();
        let original = stored.unwrap_or(default);
        let mut value = original.clone();
        for operation in &set.operations {
            match datastore::apply_tracked(&operation.operation, value, &operation.value) {
                Ok((new_value, same_object)) => {
                    is_default_object &= same_object;
                    if is_default_object {
                        default_shown = Some(new_value.clone());
                    }
                    value = new_value;
                }
                Err(error) => {
                    // Le serveur officiel plante ici et ferme la connexion ;
                    // la valeur rangée ne change pas.
                    self.close(id, &error.to_string());
                    return;
                }
            }
        }
        self.store.set(set.key.clone(), value.clone());

        // Le `SetReply` reprend tout le `Set` : options, opérations, champs libres.
        let mut extra = Map::new();
        if let Some(default) = default_shown {
            extra.insert("default".to_string(), default);
        }
        if let Some(want_reply) = set.want_reply {
            extra.insert("want_reply".to_string(), Value::Bool(want_reply));
        }
        let operations = serde_json::to_value(&set.operations).unwrap_or(Value::Null);
        extra.insert("operations".to_string(), operations);
        extra.extend(set.extra);
        let reply = SetReply {
            key: set.key.clone(),
            value,
            original_value: Some(original),
            slot: Some(player.slot),
            extra,
        };

        let mut targets = self.subscribers.get(&set.key).cloned().unwrap_or_default();
        if set.want_reply.unwrap_or(false) {
            targets.insert(id);
        }
        self.send_to(&targets, &[ServerPacket::SetReply(reply)]);
    }

    /// Prévient les abonnés d'une clé `_read_…` qui vient de changer.
    pub(super) fn notify_read_key(&self, key: String, value: Value) {
        let Some(targets) = self.subscribers.get(&key) else {
            return;
        };
        let reply = SetReply {
            key,
            value,
            original_value: None,
            slot: None,
            extra: Map::new(),
        };
        self.send_to(targets, &[ServerPacket::SetReply(reply)]);
    }
}
