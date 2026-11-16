//! Serveur de parties multimondes de Raftup, compatible avec le protocole
//! réseau d'Archipelago (ADR 0004).
//!
//! Organisation (ADR 0006) : une tâche « salle » possède l'état de la partie ;
//! chaque connexion a sa tâche, qui ne fait que lire et écrire sur le réseau.
//!
//! Avancement (jalon J1) : accueil (`RoomInfo`), connexion des joueurs
//! (`Connect`), noms des objets et des lieux (`GetDataPackage`), partie
//! complète, limites des connexions (ADR 0007), chiffrement `wss` (ADR 0009).

mod command_line;
mod connection;
mod datastore;
pub mod error;
mod fuzzy;
mod game;
pub mod limits;
mod python;
pub mod room;
mod room_task;
pub mod tls;

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::{TcpListener, ToSocketAddrs};
use tokio::sync::mpsc;
use tracing::warn;

pub use error::{ConnectionError, ServerError, TlsError};
pub use limits::Limits;
pub use room::Room;
pub use tls::TlsConfig;

/// Port par défaut du serveur officiel.
pub const DEFAULT_PORT: u16 = 38281;

/// Un serveur à l'écoute, prêt à accepter des clients.
pub struct Server {
    listener: TcpListener,
    room: Room,
    limits: Limits,
    /// `None` : en clair (`ws`), seulement sur demande explicite (NF06).
    tls: Option<TlsConfig>,
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

    /// Adresse réellement écoutée.
    pub fn local_addr(&self) -> Result<SocketAddr, ServerError> {
        Ok(self.listener.local_addr()?)
    }

    /// Lance la tâche salle, puis accepte les clients sans fin.
    pub async fn run(self) -> Result<(), ServerError> {
        // Canal unique vers la tâche salle : chaque connexion en garde une copie
        // de l'émetteur (`clone()`), la salle garde le récepteur. Il est borné :
        // quand il est plein, une connexion attend avant de lire la suite.
        let (events, inbox) = mpsc::channel(self.limits.room_queue.max(1));
        tokio::spawn(room_task::run(self.room, self.limits, inbox));

        let mut next_id = 0;
        loop {
            match self.listener.accept().await {
                Ok((stream, peer)) => {
                    next_id += 1;
                    // `tokio::spawn` lance la connexion en parallèle des autres.
                    tokio::spawn(connection::handle(
                        stream,
                        peer,
                        next_id,
                        events.clone(),
                        self.limits,
                        self.tls.as_ref().map(TlsConfig::acceptor),
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
}
