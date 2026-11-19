//! Serveur de parties multimondes de Raftup, compatible avec le protocole
//! réseau d'Archipelago (ADR 0004).
//!
//! Organisation (ADR 0006) : une tâche « salle » possède l'état de la partie ;
//! chaque connexion a sa tâche, qui ne fait que lire et écrire sur le réseau.
//!
//! Avancement (jalon J1) : accueil (`RoomInfo`), connexion des joueurs
//! (`Connect`), noms des objets et des lieux (`GetDataPackage`), partie
//! complète, limites des connexions (ADR 0007), chiffrement `wss` (ADR 0009),
//! suivi depuis le launcher (ADR 0010).

mod command_line;
mod connection;
mod datastore;
pub mod error;
mod follow;
mod fuzzy;
mod game;
mod hints;
pub mod limits;
pub mod monitor;
mod pyset;
mod python;
pub mod room;
mod room_task;
pub mod save;
pub mod tls;

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::{TcpListener, ToSocketAddrs};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

pub use error::{ConnectionError, ServerError, TlsError};
pub use limits::Limits;
pub use monitor::{GameEvent, ServerStatus, SlotStatus};
pub use room::Room;
pub use tls::TlsConfig;

use multidata::raftup::{SaveFile, Snapshot};
use room_task::{Event, SaveState};

/// Port par défaut du serveur officiel.
pub const DEFAULT_PORT: u16 = 38281;

/// Un serveur à l'écoute, prêt à accepter des clients.
pub struct Server {
    listener: TcpListener,
    room: Room,
    limits: Limits,
    /// `None` : en clair (`ws`), seulement sur demande explicite (NF06).
    tls: Option<TlsConfig>,
    /// `None` : pas de sauvegarde (`--disable_save`).
    save: Option<SaveState>,
}

impl Server {
    /// Ouvre le port d'écoute. Avec le port `0`, le système en choisit un libre
    /// (pratique pour les tests) : voir [`Server::local_addr`].
    pub async fn bind(address: impl ToSocketAddrs, room: Room) -> Result<Server, ServerError> {
        let listener = TcpListener::bind(address).await?;
        Ok(Server {
            listener,
            room,
            limits: Limits::default(),
            tls: None,
            save: None,
        })
    }

    /// Remplace les limites par défaut (celles de la spec) ; utile aux tests.
    pub fn with_limits(mut self, limits: Limits) -> Server {
        self.limits = limits;
        self
    }

    /// Sert les clients en `wss` avec ce certificat (ADR 0009).
    pub fn with_tls(mut self, tls: TlsConfig) -> Server {
        self.tls = Some(tls);
        self
    }

    /// Sauvegarde la partie dans ce fichier `.raftup` (ADR 0014). Le fichier
    /// est déjà ouvert (`SaveFile::open`) ou créé (`SaveFile::create`) ; la
    /// partie reprend à l'état `snapshot` qu'il contient.
    pub fn with_save(mut self, file: SaveFile, snapshot: Snapshot) -> Server {
        self.save = Some(SaveState { file, snapshot });
        self
    }

    /// Adresse réellement écoutée.
    pub fn local_addr(&self) -> Result<SocketAddr, ServerError> {
        Ok(self.listener.local_addr()?)
    }

    /// Lance la tâche salle, puis accepte les clients sans fin.
    pub async fn run(self) -> Result<(), ServerError> {
        let running = self.start()?;
        // La boucle d'accueil ne s'arrête jamais d'elle-même.
        running
            .accept
            .await
            .map_err(|e| ServerError::Task(e.to_string()))
    }

