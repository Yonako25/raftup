//! La tâche salle : elle possède tout l'état de la partie (ADR 0006).
//!
//! Elle reçoit des événements des connexions (arrivée, paquets, départ) et les
//! traite un par un, dans l'ordre, comme la boucle unique du serveur officiel.
//! Les réponses partent par le canal de chaque client.
//!
//! Gérés : `Connect`, `GetDataPackage`, `ConnectUpdate`, `Sync`, `LocationChecks`,
//! `StatusUpdate` (avec `collect` et `release` automatiques à l'objectif),
//! `LocationScouts`, `CreateHints`, `UpdateHint`, `Say` (chat et commandes `!`), `Bounce`,
//! stockage de données (`Get`, `Set`, `SetNotify`, clés `_read_…`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use ap_protocol::client::{
    Bounce, Connect, ConnectUpdate, Get, GetDataPackage, LocationChecks, LocationScouts, Set,
};
use ap_protocol::server::{
    Bounced, Connected, ConnectionRefused, DataPackage, DataPackageData, GameData, InvalidPacket,
    JsonMessagePart, LocationInfo, PrintJson, ReceivedItems, Retrieved, RoomUpdate, SetReply,
};
use ap_protocol::{
    encode_server_frame, ClientPacket, ClientStatus, Hint, HintStatus, ItemFlags, LocationId,
    NetworkItem, NetworkPlayer, NetworkVersion, ServerPacket, SlotId, SlotType, TeamId,
};
use indexmap::IndexMap;
use serde_json::{Map, Value};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::{broadcast, oneshot, Notify};
use tracing::{debug, error, info, warn};

use crate::connection::Incoming;
use crate::datastore::{self, DataStore};
use crate::game::{locations_for_player, GameState, Sent};
use crate::limits::{ChatRate, Limits};
use crate::monitor::{GameEvent, Names, ServerStatus, SlotStatus};
use crate::python;
use crate::room::{now, Room, SERVER_VERSION};

// Le gros `impl RoomTask` est réparti entre les sous-modules : en Rust, un
// `impl` peut être découpé en plusieurs blocs, même dans plusieurs fichiers
// d'un même module. Les champs privés de `RoomTask` restent visibles des
// modules enfants, d'où le `pub(super)` sur les méthodes partagées.
mod commands;
mod connect;
mod exchange;
mod hints;
mod messages;
mod monitoring;
mod play;
mod storage;

use connect::ItemsChoice;
use messages::*;
use storage::*;
/// Numéro unique d'une connexion, donné à son arrivée.
pub(crate) type ClientId = u64;

/// Ce qu'une connexion signale à la tâche salle.
#[derive(Debug)]
pub(crate) enum Event {
    /// Nouveau client ; `outbox` sert à lui envoyer des trames, `kill` à le
    /// fermer tout de suite quand `outbox` est plein (ADR 0007).
    Opened {
        id: ClientId,
        outbox: Sender<Outgoing>,
        kill: Arc<Notify>,
    },
    /// Une trame reçue, déjà découpée en paquets.
    Frame { id: ClientId, items: Vec<Incoming> },
    /// Le client est parti (ou sa connexion a échoué).
    Closed { id: ClientId },
    /// Demande de photo de l'état, pour le suivi (ADR 0010).
    Status {
        reply: oneshot::Sender<ServerStatus>,
    },
    /// Message de l'hôte à tous les joueurs.
    Announce { text: String },
}

/// Ce que la tâche salle demande à une connexion.
#[derive(Debug)]
pub(crate) enum Outgoing {
    /// Trame JSON à envoyer telle quelle.
    Frame(String),
    /// Fermer la connexion.
    Close,
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
    /// File bornée vers la connexion.
    outbox: Sender<Outgoing>,
    /// Ferme la connexion sans passer par la file (quand elle est pleine).
    kill: Arc<Notify>,
    /// `None` tant que le client n'a pas réussi son `Connect`.
    player: Option<Player>,
    /// Vrai quand la salle a demandé la fermeture : on ignore la suite.
    closing: bool,
    /// Messages de chat récents (NF04).
    chat: ChatRate,
}

impl Client {
    /// Dépose un message dans la file du client, sans jamais attendre.
    ///
    /// File pleine : le client lit trop lentement, on le ferme par `kill`.
    /// File fermée : la connexion est finie, son événement `Closed` va arriver.
    fn push(&self, message: Outgoing) {
        if let Err(TrySendError::Full(_)) = self.outbox.try_send(message) {
            // `notify_one` garde le signal même si la connexion n'attend pas encore.
            self.kill.notify_one();
        }
    }
}

