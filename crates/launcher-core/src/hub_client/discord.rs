//! Connexion Discord vers un Rafthub (ADR 0033, 0035, 0038).
//!
//! Le hub est le client OAuth : le launcher ne voit jamais le code de Discord. Parcours :
//!
//! 1. [`DiscordReturn::bind`] ouvre un petit serveur local sur `127.0.0.1` (port libre
//!    choisi par le système) ; il ne servira qu'**une** requête utile ;
//! 2. [`HubClient::discord_start`](super::HubClient::discord_start) avec ce port :
//!    le hub rend l'adresse à ouvrir chez Discord, un `state` et un `secret` ;
//! 3. le navigateur du système ouvre l'adresse ; le joueur dit oui chez Discord, qui
//!    renvoie au hub, qui renvoie à `http://127.0.0.1:<port>/discord?state=…&ticket=…` ;
//! 4. [`DiscordReturn::wait`] lit ce retour et rend un [`DiscordTicket`] ;
//! 5. [`HubClient::discord_finish`](super::HubClient::discord_finish) : connecté, ou
//!    pseudo à choisir ([`DiscordOutcome::NeedsUsername`]), puis
//!    [`HubClient::discord_register`](super::HubClient::discord_register).
//!
//! `secret` et `ticket` sont des secrets : leur `Debug` est masqué.

use std::fmt;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::{HubClientError, Session};

/// Temps laissé au joueur pour dire oui chez Discord.
pub const DISCORD_WAIT: Duration = Duration::from_secs(5 * 60);

/// Taille maximale de la requête lue par le serveur local (une ligne et des en-têtes).
const MAX_REQUEST_BYTES: usize = 8 * 1024;

/// Délai de lecture d'une connexion au serveur local.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Toutes les combien le serveur local regarde s'il y a une connexion (ou une annulation).
const POLL_EVERY: Duration = Duration::from_millis(100);

/// Réponse de `POST /v1/oauth/discord/start`.
#[derive(Clone)]
pub struct DiscordStart {
    /// Adresse à ouvrir dans le navigateur du système.
    pub authorize_url: String,
    pub(crate) state: String,
    pub(crate) secret: String,
}

impl fmt::Debug for DiscordStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DiscordStart(***)")
    }
}

/// Forme JSON de la réponse de `start`.
#[derive(Deserialize)]
pub(crate) struct DiscordStartJson {
    pub authorize_url: String,
    pub state: String,
    pub secret: String,
}

/// Ce qu'il faut renvoyer au hub après le retour de Discord : `state`, `secret` et
/// `ticket`. `Clone` : l'interface le garde pour réessayer l'inscription (pseudo pris).
#[derive(Clone, PartialEq, Eq)]
pub struct DiscordTicket {
    pub(crate) state: String,
    pub(crate) secret: String,
    pub(crate) ticket: String,
}

impl DiscordTicket {
    /// Pour les tests et les appels faits sans [`DiscordReturn`] (lien `raftup://`).
    pub fn new(start: &DiscordStart, ticket: impl Into<String>) -> DiscordTicket {
        DiscordTicket {
            state: start.state.clone(),
            secret: start.secret.clone(),
            ticket: ticket.into(),
        }
    }

    /// Les trois valeurs déjà connues (retour par un lien `raftup://`, tests).
    pub fn from_parts(
        state: impl Into<String>,
        secret: impl Into<String>,
        ticket: impl Into<String>,
    ) -> DiscordTicket {
        DiscordTicket {
            state: state.into(),
            secret: secret.into(),
            ticket: ticket.into(),
        }
    }
}

impl fmt::Debug for DiscordTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DiscordTicket(***)")
    }
}

/// Résultat de `finish`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscordOutcome {
    /// Compte trouvé : session ouverte (le jeton est gardé dans le client).
    LoggedIn(Session),
    /// Discord a été lié au compte déjà connecté.
    Linked,
    /// Nouveau joueur : il faut choisir un pseudo (et donner un code d'invitation si
    /// `invite_required`), puis appeler `discord_register`.
    NeedsUsername {
        /// Pseudo proposé (nom Discord, s'il est valide et libre).
        suggested_username: Option<String>,
        invite_required: bool,
    },
}

/// Forme JSON de la réponse de `finish` / `register`. Les champs de la session sont au
/// même niveau que `status` quand il vaut `logged_in`.
#[derive(Deserialize)]
pub(crate) struct DiscordFinishJson {
    pub status: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub suggested_username: Option<String>,
    #[serde(default)]
    pub invite_required: bool,
}

/// Petit serveur HTTP local qui attend le retour du navigateur.
///
/// Il n'écoute que sur `127.0.0.1` : rien d'autre que ce PC ne peut lui parler.
#[derive(Debug)]
pub struct DiscordReturn {
    listener: TcpListener,
    port: u16,
}

