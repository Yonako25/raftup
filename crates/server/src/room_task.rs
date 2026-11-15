//! La tâche salle : elle possède tout l'état de la partie (ADR 0006).
//!
//! Elle reçoit des événements des connexions (arrivée, paquets, départ) et les
//! traite un par un, dans l'ordre, comme la boucle unique du serveur officiel.
//! Les réponses partent par le canal de chaque client.
//!
//! Étape 4 de J1 : `Connect`, `Connected`, inventaire de départ, `GetDataPackage`,
//! messages « a rejoint » et « est parti ». Les paquets de jeu viennent à l'étape 5.

use std::collections::{BTreeMap, BTreeSet};

use ap_protocol::client::{Connect, GetDataPackage};
use ap_protocol::server::{
    Connected, ConnectionRefused, DataPackage, DataPackageData, GameData, InvalidPacket,
    JsonMessagePart, PrintJson, ReceivedItems,
};
use ap_protocol::{
    encode_server_frame, ClientPacket, ItemFlags, ItemsHandling, NetworkItem, NetworkPlayer,
    NetworkVersion, ServerPacket, SlotId, TeamId,
};
use indexmap::IndexMap;
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{debug, error, info, warn};

use crate::connection::Incoming;
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

/// Un joueur authentifié (après un `Connect` accepté).
#[derive(Debug, Clone)]
struct Player {
    team: TeamId,
    slot: SlotId,
    version: NetworkVersion,
    tags: Vec<String>,
    /// Nombre d'objets déjà envoyés à ce client (servira à l'étape 5).
    #[allow(dead_code)]
    send_index: usize,
    /// Vrai si le client ne veut pas des messages texte (`PrintJSON`).
    no_text: bool,
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
    },
    Refused(Vec<String>),
    /// Cas où le serveur officiel plante et ferme la connexion.
    Fatal(&'static str),
}

/// Fait tourner la tâche salle jusqu'à ce que plus personne ne puisse lui écrire.
pub(crate) async fn run(room: Room, mut events: UnboundedReceiver<Event>) {
    let mut task = RoomTask {
        room,
        clients: BTreeMap::new(),
    };
    while let Some(event) = events.recv().await {
        task.handle(event);
    }
}

struct RoomTask {
    room: Room,
    /// `BTreeMap` : les diffusions suivent l'ordre d'arrivée des clients.
    clients: BTreeMap<ClientId, Client>,
}

