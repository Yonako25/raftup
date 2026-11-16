//! Connexion d'un client : `Connect`, `ConnectUpdate`, `Sync`, `GetDataPackage`,
//! départ du client et données des joueurs.

use super::*;

/// Choix de réception des objets (`items_handling`), décodé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ItemsChoice {
    /// Aucun objet : le jeu gère tout lui-même.
    pub(super) no_items: bool,
    /// Recevoir aussi les objets trouvés dans son propre monde.
    pub(super) remote_items: bool,
    /// Recevoir l'inventaire de départ.
    pub(super) remote_start_inventory: bool,
}

impl ItemsChoice {
    /// Décode `items_handling` ; `None` si la combinaison est impossible
    /// (bits 2 ou 3 sans le bit 1), comme le refuse le serveur officiel.
    pub(super) fn decode(value: u8) -> Option<ItemsChoice> {
        if value & 0b001 == 0 && value & 0b110 != 0 {
            return None;
        }
        Some(ItemsChoice {
            no_items: value & 0b001 == 0,
            remote_items: value & 0b010 != 0,
            remote_start_inventory: value & 0b100 != 0,
        })
    }

    /// Valeur actuelle, recalculée comme la propriété `items_handling` officielle.
    pub(super) fn encode(self) -> u8 {
        if self.no_items {
            return 0;
        }
        1 + (u8::from(self.remote_items) << 1) + (u8::from(self.remote_start_inventory) << 2)
    }
}

/// Résultat de la vérification d'un `Connect`.
pub(super) enum ConnectCheck {
    Accepted {
        team: TeamId,
        slot: SlotId,
        items: ItemsChoice,
    },
    /// Refus ; `items` : le `items_handling` demandé, s'il est valable et que
    /// le nom existe (le serveur officiel l'applique même en cas de refus).
    Refused {
        errors: Vec<String>,
        items: Option<ItemsChoice>,
    },
    /// Cas où le serveur officiel plante et ferme la connexion.
    Fatal(&'static str),
}

impl RoomTask {
    pub(super) fn on_connect(&mut self, id: ClientId, connect: &Connect) {
        let (team, slot, items) = match self.check_connect(connect) {
            ConnectCheck::Accepted { team, slot, items } => (team, slot, items),
            ConnectCheck::Refused { errors, items } => {
                info!(id, ?errors, name = %connect.name, "connexion refusée");
                // Le serveur officiel range le nouveau `items_handling` avant de
                // décider du refus : un client déjà connecté le garde.
                if let Some(items) = items {
                    if let Some(player) = self.clients.get_mut(&id).and_then(|c| c.player.as_mut())
                    {
                        player.items = items;
                    }
                }
                let refused = ConnectionRefused { errors };
                self.send(id, vec![ServerPacket::ConnectionRefused(refused)]);
                return;
            }
            ConnectCheck::Fatal(reason) => {
                self.close(id, reason);
                return;
            }
        };

        // `clone()` : on garde une copie de l'ancien joueur, car `self` sera
        // modifié plus bas (le compilateur interdit de garder une référence).
        let previous = self.player(id).cloned();
        if let Some(previous) = &previous {
            if (previous.team, previous.slot) != (team, slot) {
                // Le serveur officiel 0.6.8 plante dans ce cas et ferme la connexion.
                self.close(id, "changement de slot sur la même connexion");
                return;
            }
        }
        // Le serveur officiel plante aussi pour un slot sans lieux (un groupe).
        let Some(missing_locations) = self.game.missing(self.room.data(), team, slot) else {
            self.close(id, "slot sans lieux (groupe)");
            return;
        };

        let data = self.room.data();
        // Absent = vrai, comme sur le serveur officiel.
        let slot_data = if connect.slot_data.unwrap_or(true) {
            let empty = Value::Object(Default::default());
            Some(data.slot_data.get(&slot).cloned().unwrap_or(empty))
        } else {
            None
        };
        let connected = Connected {
            team,
            slot,
            players: self.players(),
            missing_locations,
            checked_locations: self.game.checked(team, slot),
            slot_data,
            slot_info: data
                .slots
                .iter()
                .map(|(s, info)| (*s, info.clone()))
                .collect(),
            hint_points: self.hint_points(team, slot),
        };

        let mut player = Player {
            team,
            slot,
            version: connect.version,
            tags: Vec::new(),
            items,
            // Une reconnexion garde son compteur, comme sur le serveur officiel.
            send_index: previous.as_ref().map_or(0, |p| p.send_index),
            no_locations: false,
            no_text: false,
        };
        player.set_tags(connect.tags.clone());

        let mut reply = vec![ServerPacket::Connected(connected)];
        let all_items = self.all_items(&player);
        if !all_items.is_empty() && !player.items.no_items {
            player.send_index = all_items.len();
            reply.push(ServerPacket::ReceivedItems(ReceivedItems {
                index: 0,
                items: all_items,
            }));
        }
        if let Some(client) = self.clients.get_mut(&id) {
            client.player = Some(player.clone());
        }
        info!(id, team, slot, name = %connect.name, "joueur connecté");
        self.send(id, reply);

        // Une reconnexion sur la même connexion ne s'annonce pas.
        if previous.is_none() {
            self.on_joined(id, &player);
        }
    }

