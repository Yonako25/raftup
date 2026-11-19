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

use std::cell::RefCell;
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
use crate::follow::FollowState;
use crate::game::{locations_for_player, GameState, Sent};
use crate::limits::{ChatRate, Limits};
use crate::monitor::{GameEvent, Names, ServerStatus, SlotStatus};
use crate::python;
use crate::room::{now, Room, SERVER_VERSION};
use multidata::raftup::{diff, SaveFile, SlotActivity, Snapshot, StoreEntry};

// Le gros `impl RoomTask` est réparti entre les sous-modules : en Rust, un
// `impl` peut être découpé en plusieurs blocs, même dans plusieurs fichiers
// d'un même module. Les champs privés de `RoomTask` restent visibles des
// modules enfants, d'où le `pub(super)` sur les méthodes partagées.
mod commands;
mod connect;
mod exchange;
mod follow;
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
    /// Arrêt de la salle (`RunningServer::stop`).
    Stop,
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
    /// Messages en attente : ils ne partent qu'après l'écriture de la
    /// sauvegarde sur le disque (NF07, ADR 0014), dans `flush`.
    ///
    /// `RefCell` : les fonctions d'envoi ne reçoivent qu'une référence
    /// partagée (`&self`) ; `RefCell` permet quand même d'ajouter à la file.
    /// Il vérifie à l'exécution qu'un seul emprunt modifiable existe à la fois
    /// (la tâche salle n'a qu'un fil d'exécution : ça ne peut pas échouer ici).
    pending: RefCell<Vec<Outgoing>>,
}

impl Client {
    fn new(outbox: Sender<Outgoing>, kill: Arc<Notify>) -> Client {
        Client {
            outbox,
            kill,
            player: None,
            closing: false,
            chat: ChatRate::default(),
            pending: RefCell::new(Vec::new()),
        }
    }

    /// Met un message en attente ; il part au prochain `flush`.
    fn push(&self, message: Outgoing) {
        self.pending.borrow_mut().push(message);
    }

