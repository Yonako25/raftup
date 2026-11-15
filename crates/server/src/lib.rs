//! Serveur de parties multimondes de Raftup, compatible avec le protocole
//! réseau d'Archipelago (ADR 0004).
//!
//! Étape 3 du jalon J1 : le socle. Le serveur charge une seed, accepte les
//! connexions WebSocket en clair et envoie `RoomInfo` à chaque client.
//! La connexion des joueurs (`Connect`) arrive à l'étape 4.

mod connection;
pub mod error;
pub mod room;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, ToSocketAddrs};
use tracing::warn;

pub use error::{ConnectionError, ServerError};
pub use room::Room;

/// Port par défaut du serveur officiel.
pub const DEFAULT_PORT: u16 = 38281;

/// Un serveur à l'écoute, prêt à accepter des clients.
pub struct Server {
    listener: TcpListener,
    /// `Arc` (compteur de références partagé entre tâches) : chaque connexion
    /// tourne dans sa propre tâche tokio et a besoin de lire la salle.
    room: Arc<Room>,
}

impl Server {
    /// Ouvre le port d'écoute. Avec le port `0`, le système en choisit un libre
    /// (pratique pour les tests) : voir [`Server::local_addr`].
    pub async fn bind(address: impl ToSocketAddrs, room: Room) -> Result<Server, ServerError> {
        let listener = TcpListener::bind(address).await?;
        Ok(Server {
            listener,
            room: Arc::new(room),
        })
    }

    /// Adresse réellement écoutée.
    pub fn local_addr(&self) -> Result<SocketAddr, ServerError> {
        Ok(self.listener.local_addr()?)
    }

    /// Accepte les clients sans fin. Chaque client est servi dans sa propre tâche.
    pub async fn run(self) -> Result<(), ServerError> {
        loop {
            match self.listener.accept().await {
                Ok((stream, peer)) => {
                    // `tokio::spawn` lance la tâche en parallèle ; elle reçoit sa
                    // propre copie de l'`Arc` (même salle, compteur + 1).
                    tokio::spawn(connection::handle(stream, peer, Arc::clone(&self.room)));
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