    /// Vérifie un `Connect`, dans l'ordre du serveur officiel.
    pub(super) fn check_connect(&self, connect: &Connect) -> ConnectCheck {
        let data = self.room.data();
        // `BTreeSet` : pas de doublon, et un ordre fixe (alphabétique).
        let mut errors = BTreeSet::new();
        if let Some(password) = self.room.password() {
            if connect.password.as_deref() != Some(password) {
                errors.insert("InvalidPassword");
            }
        }

        let mut accepted = None;
        match data.connect_names.get(&connect.name) {
            None => {
                errors.insert("InvalidSlot");
            }
            Some(&(team, slot)) => {
                let game = data.slots.get(&slot).map(|s| s.game.as_str()).unwrap_or("");
                let ignore_game = connect.game.is_empty()
                    && connect
                        .tags
                        .iter()
                        .any(|t| NON_GAME_TAGS.iter().any(|(tag, _)| t == tag));
                if !ignore_game && connect.game != game {
                    errors.insert("InvalidGame");
                }
                let minimum = if ignore_game {
                    MIN_CLIENT_VERSION
                } else {
                    match self.minimum_client_version(slot) {
                        Some(version) => version,
                        // Pas de version minimale pour ce slot (un groupe) :
                        // le serveur officiel plante et ferme la connexion.
                        None => return ConnectCheck::Fatal("slot sans version minimale"),
                    }
                };
                if version_key(&minimum) > version_key(&connect.version) {
                    errors.insert("IncompatibleVersion");
                }
                match ItemsChoice::decode(connect.items_handling.0) {
                    Some(items) => accepted = Some((team, slot, items)),
                    None => {
                        errors.insert("InvalidItemsHandling");
                    }
                }
            }
        }
        // Compatibilité 0 : seule la version exacte du serveur est acceptée.
        if data.server_options.compatibility == 0 && connect.version != SERVER_VERSION {
            errors.insert("IncompatibleVersion");
        }

        match accepted {
            Some((team, slot, items)) if errors.is_empty() => {
                ConnectCheck::Accepted { team, slot, items }
            }
            _ => ConnectCheck::Refused {
                errors: errors.into_iter().map(String::from).collect(),
                items: accepted.map(|(_, _, items)| items),
            },
        }
    }

    /// Version minimale du client pour ce slot, au moins celle du serveur officiel.
    pub(super) fn minimum_client_version(&self, slot: SlotId) -> Option<NetworkVersion> {
        let data = self.room.data();
        let floor = if version_key(&data.generator_version) < (0, 6, 2) {
            OLD_MIN_CLIENT_VERSION
        } else {
            MIN_CLIENT_VERSION
        };
        let version = data.minimum_client_versions.get(&slot)?;
        Some(if version_key(version) > version_key(&floor) {
            *version
        } else {
            floor
        })
    }

