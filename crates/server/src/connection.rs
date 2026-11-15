//! Une connexion de client : lecture et écriture sur le réseau, rien d'autre.
//!
//! La connexion ne touche pas à l'état de la partie (ADR 0006) :
//! - elle découpe chaque trame reçue en paquets et les passe à la tâche salle ;
//! - elle envoie au client les trames que la tâche salle lui confie ;
//! - elle ferme quand la tâche salle le demande, ou quand le client part.
//!
//! Une trame qui n'est pas un tableau JSON ferme la connexion, comme sur le
//! serveur officiel (`MultiServer.server`).

use std::net::SocketAddr;

use ap_protocol::{ClientPacket, ProtocolError};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;
use tracing::info;

use crate::error::ConnectionError;
use crate::room_task::{ClientId, Event, Outgoing};

/// Ce qu'on a trouvé dans un élément de la trame.
#[derive(Debug)]
pub(crate) enum Incoming {
    /// Paquet lu correctement (dans une `Box` : certains paquets sont gros).
    Packet(Box<ClientPacket>),
    /// Commande connue, mais champs incorrects.
    BadArguments {
        cmd: String,
        error: serde_json::Error,
        /// Le paquet tel que reçu, gardé seulement pour `Bounce` : la réponse
        /// du serveur officiel nomme le champ fautif.
        raw: Option<Value>,
    },
    /// `cmd` présent mais pas du texte.
    CmdNotText,
    /// Pas de `cmd` (ou pas un objet).
    NoCmd,
}

/// Sert un client jusqu'à sa déconnexion. Ne renvoie jamais d'erreur :
/// une connexion qui échoue ne doit pas gêner les autres.
pub(crate) async fn handle(
    stream: TcpStream,
    peer: SocketAddr,
    id: ClientId,
    events: UnboundedSender<Event>,
) {
    info!(%peer, id, "connexion entrante");
    let result = serve_client(stream, id, &events).await;
    // Toujours prévenir la salle, même après une erreur. Si la salle est déjà
    // arrêtée, il n'y a plus personne à prévenir : on ignore l'échec.
    let _ = events.send(Event::Closed { id });
    match result {
        Ok(()) => info!(%peer, id, "déconnecté"),
        Err(error) => info!(%peer, id, "connexion fermée : {error}"),
    }
}

async fn serve_client(
    stream: TcpStream,
    id: ClientId,
    events: &UnboundedSender<Event>,
) -> Result<(), ConnectionError> {
    let mut socket = tokio_tungstenite::accept_async(stream).await?;

    // Canal propre à ce client : la salle y dépose les trames à envoyer.
    let (outbox, mut inbox) = unbounded_channel();
    events
        .send(Event::Opened { id, outbox })
        .map_err(|_| ConnectionError::RoomClosed)?;

    loop {
        // `select!` attend deux choses à la fois et traite la première prête :
        // un message du client, ou une trame à lui envoyer.
        tokio::select! {
            message = socket.next() => {
                // `None` : le client est parti.
                let Some(message) = message else { return Ok(()) };
                let items = match message? {
                    Message::Text(text) => read_frame(text.as_str())?,
                    // Le serveur officiel accepte aussi le JSON envoyé en binaire.
                    Message::Binary(bytes) => match std::str::from_utf8(&bytes) {
                        Ok(text) => read_frame(text)?,
                        Err(_) => {
                            return Err(ConnectionError::BadFrame("binaire non UTF-8".into()))
                        }
                    },
                    // tungstenite répond tout seul aux pings et à la fermeture.
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                    Message::Close(_) => return Ok(()),
                };
                events
                    .send(Event::Frame { id, items })
                    .map_err(|_| ConnectionError::RoomClosed)?;
            }
            outgoing = inbox.recv() => match outgoing {
                Some(Outgoing::Frame(text)) => socket.send(Message::text(text)).await?,
                // Fermeture demandée par la salle, ou salle arrêtée (`None`).
                Some(Outgoing::Close) | None => {
                    // On tente une fermeture propre ; si elle échoue, tant pis.
                    let _ = socket.close(None).await;
                    return Ok(());
                }
            },
        }
    }
}

/// Lit une trame : un tableau JSON de paquets.
///
/// Erreur si ce n'est pas du JSON ou pas un tableau ; sinon, un résultat par élément.
pub(crate) fn read_frame(text: &str) -> Result<Vec<Incoming>, ConnectionError> {
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
    // `from_value` consomme la valeur : on en garde une copie si besoin.
    let raw = (value["cmd"] == "Bounce").then(|| value.clone());
    match ClientPacket::from_value(value) {
        Ok(packet) => Incoming::Packet(Box::new(packet)),
        Err(ProtocolError::InvalidPacket { cmd, source }) => Incoming::BadArguments {
            cmd,
            error: source,
            raw,
        },
        // Déjà vérifié plus haut : objet avec un `cmd` texte.
        Err(_) => Incoming::NoCmd,
    }
}