impl RoomTask {
    fn handle(&mut self, event: Event) {
        match event {
            Event::Opened { id, outbox } => {
                self.clients.insert(
                    id,
                    Client {
                        outbox,
                        player: None,
                        closing: false,
                    },
                );
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
                        let reply = invalid_packet("arguments", Some("Connect"), "Connect");
                        self.send(id, vec![reply]);
                    }
                    // Les autres seront traités avec les paquets de jeu (étape 5).
                }
                Incoming::CmdNotText => {
                    let reply = invalid_packet("cmd", None, "cmd doit être du texte");
                    self.send(id, vec![reply]);
                }
                Incoming::NoCmd => {
                    let reply = invalid_packet("cmd", None, "paquet sans champ « cmd »");
                    self.send(id, vec![reply]);
                    self.close(id, "paquet sans « cmd »");
                }
            }
        }
    }

    fn on_packet(&mut self, id: ClientId, packet: ClientPacket) {
        match packet {
            ClientPacket::Connect(connect) => self.on_connect(id, &connect),
            ClientPacket::GetDataPackage(request) => self.on_get_data_package(id, &request),
            other => {
                // Non connecté : ignoré, comme sur le serveur officiel.
                // Connecté : paquets de jeu, pas encore gérés (étape 5).
                debug!(id, cmd = other.cmd(), "paquet pas encore géré, ignoré");
            }
        }
    }

    // ----- Connect -------------------------------------------------------

    fn on_connect(&mut self, id: ClientId, connect: &Connect) {
        let (team, slot) = match self.check_connect(connect) {
            ConnectCheck::Accepted { team, slot } => (team, slot),
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
        let previous = self.clients.get(&id).and_then(|c| c.player.clone());
        if let Some(previous) = &previous {
            if (previous.team, previous.slot) != (team, slot) {
                // Le serveur officiel 0.6.8 plante dans ce cas et ferme la connexion.
                self.close(id, "changement de slot sur la même connexion");
                return;
            }
        }
        // Le serveur officiel plante aussi pour un slot sans lieux (un groupe).
        let Some(locations) = self.room.data().locations.get(&slot) else {
            self.close(id, "slot sans lieux (groupe)");
            return;
        };
        // Aucun lieu visité pour l'instant : tous manquent, dans l'ordre croissant.
        let missing_locations: Vec<_> = locations.keys().copied().collect();

        let handling = connect.items_handling.0;
        let wants_items = handling & ItemsHandling::OTHER_WORLDS.0 != 0;
        let wants_start_inventory = handling & ItemsHandling::STARTING_INVENTORY.0 != 0;
        let start_inventory = if wants_start_inventory {
            self.start_inventory(slot)
        } else {
            Vec::new()
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
            checked_locations: Vec::new(),
            slot_data,
            slot_info: data
                .slots
                .iter()
                .map(|(s, info)| (*s, info.clone()))
                .collect(),
            // Points d'indice : aucun lieu visité ni indice acheté pour l'instant.
            hint_points: 0,
        };
        let mut reply = vec![ServerPacket::Connected(connected)];
        let mut send_index = 0;
        if !start_inventory.is_empty() && wants_items {
            send_index = start_inventory.len();
            reply.push(ServerPacket::ReceivedItems(ReceivedItems {
                index: 0,
                items: start_inventory,
            }));
        }

        let has_tag = |tag: &str| connect.tags.iter().any(|t| t == tag);
        let no_text = has_tag("NoText")
            || (has_tag("PopTracker") && version_key(&connect.version) < (0, 5, 1));
        let player = Player {
            team,
            slot,
            version: connect.version,
            tags: connect.tags.clone(),
            send_index,
            no_text,
        };
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
                // Recevoir ses propres objets ou l'inventaire de départ sans
                // recevoir d'objets du tout n'a pas de sens.
                let handling = connect.items_handling.0;
                if handling & 0b001 == 0 && handling & 0b110 != 0 {
                    errors.insert("InvalidItemsHandling");
                }
                accepted = Some((team, slot));
            }
        }
        // Compatibilité 0 : seule la version exacte du serveur est acceptée.
        if data.server_options.compatibility == 0 && connect.version != SERVER_VERSION {
            errors.insert("IncompatibleVersion");
        }

        match accepted {
            Some((team, slot)) if errors.is_empty() => ConnectCheck::Accepted { team, slot },
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

    /// Annonce l'arrivée d'un joueur à tous, puis lui envoie le tutoriel.
    fn on_joined(&mut self, id: ClientId, player: &Player) {
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
        self.broadcast_text(join);

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
        self.broadcast_text(part);
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

    // ----- Envois --------------------------------------------------------

    fn slot_name(&self, slot: SlotId) -> String {
        let slots = &self.room.data().slots;
        slots.get(&slot).map(|s| s.name.clone()).unwrap_or_default()
    }

    fn slot_game(&self, slot: SlotId) -> String {
        let slots = &self.room.data().slots;
        slots.get(&slot).map(|s| s.game.clone()).unwrap_or_default()
    }

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

    /// Message texte pour un seul client connecté (`notify_client`).
    fn notify(&self, id: ClientId, message: PrintJson) {
        let wants_text = self
            .clients
            .get(&id)
            .and_then(|c| c.player.as_ref())
            .is_some_and(|p| !p.no_text);
        if wants_text {
            self.send(id, vec![ServerPacket::PrintJson(message)]);
        }
    }

    /// Message texte pour tous les clients connectés (`broadcast_text_all`).
    fn broadcast_text(&self, message: PrintJson) {
        let Some(frame) = encode(&[ServerPacket::PrintJson(message)]) else {
            return;
        };
        for client in self.clients.values() {
            let wants_text = client.player.as_ref().is_some_and(|p| !p.no_text);
            if wants_text && !client.closing {
                let _ = client.outbox.send(Outgoing::Frame(frame.clone()));
            }
        }
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

fn invalid_packet(kind: &str, original_cmd: Option<&str>, text: &str) -> ServerPacket {
    ServerPacket::InvalidPacket(InvalidPacket {
        kind: kind.to_string(),
        original_cmd: original_cmd.map(String::from),
        text: text.to_string(),
    })
}

/// Verbe affiché pour un outil qui ne joue pas (`viewing`…), selon ses tags.
fn non_game_verb(tags: &[String]) -> Option<&'static str> {
    NON_GAME_TAGS
        .iter()
        .find(|(tag, _)| tags.iter().any(|t| t == tag))
        .map(|(_, verb)| *verb)
}

/// `PrintJSON` d'un seul morceau de texte, sans genre.
fn text_message(text: String) -> PrintJson {
    PrintJson {
        data: vec![JsonMessagePart {
            text: Some(text),
            kind: None,
            color: None,
            flags: None,
            player: None,
            hint_status: None,
        }],
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
}
