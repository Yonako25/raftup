//! Serveur de parties multimondes de Raftup, compatible avec le protocole
//! réseau d'Archipelago (ADR 0004).
//!
//! Organisation (ADR 0006) : une tâche « salle » possède l'état de la partie ;
//! chaque connexion a sa tâche, qui ne fait que lire et écrire sur le réseau.
//!
//! Avancement (jalon J1) : accueil (`RoomInfo`), connexion des joueurs
//! (`Connect`), noms des objets et des lieux (`GetDataPackage`). En clair (`ws`)
//! seulement pour l'instant.

mod connection;
pub mod error;
mod game;
pub mod room;
mod room_task;

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::{TcpListener, ToSocketAddrs};
use tokio::sync::mpsc::unbounded_channel;
use tracing::warn;

pub use error::{ConnectionError, ServerError};
pub use room::Room;

/// Port par défaut du serveur officiel.
pub const DEFAULT_PORT: u16 = 38281;

/// Un serveur à l'écoute, prêt à accepter des clients.
pub struct Server {
    listener: TcpListener,
    room: Room,
}

impl Server {
    /// Ouvre le port d'écoute. Avec le port `0`, le système en choisit un libre
    /// (pratique pour les tests) : voir [`Server::local_addr`].
    pub async fn bind(address: impl ToSocketAddrs, room: Room) -> Result<Server, ServerError> {
        let listener = TcpListener::bind(address).await?;
        Ok(Server { listener, room })
    }

    /// Adresse réellement écoutée.
    pub fn local_addr(&self) -> Result<SocketAddr, ServerError> {
        Ok(self.listener.local_addr()?)
    }

    /// Lance la tâche salle, puis accepte les clients sans fin.
    pub async fn run(self) -> Result<(), ServerError> {
        // Canal unique vers la tâche salle : chaque connexion en garde une copie
        // de l'émetteur (`clone()`), la salle garde le récepteur.
        let (events, inbox) = unbounded_channel();
        tokio::spawn(room_task::run(self.room, inbox));

        let mut next_id = 0;
        loop {
            match self.listener.accept().await {
                Ok((stream, peer)) => {
                    next_id += 1;
                    // `tokio::spawn` lance la connexion en parallèle des autres.
                    tokio::spawn(connection::handle(stream, peer, next_id, events.clone()));
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