    /// Dépose les messages en attente dans la file du client, sans attendre.
    ///
    /// File pleine : le client lit trop lentement, on le ferme par `kill`.
    /// File fermée : la connexion est finie, son événement `Closed` va arriver.
    fn flush(&self) {
        // `take` vide la file et rend son contenu, en un seul emprunt.
        let messages = self.pending.take();
        for message in messages {
            if let Err(TrySendError::Full(_)) = self.outbox.try_send(message) {
                // `notify_one` garde le signal même si la connexion n'attend pas encore.
                self.kill.notify_one();
                return;
            }
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
    save: Option<SaveState>,
) {
    let mut task = RoomTask::new(room, limits, monitor, save);
    // Écrit tout de suite ce que l'état de départ ajoute à un fichier neuf
    // (statuts des groupes, indices de départ), puis compacte (ADR 0014 :
    // compactage au démarrage).
    if !task.commit() || !task.compact() {
        return;
    }
    loop {
        // Sans compte à rebours, on attend seulement le prochain événement.
        // Avec, `select!` attend le premier des deux : un événement, ou
        // l'heure de la prochaine annonce (la tâche ne bloque jamais).
        let event = match task.countdown_next {
            None => events.recv().await,
            Some(next) => tokio::select! {
                event = events.recv() => event,
                _ = tokio::time::sleep_until(next) => {
                    task.countdown_step();
                    if !task.commit() {
                        return;
                    }
                    continue;
                }
            },
        };
        match event {
            Some(Event::Stop) | None => break,
            Some(event) => task.handle(event),
        }
        if !task.commit() {
            return;
        }
    }
    // Arrêt propre : les joueurs encore connectés sont notés « vus » (pour
    // leur résumé de retour, ADR 0031), puis un dernier compactage. En cas
    // d'échec, le fichier garde son instantané et ses deltas, toujours
    // lisibles : rien n'est perdu.
    if task.mark_connected_seen() && !task.commit() {
        return;
    }
    task.compact();
}

/// Vrai si l'événement peut changer l'état sauvegardé (lieux, objets,
/// statuts, indices, surnoms, stockage…). En cas de doute : vrai, c'est
/// seulement une photo de trop.
fn may_change_state(event: &Event) -> bool {
    match event {
        // Lecture seule : accueil, photo pour le suivi, annonce de l'hôte.
        Event::Opened { .. } | Event::Status { .. } | Event::Announce { .. } | Event::Stop => false,
        // Le statut du slot peut repasser à « inconnu ».
        Event::Closed { .. } => true,
        Event::Frame { items, .. } => items.iter().any(|item| match item {
            Incoming::Packet(packet) => !matches!(
                **packet,
                ClientPacket::Bounce(_)
                    | ClientPacket::Get(_)
                    | ClientPacket::SetNotify(_)
                    | ClientPacket::Sync
                    | ClientPacket::GetDataPackage(_)
            ),
            // Lu champ par champ (`CreateHints`…) : peut créer des indices.
            Incoming::BadArguments { .. } => true,
            // Refusés sans rien changer.
            Incoming::CmdNotText(_) | Incoming::NoCmd(_) => false,
        }),
    }
}

/// Au-delà de ce nombre de deltas, le fichier est réécrit en un seul
/// instantané (ADR 0014 : « au-delà d'environ 10 000 lignes »).
const COMPACT_AFTER_LINES: usize = 10_000;

impl RoomTask {
    /// Une salle prête à tourner : état de départ de la seed, ou état repris
    /// d'une sauvegarde.
    fn new(
        room: Room,
        limits: Limits,
        monitor: broadcast::Sender<GameEvent>,
        save: Option<SaveState>,
    ) -> RoomTask {
        // Reprise : l'état vient de la sauvegarde ; sinon, l'état de départ.
        let (game, store, activity, follow, save) = match save {
            Some(SaveState { file, snapshot }) => (
                GameState::from_snapshot(room.data(), &snapshot),
                DataStore::from_entries(
                    snapshot
                        .store
                        .iter()
                        .map(|e| (e.key.clone(), e.value.clone())),
                ),
                snapshot
                    .activity
                    .iter()
                    .map(|a| ((a.team, a.slot), a.time))
                    .collect(),
                FollowState::from_snapshot(&snapshot),
                Some(Saver {
                    file,
                    last: snapshot,
                }),
            ),
            None => (
                GameState::new(room.data()),
                DataStore::default(),
                BTreeMap::new(),
                FollowState::default(),
                None,
            ),
        };
        let names = Names::new(room.data());
        RoomTask {
            room,
            limits,
            game,
            clients: BTreeMap::new(),
            store,
            subscribers: BTreeMap::new(),
            names,
            monitor,
            activity,
            follow,
            countdown_timer: 0,
            countdown_next: None,
            admin_client: None,
            allow_releases: BTreeSet::new(),
            save,
            // Vrai au départ : l'état de départ doit être écrit dans un fichier neuf.
            dirty: true,
            pending_events: RefCell::new(Vec::new()),
        }
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
    /// Suivi asynchrone (ADR 0031) : absences, départs, votes, fil d'activité.
    /// Toujours chargé et sauvegardé ; il ne change que si `room.async_follow()`.
    follow: FollowState,
    /// Compte à rebours en cours (`countdown_timer` du serveur officiel) :
    /// secondes restantes à annoncer.
    countdown_timer: i64,
    /// Heure de la prochaine annonce ; `None` si aucun compte à rebours ne tourne.
    countdown_next: Option<tokio::time::Instant>,
    /// Client connecté en administrateur avec `!admin login` (un seul à la fois).
    admin_client: Option<ClientId>,
    /// Joueurs autorisés à faire `!release` quel que soit le mode (`/allow_release`).
    /// Pas sauvegardé, comme `allow_releases` du serveur officiel.
    allow_releases: BTreeSet<(TeamId, SlotId)>,
    /// Sauvegarde `.raftup` ; `None` avec `--disable_save`.
    save: Option<Saver>,
    /// Vrai si un événement a pu changer l'état sauvegardé depuis le dernier
    /// `commit` : sinon, `commit` saute la photo et la comparaison.
    dirty: bool,
    /// Événements du suivi en attente : envoyés au launcher dans `commit`,
    /// après l'écriture sur le disque, comme les messages aux clients.
    /// `RefCell` : `emit` ne reçoit qu'une référence partagée (`&self`).
    pending_events: RefCell<Vec<GameEvent>>,
}

/// Le fichier de sauvegarde et l'état qu'il contient déjà.
struct Saver {
    file: SaveFile,
    /// Dernier instantané écrit (instantané + deltas du fichier) : on écrit
    /// seulement la différence avec lui (`raftup::diff`).
    last: Snapshot,
}

/// Une sauvegarde à reprendre : le fichier ouvert et ce qu'il contenait
/// (`Snapshot::default()` pour un fichier neuf).
pub struct SaveState {
    pub file: SaveFile,
    pub snapshot: Snapshot,
}

impl RoomTask {
    /// Instantané de toute la partie, pour la sauvegarde.
    fn snapshot(&self) -> Snapshot {
        let mut snapshot = Snapshot::default();
        self.game.fill_snapshot(&mut snapshot);
        snapshot.store = self
            .store
            .entries()
            .map(|(key, value)| StoreEntry {
                key: key.clone(),
                value: value.clone(),
            })
            .collect();
        snapshot.activity = self
            .activity
            .iter()
            .map(|((team, slot), time)| SlotActivity {
                team: *team,
                slot: *slot,
                time: *time,
            })
            .collect();
        self.follow.fill_snapshot(&mut snapshot);
        snapshot
    }

    /// Fin d'un événement : écrit les changements sur le disque (fsync), PUIS
    /// envoie les messages en attente (NF07, ADR 0014).
    ///
    /// Renvoie `false` si l'écriture a échoué : la salle doit s'arrêter sans
    /// rien envoyer, car un client ne doit jamais recevoir de réponse pour un
    /// changement qui n'est pas sur le disque.
    fn commit(&mut self) -> bool {
        if self.save.is_some() && self.dirty {
            let now = self.snapshot();
            // `as_mut` : on modifie le `Saver` sur place, sans le sortir de l'`Option`.
            if let Some(saver) = self.save.as_mut() {
                let records = diff(&saver.last, &now);
                if let Err(error) = saver.file.append(&records) {
                    error!("écriture de la sauvegarde impossible, arrêt de la salle : {error}");
                    return false;
                }
                saver.last = now;
            }
        }
        let too_long = self
            .save
            .as_ref()
            .is_some_and(|s| s.file.lines_since_compact() > COMPACT_AFTER_LINES);
        if too_long && !self.compact() {
            return false;
        }
        self.dirty = false;
        for client in self.clients.values() {
            client.flush();
        }
        // `take` vide la file et rend son contenu. Envoyer dans le fil ne
        // bloque jamais, même sans abonné.
        for event in self.pending_events.take() {
            let _ = self.monitor.send(event);
        }
        true
    }

    /// Réécrit le fichier en un seul instantané (le dernier écrit). Atomique :
    /// `SaveFile::compact` passe par un `.tmp` puis un renommage.
    /// Renvoie `false` si l'écriture a échoué ; sans sauvegarde, `true`.
    fn compact(&mut self) -> bool {
        let Some(saver) = self.save.as_mut() else {
            return true;
        };
        match saver.file.compact(&saver.last) {
            Ok(()) => true,
            Err(error) => {
                error!("compactage de la sauvegarde impossible : {error}");
                false
            }
        }
    }

    fn handle(&mut self, event: Event) {
        if may_change_state(&event) {
            self.dirty = true;
        }
        match event {
            Event::Opened { id, outbox, kill } => {
                self.clients.insert(id, Client::new(outbox, kill));
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
            // Traité dans la boucle de `run`, avant d'arriver ici.
            Event::Stop => {}
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

    /// Nom affiché dans les messages : « Surnom (Nom) » si le slot a un
    /// surnom, sinon le nom (`get_aliased_name` du serveur officiel).
    fn aliased_name(&self, team: TeamId, slot: SlotId) -> String {
        let name = self.slot_name(slot);
        match self.game.alias(team, slot) {
            Some(alias) => format!("{alias} ({name})"),
            None => name,
        }
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
        // Comme `broadcast` et `send_to` : rien après la demande de fermeture.
        if client.closing {
            return;
        }
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