    /// Annonce l'arrivée d'un joueur à tous, puis lui envoie le tutoriel.
    pub(super) fn on_joined(&mut self, id: ClientId, player: &Player) {
        let status = self.game.status(player.team, player.slot);
        if status.unwrap_or(ClientStatus::UNKNOWN) == ClientStatus::UNKNOWN {
            self.update_client_status(player.team, player.slot, ClientStatus::CONNECTED);
        }
        let name = self.slot_name(player.slot);
        let game = self.slot_game(player.slot);
        self.emit(GameEvent::Joined {
            time: now(),
            slot: player.slot,
            name: name.clone(),
            game: game.clone(),
            tags: player.tags.clone(),
        });
        let verb = non_game_verb(&player.tags).unwrap_or("playing");
        let text = format!(
            "{} (Team #{}) {verb} {game} has joined. Client({}), {}.",
            self.aliased_name(player.team, player.slot),
            player.team + 1,
            version_text(&player.version),
            python::list_repr(&player.tags),
        );
        let mut join = text_message(text);
        join.kind = Some("Join".to_string());
        join.team = Some(player.team);
        join.slot = Some(player.slot);
        join.tags = Some(player.tags.clone());
        self.broadcast_text_all(join);

        let mut tutorial = text_message(TUTORIAL.to_string());
        tutorial.kind = Some("Tutorial".to_string());
        self.notify(id, tutorial);
    }

    /// Un client est parti : on l'oublie, et on annonce son départ s'il jouait.
    pub(super) fn on_closed(&mut self, id: ClientId) {
        let Some(client) = self.clients.remove(&id) else {
            return;
        };
        // Une nouvelle connexion devra se reconnecter en administrateur.
        if self.admin_client == Some(id) {
            self.admin_client = None;
        }
        // Plus d'abonnement pour ce client.
        for subscribers in self.subscribers.values_mut() {
            subscribers.remove(&id);
        }
        let Some(player) = client.player else {
            return;
        };
        // Plus aucune connexion sur ce slot : statut « inconnu ».
        let others = self.clients.values().filter_map(|c| c.player.as_ref());
        if !others
            .into_iter()
            .any(|p| (p.team, p.slot) == (player.team, player.slot))
        {
            self.update_client_status(player.team, player.slot, ClientStatus::UNKNOWN);
        }
        let name = self.slot_name(player.slot);
        self.emit(GameEvent::Left {
            time: now(),
            slot: player.slot,
            name: name.clone(),
        });
        let verb = match non_game_verb(&player.tags) {
            Some(verb) => format!("stopped {verb}"),
            None => "left".to_string(),
        };
        let text = format!(
            "{} (Team #{}) has {verb} the game. Client({}), {}.",
            self.aliased_name(player.team, player.slot),
            player.team + 1,
            version_text(&player.version),
            python::list_repr(&player.tags),
        );
        let mut part = text_message(text);
        part.kind = Some("Part".to_string());
        part.team = Some(player.team);
        part.slot = Some(player.slot);
        info!(id, slot = player.slot, "joueur parti");
        self.broadcast_text_all(part);
    }

    /// Change `items_handling` ou les tags d'un joueur déjà connecté.
    pub(super) fn on_connect_update(&mut self, id: ClientId, update: ConnectUpdate) {
        let Some(mut player) = self.player(id).cloned() else {
            return;
        };
        if let Some(handling) = update.items_handling {
            if handling.0 != player.items.encode() {
                let Some(items) = ItemsChoice::decode(handling.0) else {
                    let text = "Invalid items_handling: Invalid flag combination";
                    self.send(id, vec![invalid_packet("arguments", "ConnectUpdate", text)]);
                    return;
                };
                player.items = items;
                let all_items = self.all_items(&player);
                if !all_items.is_empty() && !player.items.no_items {
                    player.send_index = all_items.len();
                    self.set_player(id, player.clone());
                    let packet = ReceivedItems {
                        index: 0,
                        items: all_items,
                    };
                    self.send(id, vec![ServerPacket::ReceivedItems(packet)]);
                } else {
                    player.send_index = 0;
                }
            }
        }
        if let Some(tags) = update.tags {
            let old_tags = player.tags.clone();
            // Comparaison comme des ensembles : l'ordre et les doublons ne comptent pas.
            let changed = to_set(&old_tags) != to_set(&tags);
            if changed {
                player.set_tags(tags);
            } else {
                player.tags = tags;
            }
            self.set_player(id, player.clone());
            if changed {
                let text = format!(
                    "{} (Team #{}) has changed tags from {} to {}.",
                    self.aliased_name(player.team, player.slot),
                    player.team + 1,
                    python::list_repr(&old_tags),
                    python::list_repr(&player.tags),
                );
                let mut message = text_message(text);
                message.kind = Some("TagsChanged".to_string());
                message.team = Some(player.team);
                message.slot = Some(player.slot);
                message.tags = Some(player.tags.clone());
                self.broadcast_text_all(message);
            }
        }
        self.set_player(id, player);
    }

