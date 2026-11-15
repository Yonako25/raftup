//! La tâche salle : elle possède tout l'état de la partie (ADR 0006).
//!
//! Elle reçoit des événements des connexions (arrivée, paquets, départ) et les
//! traite un par un, dans l'ordre, comme la boucle unique du serveur officiel.
//! Les réponses partent par le canal de chaque client.
//!
//! Gérés : `Connect`, `GetDataPackage`, `ConnectUpdate`, `Sync`, `LocationChecks`,
//! `StatusUpdate` (avec `collect` et `release` automatiques à l'objectif).
//! Les autres paquets de jeu arrivent dans la suite de l'étape 5 de J1.

use std::collections::{BTreeMap, BTreeSet};

use ap_protocol::client::{Connect, ConnectUpdate, GetDataPackage, LocationChecks};
use ap_protocol::server::{
    Connected, ConnectionRefused, DataPackage, DataPackageData, GameData, InvalidPacket,
    JsonMessagePart, PrintJson, ReceivedItems, RoomUpdate,
};
use ap_protocol::{
    encode_server_frame, ClientPacket, ClientStatus, ItemFlags, LocationId, NetworkItem,
    NetworkPlayer, NetworkVersion, ServerPacket, SlotId, SlotType, TeamId,
};
use indexmap::IndexMap;
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{debug, error, info, warn};

use crate::connection::Incoming;
use crate::game::{locations_for_player, GameState, Sent};
use crate::room::{now, Room, SERVER_VERSION};

/// Numéro unique d'une connexion, donné à son arrivée.
pub(crate) type ClientId = u64;

/// Ce qu'une connexion signale à la tâche salle.
#[derive(Debug)]
pub(crate) enum Event {
    /// Nouveau client ; `outbox` sert à lui envoyer des trames.
    Opened {
        id: ClientId,
        outbox: UnboundedSender<Outgoing>,
    },
    /// Une trame reçue, déjà découpée en paquets.
    Frame { id: ClientId, items: Vec<Incoming> },
    /// Le client est parti (ou sa connexion a échoué).
    Closed { id: ClientId },
}

/// Ce que la tâche salle demande à une connexion.
#[derive(Debug)]
pub(crate) enum Outgoing {
    /// Trame JSON à envoyer telle quelle.
    Frame(String),
    /// Fermer la connexion.
    Close,
}

/// Version minimale d'un client (`min_client_version` du serveur officiel).
const MIN_CLIENT_VERSION: NetworkVersion = NetworkVersion {
    major: 0,
    minor: 5,
    build: 0,
};

/// Ancienne version minimale, pour les seeds d'avant 0.6.2.
const OLD_MIN_CLIENT_VERSION: NetworkVersion = NetworkVersion {
    major: 0,
    minor: 1,
    build: 6,
};

/// Tags des outils qui ne jouent pas, et le verbe affiché quand ils arrivent.
/// L'ordre compte : c'est celui du serveur officiel (`_non_game_messages`).
const NON_GAME_TAGS: [(&str, &str); 3] = [
    ("HintGame", "hinting"),
    ("Tracker", "tracking"),
    ("TextOnly", "viewing"),
];

const TUTORIAL: &str = "Now that you are connected, you can use !help to list commands to run \
via the server. If your client supports it, you may have additional local commands you can \
list with /help.";

/// Nombre maximal de messages `ItemSend` par trame, comme le serveur officiel.
const ITEM_SEND_CHUNK: usize = 140;

/// Choix de réception des objets (`items_handling`), décodé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ItemsChoice {
    /// Aucun objet : le jeu gère tout lui-même.
    no_items: bool,
    /// Recevoir aussi les objets trouvés dans son propre monde.
    remote_items: bool,
    /// Recevoir l'inventaire de départ.
    remote_start_inventory: bool,
}

