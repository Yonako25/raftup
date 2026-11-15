//! Connexion à un serveur, ouverture de la session, envoi et réception de paquets.
//!
//! Déroulé d'une connexion (protocole Archipelago) :
//! 1. ouverture du WebSocket ;
//! 2. le serveur envoie `RoomInfo` ;
//! 3. le client envoie `Connect` ;
//! 4. le serveur répond `Connected` (souvent suivi de `ReceivedItems` et
//!    `PrintJSON` dans la même trame) ou `ConnectionRefused`.
//!
//! Le client n'a pas de tâche de fond : l'appelant appelle [`Client::recv`] en
//! boucle. tungstenite répond tout seul aux « ping » du serveur pendant la lecture.

use std::collections::VecDeque;
use std::time::Duration;

use ap_protocol::client::Connect;
use ap_protocol::server::{Connected, RoomInfo};
use ap_protocol::{
    decode_server_frame, encode_client_frame, ClientPacket, ItemsHandling, NetworkVersion,
    ServerPacket,
};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::address::candidate_urls;
use crate::error::ClientError;

/// Version d'Archipelago annoncée au serveur dans `Connect`.
///
/// Le serveur refuse un client plus ancien que la version demandée par le jeu
/// (`IncompatibleVersion`). C'est la version avec laquelle Raftup est testé.
pub const CLIENT_VERSION: NetworkVersion = NetworkVersion {
    major: 0,
    minor: 6,
    build: 8,
};

/// Taille maximale d'un message reçu par défaut : 64 Mio.
///
/// Le DataPackage d'une grande partie multi-jeux peut peser plusieurs Mio.
/// (La limite de 1 Mio de NF03 concerne ce que le *serveur* accepte.)
pub const DEFAULT_MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

/// Délai maximal pour chaque étape de l'ouverture de session.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Le flux WebSocket, chiffré (`wss`) ou non (`ws`).
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Réglages d'une connexion.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// Adresse saisie par le joueur, par exemple `archipelago.gg:38281`.
    pub address: String,
    /// Nom du slot.
    pub slot: String,
    /// Mot de passe de la salle, s'il y en a un.
    pub password: Option<String>,
    /// Nom du jeu. Vide pour un outil sans jeu (avec le tag `TextOnly` ou `Tracker`).
    pub game: String,
    pub items_handling: ItemsHandling,
    pub tags: Vec<String>,
    /// Identifiant de la machine, envoyé au serveur.
    pub uuid: String,
    /// Vrai pour recevoir les données propres au jeu (`slot_data`).
    pub want_slot_data: bool,
    pub max_message_size: usize,
}

impl ConnectOptions {
    /// Réglages courants : tous les objets, tag `AP`, `slot_data` demandé.
    pub fn new(address: &str, slot: &str, game: &str) -> Self {
        ConnectOptions {
            address: address.to_string(),
            slot: slot.to_string(),
            password: None,
            game: game.to_string(),
            items_handling: ItemsHandling::ALL,
            tags: vec!["AP".to_string()],
            uuid: "raftup".to_string(),
            want_slot_data: true,
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
        }
    }
}

/// Ce que le serveur a dit à l'ouverture de la session.
#[derive(Debug, Clone)]
pub struct Session {
    /// URL réellement utilisée (après un éventuel secours en `ws://`).
    pub url: String,
    pub room: RoomInfo,
    pub connected: Connected,
}

/// Un client connecté à un slot.
pub struct Client {
    socket: Socket,
    /// Paquets déjà lus mais pas encore rendus par `recv` : une trame peut en
    /// contenir plusieurs.
    pending: VecDeque<ServerPacket>,
}

