//! Une connexion de client : accueil, lecture des trames, réponses.
//!
//! Comme le serveur officiel (`MultiServer.server`) :
//! - dès l'ouverture, le serveur envoie `RoomInfo` ;
//! - chaque trame est un tableau JSON de paquets, traités dans l'ordre ;
//! - une trame illisible ou un paquet sans `cmd` ferme la connexion ;
//! - un `cmd` qui n'est pas du texte donne `InvalidPacket`, sans fermer.

use std::net::SocketAddr;
use std::sync::Arc;

use ap_protocol::server::InvalidPacket;
use ap_protocol::{encode_server_frame, ClientPacket, ProtocolError, ServerPacket};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, info};

use crate::error::ConnectionError;
use crate::room::{now, Room};

/// Socket WebSocket d'un client, sur une connexion TCP en clair.
type Socket = WebSocketStream<TcpStream>;

/// Ce qu'on a trouvé dans un élément de la trame.
enum Incoming {
    /// Paquet lu correctement (dans une `Box` : certains paquets sont gros).
    Packet(Box<ClientPacket>),
    /// Commande connue, mais champs incorrects.
    BadArguments {
        cmd: String,
        error: serde_json::Error,
    },
    /// `cmd` présent mais pas du texte : `InvalidPacket`, et on continue.
    CmdNotText,
    /// Pas de `cmd` (ou pas un objet) : `InvalidPacket`, puis fermeture.
    NoCmd,
}

/// Sert un client jusqu'à sa déconnexion. Ne renvoie jamais d'erreur :
/// une connexion qui échoue ne doit pas gêner les autres.
pub(crate) async fn handle(stream: TcpStream, peer: SocketAddr, room: Arc<Room>) {
    info!(%peer, "connexion entrante");
    match serve_client(stream, &room).await {
        Ok(()) => info!(%peer, "déconnecté"),
        Err(error) => info!(%peer, "connexion fermée : {error}"),
    }
}

async fn serve_client(stream: TcpStream, room: &Room) -> Result<(), ConnectionError> {
    let mut socket = tokio_tungstenite::accept_async(stream).await?;
    send(
        &mut socket,
        vec![ServerPacket::RoomInfo(room.room_info(now()))],
    )
    .await?;

    // `next()` attend le prochain message ; `None` : le client est parti.
    while let Some(message) = socket.next().await {
        let incoming = match message? {
            Message::Text(text) => read_frame(text.as_str())?,
            // Le serveur officiel accepte aussi le JSON envoyé en binaire.
            Message::Binary(bytes) => match std::str::from_utf8(&bytes) {
                Ok(text) => read_frame(text)?,
                Err(_) => return Err(ConnectionError::BadFrame("binaire non UTF-8".into())),
            },
            // tungstenite répond tout seul aux pings et à la fermeture.
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            Message::Close(_) => break,
        };

        // Les réponses à toute la trame partent ensemble, dans une seule trame.
        let mut replies = Vec::new();
        let mut must_close = false;
        for item in incoming {
            match item {
                Incoming::Packet(packet) => handle_packet(room, *packet, &mut replies),
                Incoming::BadArguments { cmd, error } => {
                    // Avant la connexion d'un joueur, le serveur officiel ignore
                    // tout sauf `Connect` et `GetDataPackage` (étape 4).
                    debug!(%cmd, "paquet aux champs incorrects ignoré : {error}");
                }
                Incoming::CmdNotText => replies.push(invalid_cmd("cmd doit être du texte")),
                Incoming::NoCmd => {
                    replies.push(invalid_cmd("paquet sans champ « cmd »"));
                    must_close = true;
                    break;
                }
            }
        }
        if !replies.is_empty() {
            send(&mut socket, replies).await?;
        }
        if must_close {
            // On tente une fermeture propre ; si elle échoue, tant pis.
            let _ = socket.close(None).await;
            return Err(ConnectionError::BadFrame("paquet sans « cmd »".into()));
        }
    }
    Ok(())
}

/// Lit une trame : un tableau JSON de paquets.
///
/// Erreur si ce n'est pas du JSON ou pas un tableau ; sinon, un résultat par élément.
fn read_frame(text: &str) -> Result<Vec<Incoming>, ConnectionError> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| ConnectionError::BadFrame(e.to_string()))?;
    let Value::Array(items) = value else {
        return Err(ConnectionError::BadFrame(
            "la trame n'est pas un tableau".into(),
        ));
    };
    Ok(items.into_iter().map(read_packet).collect())
}

fn read_packet(value: Value) -> Incoming {
    match value.get("cmd") {
        None => return Incoming::NoCmd,
        Some(Value::String(_)) => {}
        Some(_) => return Incoming::CmdNotText,
    }
    match ClientPacket::from_value(value) {
        Ok(packet) => Incoming::Packet(Box::new(packet)),
        Err(ProtocolError::InvalidPacket { cmd, source }) => {
            Incoming::BadArguments { cmd, error: source }
        }
        // Déjà vérifié plus haut : objet avec un `cmd` texte.
        Err(_) => Incoming::NoCmd,
    }
}

/// Traite un paquet lu correctement. Les réponses vont dans `replies`.
fn handle_packet(_room: &Room, packet: ClientPacket, _replies: &mut Vec<ServerPacket>) {
    // Étape 3 : aucun paquet n'est encore géré. Le serveur officiel ignore
    // aussi tout ce qu'un client non connecté envoie, sauf `Connect` et
    // `GetDataPackage`, qui arrivent à l'étape 4.
    debug!(cmd = packet.cmd(), "paquet pas encore géré, ignoré");
}

fn invalid_cmd(text: &str) -> ServerPacket {
    ServerPacket::InvalidPacket(InvalidPacket {
        kind: "cmd".to_string(),
        original_cmd: None,
        text: text.to_string(),
    })
}

/// Envoie des paquets au client, dans une seule trame.
async fn send(socket: &mut Socket, packets: Vec<ServerPacket>) -> Result<(), ConnectionError> {
    let text = encode_server_frame(&packets)?;
    socket.send(Message::text(text)).await?;
    Ok(())
}