impl ItemsChoice {
    /// Décode `items_handling` ; `None` si la combinaison est impossible
    /// (bits 2 ou 3 sans le bit 1), comme le refuse le serveur officiel.
    fn decode(value: u8) -> Option<ItemsChoice> {
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
    fn encode(self) -> u8 {
        if self.no_items {
            return 0;
        }
        1 + (u8::from(self.remote_items) << 1) + (u8::from(self.remote_start_inventory) << 2)
    }
}

/// Un joueur authentifié (après un `Connect` accepté).
#[derive(Debug, Clone)]
struct Player {
    team: TeamId,
    slot: SlotId,
    version: NetworkVersion,
    tags: Vec<String>,
    items: ItemsChoice,
    /// Nombre d'objets (inventaire de départ compris) déjà envoyés à ce client.
    send_index: usize,
    /// Vrai pour un outil qui ne joue pas : il ne peut pas visiter de lieux.
    no_locations: bool,
    /// Vrai si le client ne veut pas des messages texte (`PrintJSON`).
    no_text: bool,
}

impl Player {
    fn set_tags(&mut self, tags: Vec<String>) {
        let has_tag = |tag: &str| tags.iter().any(|t| t == tag);
        self.no_locations = NON_GAME_TAGS.iter().any(|(tag, _)| has_tag(tag));
        self.no_text =
            has_tag("NoText") || (has_tag("PopTracker") && version_key(&self.version) < (0, 5, 1));
        self.tags = tags;
    }
}

/// Une connexion vue par la salle.
#[derive(Debug)]
struct Client {
    outbox: UnboundedSender<Outgoing>,
    /// `None` tant que le client n'a pas réussi son `Connect`.
    player: Option<Player>,
    /// Vrai quand la salle a demandé la fermeture : on ignore la suite.
    closing: bool,
}

/// Résultat de la vérification d'un `Connect`.
enum ConnectCheck {
    Accepted {
        team: TeamId,
        slot: SlotId,
        items: ItemsChoice,
    },
    Refused(Vec<String>),
    /// Cas où le serveur officiel plante et ferme la connexion.
    Fatal(&'static str),
}

/// Fait tourner la tâche salle jusqu'à ce que plus personne ne puisse lui écrire.
pub(crate) async fn run(room: Room, mut events: UnboundedReceiver<Event>) {
    let game = GameState::new(room.data());
    let mut task = RoomTask {
        room,
        game,
        clients: BTreeMap::new(),
    };
    while let Some(event) = events.recv().await {
        task.handle(event);
    }
}

struct RoomTask {
    room: Room,
    game: GameState,
    /// `BTreeMap` : les diffusions suivent l'ordre d'arrivée des clients.
    clients: BTreeMap<ClientId, Client>,
}

impl RoomTask {
    fn handle(&mut self, event: Event) {
        match event {
            Event::Opened { id, outbox } => {
                let client = Client {
                    outbox,
                    player: None,
                    closing: false,
                };
                self.clients.insert(id, client);
                let info = self.room.room_info(now());
                self.send(id, vec![ServerPacket::RoomInfo(info)]);
            }
            Event::Frame { id, items } => self.on_frame(id, items),
            Event::Closed { id } => self.on_closed(id),
        }
    }

    fn on_frame(&mut self, id: ClientId, items: Vec<Incoming>) {
        for item in items {
            // Après une demande de fermeture, la suite de la trame est ignorée.
            match self.clients.get(&id) {
                Some(client) if !client.closing => {}
                _ => return,
            }
            match item {
                Incoming::Packet(packet) => self.on_packet(id, *packet),
                Incoming::BadArguments { cmd, error } => {
                    debug!(id, %cmd, "paquet aux champs incorrects : {error}");
                    if cmd == "Connect" {
                        let reply = invalid_packet("arguments", "Connect", "Connect");
                        self.send(id, vec![reply]);
                    }
                }
                Incoming::CmdNotText => {
                    let reply = invalid_cmd("cmd doit être du texte");
                    self.send(id, vec![reply]);
                }
                Incoming::NoCmd => {
                    let reply = invalid_cmd("paquet sans champ « cmd »");
                    self.send(id, vec![reply]);
                    self.close(id, "paquet sans « cmd »");
                }
            }
        }
    }