    /// Renvoie toute la liste des objets reçus, depuis le début.
    pub(super) fn on_sync(&mut self, id: ClientId) {
        let Some(mut player) = self.player(id).cloned() else {
            return;
        };
        let all_items = self.all_items(&player);
        if !all_items.is_empty() && !player.items.no_items {
            player.send_index = all_items.len();
            self.set_player(id, player);
            let packet = ReceivedItems {
                index: 0,
                items: all_items,
            };
            self.send(id, vec![ServerPacket::ReceivedItems(packet)]);
        }
    }

    /// Noms des objets et des lieux des jeux demandés (ceux de la partie seulement).
    pub(super) fn on_get_data_package(&mut self, id: ClientId, request: &GetDataPackage) {
        let wanted: BTreeSet<&str> = request.games.iter().flatten().map(String::as_str).collect();
        let mut games = IndexMap::new();
        // Même ordre que le serveur officiel : les jeux de la partie, triés.
        for game in self.room.played_games() {
            if !wanted.contains(game.as_str()) {
                continue;
            }
            if let Some(package) = self.room.data().datapackage.get(&game) {
                let data = GameData {
                    item_name_to_id: package.item_name_to_id.clone(),
                    location_name_to_id: package.location_name_to_id.clone(),
                    checksum: package.checksum.clone(),
                };
                games.insert(game, data);
            }
        }
        let package = DataPackage {
            data: DataPackageData { games },
        };
        self.send(id, vec![ServerPacket::DataPackage(package)]);
    }

    /// Inventaire de départ (si demandé) puis objets reçus : la liste complète
    /// que le client doit avoir, dans l'ordre.
    pub(super) fn all_items(&self, player: &Player) -> Vec<NetworkItem> {
        let mut items = Vec::new();
        if player.items.remote_start_inventory {
            items.extend(self.start_inventory(player.slot));
        }
        let received = self
            .game
            .received(player.team, player.slot, player.items.remote_items);
        items.extend_from_slice(received);
        items
    }

    /// Inventaire de départ : lieu `-2`, joueur `0` (le serveur).
    pub(super) fn start_inventory(&self, slot: SlotId) -> Vec<NetworkItem> {
        let items = self.room.data().precollected_items.get(&slot);
        // `into_iter().flatten()` : rien si le slot n'a pas d'entrée.
        items
            .into_iter()
            .flatten()
            .map(|&item| NetworkItem {
                item,
                location: -2,
                player: 0,
                flags: ItemFlags(0),
            })
            .collect()
    }

    /// Tous les slots de l'équipe 0, groupes compris, dans l'ordre des slots.
    pub(super) fn players(&self) -> Vec<NetworkPlayer> {
        let slots = &self.room.data().slots;
        slots
            .iter()
            .map(|(slot, info)| NetworkPlayer {
                team: 0,
                slot: *slot,
                alias: self.aliased_name(0, *slot),
                name: info.name.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_handling_comme_python() {
        assert_eq!(ItemsChoice::decode(0b110), None);
        assert_eq!(ItemsChoice::decode(0b010), None);
        for value in [0, 1, 3, 5, 7] {
            assert_eq!(
                ItemsChoice::decode(value).map(ItemsChoice::encode),
                Some(value)
            );
        }
    }
}