impl Client {
    /// Se connecte au serveur et ouvre la session sur le slot demandé.
    pub async fn connect(options: &ConnectOptions) -> Result<(Client, Session), ClientError> {
        let (socket, url) = open_socket(options).await?;
        let mut client = Client {
            socket,
            pending: VecDeque::new(),
        };

        // 2. Le premier paquet doit être `RoomInfo`.
        let room = match client.recv_handshake().await? {
            ServerPacket::RoomInfo(room) => room,
            other => return Err(ClientError::UnexpectedPacket(other.cmd().to_string())),
        };

        // 3. On envoie `Connect`.
        let connect = Connect {
            password: options.password.clone(),
            game: options.game.clone(),
            name: options.slot.clone(),
            uuid: options.uuid.clone(),
            version: CLIENT_VERSION,
            items_handling: options.items_handling,
            tags: options.tags.clone(),
            slot_data: Some(options.want_slot_data),
        };
        client.send(&[ClientPacket::Connect(connect)]).await?;

        // 4. On attend `Connected` ou `ConnectionRefused`. Les autres paquets
        //    éventuels (messages du serveur) restent dans la file pour `recv`.
        let mut skipped = VecDeque::new();
        let connected = loop {
            match client.recv_handshake().await? {
                ServerPacket::Connected(connected) => break connected,
                ServerPacket::ConnectionRefused(refused) => {
                    return Err(ClientError::Refused(refused.errors))
                }
                other => skipped.push_back(other),
            }
        };
        // Les paquets mis de côté passent avant ceux qui suivent `Connected`.
        skipped.append(&mut client.pending);
        client.pending = skipped;

        Ok((
            client,
            Session {
                url,
                room,
                connected,
            },
        ))
    }

    /// Envoie des paquets au serveur, dans une seule trame.
    pub async fn send(&mut self, packets: &[ClientPacket]) -> Result<(), ClientError> {
        let text = encode_client_frame(packets)?;
        self.socket.send(Message::text(text)).await?;
        Ok(())
    }

    /// Attend le prochain paquet du serveur.
    ///
    /// Renvoie `Ok(None)` quand le serveur a fermé la connexion.
    pub async fn recv(&mut self) -> Result<Option<ServerPacket>, ClientError> {
        loop {
            if let Some(packet) = self.pending.pop_front() {
                return Ok(Some(packet));
            }
            // `next()` attend le prochain message ; `None` = flux terminé.
            let Some(message) = self.socket.next().await else {
                return Ok(None);
            };
            match message? {
                Message::Text(text) => self.pending.extend(decode_server_frame(text.as_str())?),
                Message::Close(_) => return Ok(None),
                // Ping et Pong : tungstenite y répond tout seul.
                Message::Ping(_) | Message::Pong(_) => {}
                // Le protocole n'utilise que du texte : on ignore le reste.
                Message::Binary(_) | Message::Frame(_) => {
                    tracing::warn!("message binaire ignoré");
                }
            }
        }
    }

    /// Ferme proprement la connexion.
    pub async fn close(mut self) -> Result<(), ClientError> {
        self.socket.close(None).await?;
        Ok(())
    }

    /// Comme `recv`, mais avec un délai, et une fermeture vue comme une erreur.
    async fn recv_handshake(&mut self) -> Result<ServerPacket, ClientError> {
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, self.recv()).await {
            Err(_elapsed) => Err(ClientError::Timeout),
            Ok(result) => result?.ok_or(ClientError::Closed),
        }
    }
}

/// Ouvre le WebSocket, en essayant chaque URL candidate dans l'ordre.
async fn open_socket(options: &ConnectOptions) -> Result<(Socket, String), ClientError> {
    let urls = candidate_urls(&options.address)?;
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(options.max_message_size);
    config.max_frame_size = Some(options.max_message_size);

    let mut last_error = None;
    for url in urls {
        let attempt = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            tokio_tungstenite::connect_async_with_config(url.as_str(), Some(config), false),
        )
        .await;
        match attempt {
            Ok(Ok((socket, _response))) => {
                if url.starts_with("ws://") {
                    tracing::warn!(%url, "connexion non chiffrée");
                }
                return Ok((socket, url));
            }
            Ok(Err(error)) => {
                tracing::info!(%url, %error, "échec de connexion");
                last_error = Some(ClientError::from(error));
            }
            Err(_elapsed) => {
                tracing::info!(%url, "pas de réponse");
                last_error = Some(ClientError::Timeout);
            }
        }
    }
    // `candidate_urls` renvoie toujours au moins une URL, donc `last_error` est rempli.
    Err(last_error.unwrap_or(ClientError::Closed))
}