impl DiscordReturn {
    /// Ouvre le serveur local sur un port libre.
    pub fn bind() -> Result<DiscordReturn, HubClientError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(local_error)?;
        let port = listener.local_addr().map_err(local_error)?.port();
        // Non bloquant : `wait` regarde régulièrement s'il faut abandonner.
        listener.set_nonblocking(true).map_err(local_error)?;
        Ok(DiscordReturn { listener, port })
    }

    /// Le port à donner au hub (`return_port`).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Attend le retour du navigateur pour la connexion `start`, au plus `timeout`.
    ///
    /// Une requête qui n'est pas ce retour (icône du site, autre `state`) reçoit une
    /// erreur et l'attente continue. `cancel` passé à vrai arrête l'attente.
    pub fn wait(
        self,
        start: &DiscordStart,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<DiscordTicket, HubClientError> {
        let deadline = Instant::now() + timeout;
        loop {
            // `Ordering::Relaxed` : on lit seulement un drapeau, sans autre donnée liée.
            if cancel.load(Ordering::Relaxed) {
                return Err(HubClientError::DiscordCancelled);
            }
            if Instant::now() >= deadline {
                return Err(HubClientError::DiscordTimeout);
            }
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(ticket) = handle(stream, &start.state) {
                        return Ok(DiscordTicket {
                            state: start.state.clone(),
                            secret: start.secret.clone(),
                            ticket,
                        });
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(POLL_EVERY);
                }
                Err(e) => return Err(local_error(e)),
            }
        }
    }
}

fn local_error(error: std::io::Error) -> HubClientError {
    HubClientError::DiscordReturn(error.to_string())
}

/// Lit une requête du navigateur ; rend le `ticket` si c'est le bon retour.
/// Une erreur de lecture ou d'écriture sur cette connexion est ignorée : on attend
/// la suivante.
fn handle(mut stream: TcpStream, expected_state: &str) -> Option<String> {
    // La connexion acceptée hérite parfois du mode non bloquant : on le retire.
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    let head = read_head(&mut stream)?;
    let target = head
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("GET "))
        .and_then(|rest| rest.split(' ').next());
    let result = target.and_then(|target| parse_return(target, expected_state));
    let (status, text) = match &result {
        Some(_) => (
            "200 OK",
            "Connexion Discord reçue. Vous pouvez fermer cet onglet et revenir au launcher Raftup.",
        ),
        None => (
            "400 Bad Request",
            "Ce lien ne correspond pas à la connexion en cours. Recommencez depuis le launcher.",
        ),
    };
    let body = format!(
        "<!doctype html><html lang=\"fr\"><meta charset=\"utf-8\"><title>Raftup</title>\
         <body style=\"font-family:sans-serif;background:#10151c;color:#e6edf3;padding:40px\">\
         <p>{text}</p></body></html>"
    );
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(reply.as_bytes());
    result
}

/// Lit le début de la requête, jusqu'à la ligne vide (ou 8 Kio au plus).
fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    while buffer.len() < MAX_REQUEST_BYTES {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(buffer).ok()
}

/// `/discord?state=…&ticket=…` → le ticket, si `state` est celui attendu.
///
/// `state` et `ticket` sont en hexadécimal côté hub : rien à décoder. Un ticket qui
/// contient autre chose est refusé.
fn parse_return(target: &str, expected_state: &str) -> Option<String> {
    let (path, query) = target.split_once('?')?;
    if path != "/discord" {
        return None;
    }
    let mut state = None;
    let mut ticket = None;
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("state", value)) => state = Some(value),
            Some(("ticket", value)) => ticket = Some(value),
            _ => {}
        }
    }
    let ticket = ticket?;
    let valid = !ticket.is_empty()
        && ticket.len() <= 256
        && ticket.bytes().all(|b| b.is_ascii_alphanumeric());
    (state? == expected_state && valid).then(|| ticket.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retour_lu() {
        assert_eq!(
            parse_return("/discord?state=ab12&ticket=cd34", "ab12"),
            Some("cd34".to_string())
        );
        assert_eq!(
            parse_return("/discord?ticket=cd34&state=ab12", "ab12"),
            Some("cd34".to_string())
        );
    }

    #[test]
    fn retours_refuses() {
        for target in [
            "/discord?state=autre&ticket=cd34",
            "/favicon.ico",
            "/autre?state=ab12&ticket=cd34",
            "/discord?state=ab12",
            "/discord?state=ab12&ticket=",
            "/discord?state=ab12&ticket=%3Cscript%3E",
        ] {
            assert_eq!(parse_return(target, "ab12"), None, "{target}");
        }
    }

    #[test]
    fn secrets_masques() {
        let start = DiscordStart {
            authorize_url: "https://discord.com/x".into(),
            state: "s".into(),
            secret: "secret-a-cacher".into(),
        };
        assert!(!format!("{start:?}").contains("secret-a-cacher"));
        let ticket = DiscordTicket::new(&start, "ticket-a-cacher");
        assert!(!format!("{ticket:?}").contains("cacher"));
    }

    #[test]
    fn attente_annulee() {
        let server = DiscordReturn::bind().unwrap();
        let start = DiscordStart {
            authorize_url: String::new(),
            state: "s".into(),
            secret: "x".into(),
        };
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            server.wait(&start, DISCORD_WAIT, &cancel),
            Err(HubClientError::DiscordCancelled)
        ));
    }

    #[test]
    fn attente_trop_longue() {
        let server = DiscordReturn::bind().unwrap();
        let start = DiscordStart {
            authorize_url: String::new(),
            state: "s".into(),
            secret: "x".into(),
        };
        let cancel = AtomicBool::new(false);
        assert!(matches!(
            server.wait(&start, Duration::from_millis(50), &cancel),
            Err(HubClientError::DiscordTimeout)
        ));
    }
}