    /// Lance le serveur en arrière-plan et rend la main tout de suite.
    ///
    /// Le [`RunningServer`] renvoyé sert à suivre la partie et à l'arrêter
    /// (ADR 0010). Il faut être dans un runtime tokio.
    pub fn start(self) -> Result<RunningServer, ServerError> {
        let address = self.listener.local_addr()?;
        // Canal unique vers la tâche salle : chaque connexion en garde une copie
        // de l'émetteur (`clone()`), la salle garde le récepteur. Il est borné :
        // quand il est plein, une connexion attend avant de lire la suite.
        let (events, inbox) = mpsc::channel(self.limits.room_queue.max(1));
        let (monitor, _) = broadcast::channel(monitor::EVENT_CAPACITY);
        // La salle tourne sur un thread système à elle (ADR 0014) : ses
        // écritures sur le disque (fsync) attendent sans bloquer les
        // connexions, qui restent sur le runtime tokio de l'appelant.
        // Elle a son propre petit runtime (un seul fil) pour ses minuteurs.
        let (done, room) = oneshot::channel();
        let room_monitor = monitor.clone();
        let (room_data, limits, save) = (self.room, self.limits, self.save);
        std::thread::Builder::new()
            .name("raftup-salle".to_string())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(room_task::run(
                        room_data,
                        limits,
                        inbox,
                        room_monitor,
                        save,
                    )),
                    Err(error) => error!("runtime de la salle impossible : {error}"),
                }
                // Prévient `stop` ; personne n'attend peut-être : on ignore l'échec.
                let _ = done.send(());
            })?;
        let accept = tokio::spawn(accept_loop(
            self.listener,
            events.clone(),
            self.limits,
            self.tls,
        ));
        Ok(RunningServer {
            address,
            events,
            monitor,
            accept,
            room,
        })
    }
}

/// Accueille les clients sans fin, une tâche par connexion.
async fn accept_loop(
    listener: TcpListener,
    events: mpsc::Sender<Event>,
    limits: Limits,
    tls: Option<TlsConfig>,
) {
    let mut next_id = 0;
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                next_id += 1;
                // `tokio::spawn` lance la connexion en parallèle des autres.
                tokio::spawn(connection::handle(
                    stream,
                    peer,
                    next_id,
                    events.clone(),
                    limits,
                    tls.as_ref().map(TlsConfig::acceptor),
                ));
            }
            Err(error) => {
                // Souvent passager (trop de fichiers ouverts…) : on attend un
                // peu au lieu de boucler à pleine vitesse.
                warn!("connexion refusée : {error}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// Un serveur lancé par [`Server::start`] : on peut le suivre et l'arrêter.
pub struct RunningServer {
    address: SocketAddr,
    /// Copie de l'entrée de la tâche salle, pour lui demander une photo.
    events: mpsc::Sender<Event>,
    monitor: broadcast::Sender<GameEvent>,
    accept: JoinHandle<()>,
    /// Répond quand le thread de la salle a fini.
    room: oneshot::Receiver<()>,
}

impl RunningServer {
    /// Adresse réellement écoutée.
    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }

    /// Photo de l'état de la partie ; `None` si le serveur est arrêté.
    pub async fn status(&self) -> Option<ServerStatus> {
        // `oneshot` : un canal qui porte une seule réponse.
        let (reply, answer) = oneshot::channel();
        self.events.send(Event::Status { reply }).await.ok()?;
        answer.await.ok()
    }

    /// Envoie un message de l'hôte à tous les joueurs connectés.
    pub async fn announce(&self, text: String) -> Result<(), ServerError> {
        self.events
            .send(Event::Announce { text })
            .await
            .map_err(|_| ServerError::Task("la salle est arrêtée".to_string()))
    }

    /// Abonnement au fil d'événements : ce qui se passe à partir de maintenant.
    pub fn subscribe(&self) -> broadcast::Receiver<GameEvent> {
        self.monitor.subscribe()
    }

    /// Arrête le serveur : plus d'accueil, et toutes les connexions se ferment.
    ///
    /// Le port est libéré au retour de la fonction.
    pub async fn stop(self) {
        self.accept.abort();
        // La salle s'arrête à la réception de `Stop` ; si elle est déjà
        // arrêtée, l'envoi échoue, ce qui ne change rien.
        let _ = self.events.send(Event::Stop).await;
        // On attend la fin réelle des deux : l'écoute du port est alors
        // fermée. Une tâche annulée répond par une erreur « cancelled ».
        let _ = self.accept.await;
        let _ = self.room.await;
        info!(address = %self.address, "serveur arrêté");
    }
}