/// Fait tourner la tâche salle jusqu'à ce que plus personne ne puisse lui écrire.
///
/// `monitor` : le fil d'événements du suivi (ADR 0010) ; envoyer dedans ne
/// bloque jamais, même sans abonné.
pub(crate) async fn run(
    room: Room,
    limits: Limits,
    mut events: Receiver<Event>,
    monitor: broadcast::Sender<GameEvent>,
) {
    let game = GameState::new(room.data());
    let names = Names::new(room.data());
    let mut task = RoomTask {
        room,
        limits,
        game,
        clients: BTreeMap::new(),
        store: DataStore::default(),
        subscribers: BTreeMap::new(),
        names,
        monitor,
        activity: BTreeMap::new(),
    };
    while let Some(event) = events.recv().await {
        task.handle(event);
    }
}

struct RoomTask {
    room: Room,
    limits: Limits,
    game: GameState,
    /// `BTreeMap` : les diffusions suivent l'ordre d'arrivée des clients.
    clients: BTreeMap<ClientId, Client>,
    /// Valeurs rangées par les clients (`Set`).
    store: DataStore,
    /// Pour chaque clé, les clients à prévenir quand elle change (`SetNotify`).
    subscribers: BTreeMap<String, BTreeSet<ClientId>>,
    /// Noms des objets et des lieux, pour les événements du suivi.
    names: Names,
    /// Fil d'événements du suivi.
    monitor: broadcast::Sender<GameEvent>,
    /// Heure du dernier paquet reçu de chaque slot.
    activity: BTreeMap<(TeamId, SlotId), f64>,
}

impl RoomTask {
    fn handle(&mut self, event: Event) {
        match event {
            Event::Opened { id, outbox, kill } => {
                let client = Client {
                    outbox,
                    kill,
                    player: None,
                    closing: false,
                    chat: ChatRate::default(),
                };
                self.clients.insert(id, client);
                let info = self.room.room_info(now());
                self.send(id, vec![ServerPacket::RoomInfo(info)]);
            }
            Event::Frame { id, items } => self.on_frame(id, items),
            Event::Closed { id } => self.on_closed(id),
            Event::Status { reply } => {
                // Le demandeur a pu abandonner : on ignore l'échec.
                let _ = reply.send(self.status());
            }
            Event::Announce { text } => self.announce(text),
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
                Incoming::BadArguments { cmd, error, raw } => {
                    debug!(id, %cmd, "paquet aux champs incorrects : {error}");
                    self.on_bad_arguments(id, &cmd, raw.as_ref());
                }
                // Mêmes textes que le serveur officiel (`process_client_cmd`).
                Incoming::CmdNotText(cmd) => {
                    let text = format!(
                        "Command should be str, got <class '{}'>",
                        python::type_name(&cmd)
                    );
                    self.send(id, vec![invalid_cmd(&text)]);
                }
                Incoming::NoCmd(packet) => {
                    // `f"{args}"` en Python : le `str()` du paquet.
                    let text = format!(
                        "Could not get command from {} at `cmd`",
                        python::str_of(&packet)
                    );
                    self.send(id, vec![invalid_cmd(&text)]);
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
        let Some(player) = self.player(id) else {
            debug!(id, cmd = packet.cmd(), "paquet avant connexion, ignoré");
            return;
        };
        self.activity.insert((player.team, player.slot), now());
        match packet {
            ClientPacket::ConnectUpdate(update) => self.on_connect_update(id, update),
            ClientPacket::Sync => self.on_sync(id),
            ClientPacket::LocationChecks(checks) => self.on_location_checks(id, &checks),
            ClientPacket::StatusUpdate(update) => self.on_status_update(id, update.status),
            ClientPacket::LocationScouts(scouts) => self.on_location_scouts(id, &scouts),
            ClientPacket::CreateHints(request) => self.on_create_hints(id, request),
            ClientPacket::UpdateHint(request) => self.on_update_hint(id, request),
            ClientPacket::Say(say) => self.on_say(id, &say.text),
            ClientPacket::Bounce(bounce) => self.on_bounce(id, bounce),
            ClientPacket::Get(get) => self.on_get(id, get),
            ClientPacket::Set(set) => self.on_set(id, set),
            ClientPacket::SetNotify(notify) => {
                for key in notify.keys {
                    self.subscribers.entry(key).or_default().insert(id);
                }
            }
            other => debug!(id, cmd = other.cmd(), "paquet pas encore géré, ignoré"),
        }
    }

    fn player(&self, id: ClientId) -> Option<&Player> {
        self.clients.get(&id).and_then(|c| c.player.as_ref())
    }

    fn set_player(&mut self, id: ClientId, player: Player) {
        if let Some(client) = self.clients.get_mut(&id) {
            client.player = Some(player);
        }
    }

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
            client.push(Outgoing::Frame(frame));
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
                client.push(Outgoing::Frame(frame.clone()));
            }
        }
    }

    /// Envoie la même trame à une liste de clients.
    fn send_to(&self, ids: &BTreeSet<ClientId>, packets: &[ServerPacket]) {
        if ids.is_empty() {
            return;
        }
        let Some(frame) = encode(packets) else {
            return;
        };
        for id in ids {
            if let Some(client) = self.clients.get(id) {
                if !client.closing {
                    client.push(Outgoing::Frame(frame.clone()));
                }
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
            client.push(Outgoing::Close);
        }
    }
}
