//! Une connexion de client : lecture et écriture sur le réseau, rien d'autre.
//!
//! La connexion ne touche pas à l'état de la partie (ADR 0006) :
//! - elle découpe chaque trame reçue en paquets et les passe à la tâche salle ;
//! - elle envoie au client les trames que la tâche salle lui confie ;
//! - elle ferme quand la tâche salle le demande, ou quand le client part.
//!
//! Elle applique aussi les limites réseau (ADR 0007) : taille des messages,
//! débit de paquets, ping et fermeture d'une connexion muette.
//!
//! Une trame qui n'est pas un tableau JSON ferme la connexion, comme sur le
//! serveur officiel (`MultiServer.server`).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ap_protocol::{ClientPacket, ProtocolError};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::error::{CapacityError, Error as WsError};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::info;

use crate::error::ConnectionError;
use crate::limits::{Limits, PacketRate};
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
    /// `cmd` présent mais pas du texte ; on garde ce `cmd` pour nommer son type.
    CmdNotText(Value),
    /// Pas de `cmd` (ou pas un objet) ; on garde le paquet : le serveur
    /// officiel le recopie dans sa réponse.
    NoCmd(Value),
}

/// Sert un client jusqu'à sa déconnexion. Ne renvoie jamais d'erreur :
/// une connexion qui échoue ne doit pas gêner les autres.
pub(crate) async fn handle(
    stream: TcpStream,
    peer: SocketAddr,
    id: ClientId,
    events: Sender<Event>,
    limits: Limits,
) {
    info!(%peer, id, "connexion entrante");
    let result = serve_client(stream, id, &events, &limits).await;
    // Toujours prévenir la salle, même après une erreur. Si la salle est déjà
    // arrêtée, il n'y a plus personne à prévenir : on ignore l'échec.
    let _ = events.send(Event::Closed { id }).await;
    match result {
        Ok(()) => info!(%peer, id, "déconnecté"),
        Err(error) => info!(%peer, id, "connexion fermée : {error}"),
    }
}

/// Socket WebSocket d'un client (sans chiffrement pour l'instant).
type Socket = WebSocketStream<TcpStream>;

async fn serve_client(
    stream: TcpStream,
    id: ClientId,
    events: &Sender<Event>,
    limits: &Limits,
) -> Result<(), ConnectionError> {
    // NF03 : tungstenite refuse lui-même les messages trop gros.
    let config = WebSocketConfig::default()
        .max_message_size(Some(limits.max_message_size))
        .max_frame_size(Some(limits.max_message_size));
    let mut socket = tokio_tungstenite::accept_async_with_config(stream, Some(config)).await?;

    // Canal propre à ce client : la salle y dépose les trames à envoyer.
    // Il est borné ; si la salle le trouve plein, elle sonne `kill` (ADR 0007).
    let (outbox, mut inbox) = mpsc::channel(limits.client_queue.max(1));
    // `Arc` : le signal est partagé entre cette tâche et la salle.
    let kill = Arc::new(Notify::new());
    events
        .send(Event::Opened {
            id,
            outbox,
            kill: Arc::clone(&kill),
        })
        .await
        .map_err(|_| ConnectionError::RoomClosed)?;

    let mut rate = PacketRate::new(limits.packets_per_second, Instant::now());
    // NF05 : un ping à intervalle régulier. `interval` sonne tout de suite la
    // première fois : on saute ce premier tour.
    let mut ping = tokio::time::interval(limits.ping_interval);
    ping.tick().await;
    // Heure de la dernière chose reçue du client (paquet, pong…).
    let mut last_seen = tokio::time::Instant::now();

    loop {
        let deadline = last_seen + limits.silence_timeout;
        // `select!` attend plusieurs choses à la fois et traite la première prête.
        tokio::select! {
            message = socket.next() => {
                // `None` : le client est parti.
                let Some(message) = message else { return Ok(()) };
                let message = match message {
                    Ok(message) => message,
                    Err(WsError::Capacity(CapacityError::MessageTooLong { size, max_size })) => {
                        close_with(socket, CloseCode::Size, "message trop gros");
                        return Err(ConnectionError::TooLarge { size, max_size });
                    }
                    Err(error) => return Err(error.into()),
                };
                last_seen = tokio::time::Instant::now();
                let read = match message {
                    Message::Text(text) => read_frame(text.as_str()),
                    // Le serveur officiel accepte aussi le JSON envoyé en binaire.
                    Message::Binary(bytes) => match std::str::from_utf8(&bytes) {
                        Ok(text) => read_frame(text),
                        Err(_) => Err(ConnectionError::BadFrame("binaire non UTF-8".into())),
                    },
                    // tungstenite répond tout seul aux pings et à la fermeture.
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                    Message::Close(_) => return Ok(()),
                };
                let items = match read {
                    Ok(items) => items,
                    Err(error) => {
                        // Trame illisible : le serveur officiel plante, code 1011.
                        close_with(socket, CloseCode::Error, "");
                        return Err(error);
                    }
                };
                // NF04 : chaque paquet de la trame compte.
                if !rate.allow(items.len(), Instant::now()) {
                    close_with(socket, CloseCode::Policy, "trop de paquets");
                    return Err(ConnectionError::TooManyPackets);
                }
                // `await` : si la file de la salle est pleine, on attend ici,
                // sans lire la suite ; seul ce client est ralenti.
                events
                    .send(Event::Frame { id, items })
                    .await
                    .map_err(|_| ConnectionError::RoomClosed)?;
            }
            outgoing = inbox.recv() => match outgoing {
                Some(Outgoing::Frame(text)) => {
                    send_watched(&mut socket, Message::text(text), &kill, deadline).await?;
                }
                // Fermeture demandée par la salle, ou salle arrêtée (`None`).
                // La salle ne ferme que là où le serveur officiel plante : il
                // ferme alors avec le code 1011 (erreur interne), nous aussi.
                Some(Outgoing::Close) | None => {
                    close_with(socket, CloseCode::Error, "");
                    return Ok(());
                }
            },
            // La salle n'a plus de place pour ce client : il lit trop lentement.
            () = kill.notified() => {
                close_with(socket, CloseCode::Policy, "client trop lent");
                return Err(ConnectionError::TooSlow);
            }
            _ = ping.tick() => {
                let ping = Message::Ping(Vec::new().into());
                send_watched(&mut socket, ping, &kill, deadline).await?;
            }
            () = tokio::time::sleep_until(deadline) => {
                close_with(socket, CloseCode::Away, "aucune réponse");
                return Err(ConnectionError::Silent);
            }
        }
    }
}