    fn on_packet(&mut self, id: ClientId, packet: ClientPacket) {
        match packet {
            ClientPacket::Connect(connect) => return self.on_connect(id, &connect),
            ClientPacket::GetDataPackage(request) => return self.on_get_data_package(id, &request),
            _ => {}
        }
        // Les autres paquets demandent un joueur connecté ; sinon, ignorés
        // sans réponse, comme sur le serveur officiel.
        if self.player(id).is_none() {
            debug!(id, cmd = packet.cmd(), "paquet avant connexion, ignoré");
            return;
        }
        match packet {
            ClientPacket::ConnectUpdate(update) => self.on_connect_update(id, update),
            ClientPacket::Sync => self.on_sync(id),
            ClientPacket::LocationChecks(checks) => self.on_location_checks(id, &checks),
            ClientPacket::StatusUpdate(update) => self.on_status_update(id, update.status),
            other => debug!(id, cmd = other.cmd(), "paquet pas encore géré, ignoré"),
        }
    }

    // ----- Connect -------------------------------------------------------

    fn on_connect(&mut self, id: ClientId, connect: &Connect) {
        let (team, slot, items) = match self.check_connect(connect) {
            ConnectCheck::Accepted { team, slot, items } => (team, slot, items),
            ConnectCheck::Refused(errors) => {
                info!(id, ?errors, name = %connect.name, "connexion refusée");
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
    fn check_connect(&self, connect: &Connect) -> ConnectCheck {
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
            _ => ConnectCheck::Refused(errors.into_iter().map(String::from).collect()),
        }
    }

    /// Version minimale du client pour ce slot, au moins celle du serveur officiel.
    fn minimum_client_version(&self, slot: SlotId) -> Option<NetworkVersion> {
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
    fn on_joined(&mut self, id: ClientId, player: &Player) {
        let status = self.game.status(player.team, player.slot);
        if status.unwrap_or(ClientStatus::UNKNOWN) == ClientStatus::UNKNOWN {
            self.update_client_status(player.team, player.slot, ClientStatus::CONNECTED);
        }
        let name = self.slot_name(player.slot);
        let game = self.slot_game(player.slot);
        let verb = non_game_verb(&player.tags).unwrap_or("playing");
        let text = format!(
            "{name} (Team #{}) {verb} {game} has joined. Client({}), {}.",
            player.team + 1,
            version_text(&player.version),
            python_list_repr(&player.tags),
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
    fn on_closed(&mut self, id: ClientId) {
        let Some(client) = self.clients.remove(&id) else {
            return;
        };
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
        let verb = match non_game_verb(&player.tags) {
            Some(verb) => format!("stopped {verb}"),
            None => "left".to_string(),
        };
        let text = format!(
            "{name} (Team #{}) has {verb} the game. Client({}), {}.",
            player.team + 1,
            version_text(&player.version),
            python_list_repr(&player.tags),
        );
        let mut part = text_message(text);
        part.kind = Some("Part".to_string());
        part.team = Some(player.team);
        part.slot = Some(player.slot);
        info!(id, slot = player.slot, "joueur parti");
        self.broadcast_text_all(part);
    }

    // ----- ConnectUpdate et Sync -----------------------------------------

    /// Change `items_handling` ou les tags d'un joueur déjà connecté.
    fn on_connect_update(&mut self, id: ClientId, update: ConnectUpdate) {
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
                    self.slot_name(player.slot),
                    player.team + 1,
                    python_list_repr(&old_tags),
                    python_list_repr(&player.tags),
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
    fn on_sync(&mut self, id: ClientId) {
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

    // ----- LocationChecks ------------------------------------------------

    fn on_location_checks(&mut self, id: ClientId, checks: &LocationChecks) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        if player.no_locations {
            let text = "Trackers can't register new Location Checks";
            self.send(id, vec![invalid_packet("cmd", "LocationChecks", text)]);
            return;
        }
        self.register_location_checks(player.team, player.slot, &checks.locations);
    }

    /// Enregistre des lieux visités et prévient tout le monde
    /// (`register_location_checks` du serveur officiel).
    fn register_location_checks(&mut self, team: TeamId, slot: SlotId, locations: &[LocationId]) {
        let (new_locations, sent) =
            self.game
                .register_checks(self.room.data(), team, slot, locations);
        if new_locations.is_empty() {
            return;
        }
        info!(team, slot, count = new_locations.len(), "lieux visités");

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
    }

    /// Envoie aux clients d'un slot la liste complète de ses lieux visités.
    fn update_checked_locations(&mut self, team: TeamId, slot: SlotId) {
        let update = RoomUpdate {
            checked_locations: Some(self.game.checked(team, slot)),
            ..RoomUpdate::default()
        };
        self.broadcast_slot(team, slot, vec![ServerPacket::RoomUpdate(update)]);
    }

    // ----- Statut et objectif --------------------------------------------

    fn on_status_update(&mut self, id: ClientId, status: ClientStatus) {
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
    fn update_client_status(&mut self, team: TeamId, slot: SlotId, status: ClientStatus) {
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
    }

    /// Annonce l'objectif atteint, puis `collect` et `release` automatiques
    /// si les options de la salle le prévoient (`auto` ou `auto-enabled`).
    fn on_goal_achieved(&mut self, team: TeamId, slot: SlotId) {
        let text = format!(
            "{} (Team #{}) has completed their goal.",
            self.slot_name(slot),
            team + 1
        );
        info!(team, slot, "objectif atteint");
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
    fn release_player(&mut self, team: TeamId, slot: SlotId) {
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
    fn collect_player(&mut self, team: TeamId, slot: SlotId, is_group: bool) {
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
    fn send_new_items(&mut self) {
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

    // ----- GetDataPackage ------------------------------------------------

    /// Noms des objets et des lieux des jeux demandés (ceux de la partie seulement).
    fn on_get_data_package(&mut self, id: ClientId, request: &GetDataPackage) {
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

    // ----- Données des joueurs -------------------------------------------

    fn player(&self, id: ClientId) -> Option<&Player> {
        self.clients.get(&id).and_then(|c| c.player.as_ref())
    }

    fn set_player(&mut self, id: ClientId, player: Player) {
        if let Some(client) = self.clients.get_mut(&id) {
            client.player = Some(player);
        }
    }

    /// Inventaire de départ (si demandé) puis objets reçus : la liste complète
    /// que le client doit avoir, dans l'ordre.
    fn all_items(&self, player: &Player) -> Vec<NetworkItem> {
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
    fn start_inventory(&self, slot: SlotId) -> Vec<NetworkItem> {
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

    /// Points d'indice : gagnés en visitant des lieux.
    /// Les indices achetés (qui en coûtent) arriveront avec les indices, en J2.
    fn hint_points(&self, team: TeamId, slot: SlotId) -> i64 {
        let per_check = self.room.data().server_options.location_check_points;
        let checked = i64::try_from(self.game.checked_count(team, slot)).unwrap_or(i64::MAX);
        per_check.saturating_mul(checked)
    }

    /// Tous les slots de l'équipe 0, groupes compris, dans l'ordre des slots.
    fn players(&self) -> Vec<NetworkPlayer> {
        let slots = &self.room.data().slots;
        slots
            .iter()
            .map(|(slot, info)| NetworkPlayer {
                team: 0,
                slot: *slot,
                alias: info.name.clone(),
                name: info.name.clone(),
            })
            .collect()
    }

    fn slot_name(&self, slot: SlotId) -> String {
        let slots = &self.room.data().slots;
        slots.get(&slot).map(|s| s.name.clone()).unwrap_or_default()
    }

    fn slot_game(&self, slot: SlotId) -> String {
        let slots = &self.room.data().slots;
        slots.get(&slot).map(|s| s.game.clone()).unwrap_or_default()
    }

    // ----- Envois --------------------------------------------------------

    /// Envoie des paquets à un client, dans une seule trame.
    fn send(&self, id: ClientId, packets: Vec<ServerPacket>) {
        let Some(client) = self.clients.get(&id) else {
            return;
        };
        if let Some(frame) = encode(&packets) {
            // Échec = connexion déjà finie ; son événement `Closed` va arriver.
            let _ = client.outbox.send(Outgoing::Frame(frame));
        }
    }

    /// Envoie la même trame aux joueurs connectés qui passent le filtre.
    fn broadcast(&self, packets: &[ServerPacket], filter: impl Fn(&Player) -> bool) {
        let Some(frame) = encode(packets) else {
            return;
        };
        for client in self.clients.values() {
            let wanted = client.player.as_ref().is_some_and(&filter);
            if wanted && !client.closing {
                let _ = client.outbox.send(Outgoing::Frame(frame.clone()));
            }
        }
    }

    /// Message texte pour un seul client connecté (`notify_client`).
    fn notify(&self, id: ClientId, message: PrintJson) {
        if self.player(id).is_some_and(|p| !p.no_text) {
            self.send(id, vec![ServerPacket::PrintJson(message)]);
        }
    }

    /// Message texte pour tous les clients connectés (`broadcast_text_all`).
    fn broadcast_text_all(&self, message: PrintJson) {
        self.broadcast(&[ServerPacket::PrintJson(message)], |p| !p.no_text);
    }

    /// Messages texte pour une équipe (`broadcast_team`).
    fn broadcast_team_text(&self, team: TeamId, messages: Vec<ServerPacket>) {
        self.broadcast(&messages, |p| p.team == team && !p.no_text);
    }

    /// Paquets pour les clients d'un slot (`ctx.broadcast(ctx.clients[team][slot])`).
    fn broadcast_slot(&self, team: TeamId, slot: SlotId, packets: Vec<ServerPacket>) {
        self.broadcast(&packets, |p| p.team == team && p.slot == slot);
    }

    /// Demande la fermeture d'une connexion.
    fn close(&mut self, id: ClientId, reason: &str) {
        if let Some(client) = self.clients.get_mut(&id) {
            warn!(id, "fermeture de la connexion : {reason}");
            client.closing = true;
            let _ = client.outbox.send(Outgoing::Close);
        }
    }
}

fn encode(packets: &[ServerPacket]) -> Option<String> {
    match encode_server_frame(packets) {
        Ok(frame) => Some(frame),
        Err(err) => {
            error!("impossible d'écrire un paquet en JSON : {err}");
            None
        }
    }
}

fn invalid_packet(kind: &str, original_cmd: &str, text: &str) -> ServerPacket {
    ServerPacket::InvalidPacket(InvalidPacket {
        kind: kind.to_string(),
        original_cmd: Some(original_cmd.to_string()),
        text: text.to_string(),
    })
}

/// `InvalidPacket` de type `cmd`, sans commande d'origine.
fn invalid_cmd(text: &str) -> ServerPacket {
    ServerPacket::InvalidPacket(InvalidPacket {
        kind: "cmd".to_string(),
        original_cmd: None,
        text: text.to_string(),
    })
}

fn to_set(tags: &[String]) -> BTreeSet<&str> {
    tags.iter().map(String::as_str).collect()
}

/// Verbe affiché pour un outil qui ne joue pas (`viewing`…), selon ses tags.
fn non_game_verb(tags: &[String]) -> Option<&'static str> {
    NON_GAME_TAGS
        .iter()
        .find(|(tag, _)| tags.iter().any(|t| t == tag))
        .map(|(_, verb)| *verb)
}

/// Morceau de texte simple d'un `PrintJSON`.
fn text_part(text: impl Into<String>) -> JsonMessagePart {
    JsonMessagePart {
        text: Some(text.into()),
        kind: None,
        color: None,
        flags: None,
        player: None,
        hint_status: None,
    }
}

/// `PrintJSON` d'un seul morceau de texte, sans genre.
fn text_message(text: String) -> PrintJson {
    PrintJson {
        data: vec![text_part(text)],
        kind: None,
        receiving: None,
        item: None,
        found: None,
        team: None,
        slot: None,
        message: None,
        tags: None,
        countdown: None,
    }
}

/// Message texte d'un genre donné, qui concerne un slot (`Goal`, `Release`…).
fn slot_message(text: String, kind: &str, team: TeamId, slot: SlotId) -> PrintJson {
    let mut message = text_message(text);
    message.kind = Some(kind.to_string());
    message.team = Some(team);
    message.slot = Some(slot);
    message
}

/// Message « Alice sent Objet to Bob (Lieu) » (`json_format_send_event`).
/// Les numéros sont remplacés par des noms chez le client, grâce au DataPackage.
fn item_send_message(sent: &Sent) -> PrintJson {
    let item = sent.item;
    let player_part = |slot: SlotId| JsonMessagePart {
        kind: Some("player_id".to_string()),
        ..text_part(slot.to_string())
    };
    let item_part = |owner: SlotId| JsonMessagePart {
        kind: Some("item_id".to_string()),
        player: Some(owner),
        flags: Some(item.flags),
        ..text_part(item.item.to_string())
    };
    let mut parts = vec![player_part(item.player)];
    if item.player == sent.receiver {
        parts.push(text_part(" found their "));
        parts.push(item_part(item.player));
    } else {
        parts.push(text_part(" sent "));
        parts.push(item_part(sent.receiver));
        parts.push(text_part(" to "));
        parts.push(player_part(sent.receiver));
    }
    parts.push(text_part(" ("));
    parts.push(JsonMessagePart {
        kind: Some("location_id".to_string()),
        player: Some(item.player),
        ..text_part(item.location.to_string())
    });
    parts.push(text_part(")"));

    let mut message = text_message(String::new());
    message.data = parts;
    message.kind = Some("ItemSend".to_string());
    message.receiving = Some(sent.receiver);
    message.item = Some(item);
    message
}

/// Version sous forme de triplet, pour comparer comme Python compare des tuples.
fn version_key(version: &NetworkVersion) -> (u32, u32, u32) {
    (version.major, version.minor, version.build)
}

/// `0.6.8`
fn version_text(version: &NetworkVersion) -> String {
    format!("{}.{}.{}", version.major, version.minor, version.build)
}

/// Écrit une liste de textes comme Python l'affiche : `['AP', 'TextOnly']`.
///
/// Le serveur officiel met cette forme dans ses messages « a rejoint ».
fn python_list_repr(items: &[String]) -> String {
    let parts: Vec<String> = items.iter().map(|s| python_str_repr(s)).collect();
    format!("[{}]", parts.join(", "))
}

/// `repr()` d'un texte en Python : guillemets simples, sauf si le texte en
/// contient et pas de doubles. Caractères de contrôle échappés.
fn python_str_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_comme_python() {
        assert_eq!(python_list_repr(&[]), "[]");
        let tags = vec!["AP".to_string(), "TextOnly".to_string()];
        assert_eq!(python_list_repr(&tags), "['AP', 'TextOnly']");
        assert_eq!(python_str_repr("l'eau"), "\"l'eau\"");
        assert_eq!(python_str_repr("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(python_str_repr("a\\b\n"), "'a\\\\b\\n'");
    }

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