/// Envoie un message au client.
///
/// Un client qui ne lit plus bloque l'envoi dès que les tampons réseau sont
/// pleins. Pendant ce temps, on surveille donc aussi le signal `kill` de la
/// salle et le délai de silence : sinon la connexion resterait bloquée ici.
/// On abandonne alors l'envoi en cours ; la socket est fermée en sortant.
async fn send_watched(
    socket: &mut Socket,
    message: Message,
    kill: &Notify,
    deadline: tokio::time::Instant,
) -> Result<(), ConnectionError> {
    tokio::select! {
        result = socket.send(message) => Ok(result?),
        () = kill.notified() => Err(ConnectionError::TooSlow),
        () = tokio::time::sleep_until(deadline) => Err(ConnectionError::Silent),
    }
}

/// Temps laissé au client pour répondre à la fermeture.
const CLOSE_GRACE: Duration = Duration::from_secs(1);

/// Ferme la connexion avec un code et une raison ; si l'envoi échoue, tant pis.
///
/// On attend ensuite un peu la réponse du client. Sans cela, sous Windows, si
/// le client envoie encore quelque chose, la coupure brutale (RST) efface
/// côté client les dernières trames avant qu'il les lise : il ne saurait pas
/// pourquoi il a été déconnecté (F05).
///
/// L'attente se fait dans une tâche à part, qui prend la socket (`move`) : la
/// connexion peut ainsi prévenir la salle tout de suite, et les autres joueurs
/// apprennent le départ sans délai, comme sur le serveur officiel.
fn close_with(mut socket: Socket, code: CloseCode, reason: &'static str) {
    tokio::spawn(async move {
        let frame = CloseFrame {
            code,
            reason: reason.into(),
        };
        if socket.close(Some(frame)).await.is_err() {
            return;
        }
        let drain = async {
            if code != CloseCode::Size {
                // On jette ce qui arrive encore, jusqu'à la réponse du client.
                while let Some(Ok(_)) = socket.next().await {}
                return;
            }
            // Message trop gros : tungstenite s'est arrêté au milieu, son flux
            // n'est plus lisible. On jette les octets bruts reçus par TCP.
            let mut buffer = [0u8; 8192];
            while let Ok(read) = socket.get_mut().read(&mut buffer).await {
                if read == 0 {
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(CLOSE_GRACE, drain).await;
    });
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
        None => return Incoming::NoCmd(value),
        Some(Value::String(_)) => {}
        Some(cmd) => return Incoming::CmdNotText(cmd.clone()),
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
        // N'arrive pas : on a vérifié plus haut que c'est un objet avec un
        // `cmd` texte. La valeur est consommée, on n'a plus rien à recopier.
        Err(_) => Incoming::NoCmd(Value::Null),
    }
}
