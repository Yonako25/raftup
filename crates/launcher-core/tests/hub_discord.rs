//! Connexion Discord et `GET /v1/info` du client du hub, contre un **faux serveur HTTP**.
//!
//! Pas de vrai Discord ni de réseau : le faux hub répond ce que le test lui dit, et le
//! test joue lui-même le navigateur qui revient sur le serveur local du launcher.
//! Le vrai hub est en AGPL (ADR 0003) : `launcher-core` n'en dépend pas, même en test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use launcher_core::hub_client::{
    DiscordOutcome, DiscordReturn, ErrorCode, HubClient, HubClientError, HubUrl, DISCORD_WAIT,
};
use serde_json::Value;

/// Une requête reçue par le faux hub.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    body: String,
}

/// Faux hub : un thread répond à chaque requête avec `route` (code, corps JSON).
struct FakeHub {
    url: HubUrl,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl FakeHub {
    fn start(route: impl Fn(&Seen) -> (u16, String) + Send + 'static) -> FakeHub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_thread = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                if let Some(request) = read_request(&stream) {
                    seen_thread.lock().unwrap().push(request.clone());
                    let (status, body) = route(&request);
                    let text = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(text.as_bytes());
                }
            }
        });
        FakeHub {
            url: HubUrl::parse(&format!("http://127.0.0.1:{port}")).unwrap(),
            seen,
        }
    }

    fn body_of(&self, path: &str) -> Value {
        let seen = self.seen.lock().unwrap();
        let request = seen.iter().rev().find(|s| s.path == path).unwrap();
        serde_json::from_str(&request.body).unwrap()
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn read_request(stream: &TcpStream) -> Option<Seen> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut length = 0usize;
    let mut authorization = None;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':')?;
        let value = value.trim().to_string();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().ok()?,
            "authorization" => authorization = Some(value),
            _ => {}
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok()?;
    Some(Seen {
        method,
        path,
        authorization,
        body: String::from_utf8(body).ok()?,
    })
}

/// Le « navigateur » : une requête GET brute vers le serveur local du launcher.
/// Rend la première ligne de la réponse (`HTTP/1.1 200 OK`).
fn browser_get(port: u16, target: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response.lines().next().unwrap_or_default().to_string()
}

const START: &str = r#"{"authorize_url":"https://discord.com/oauth2/authorize?x=1","state":"ab12","secret":"s3cr3t"}"#;
const SESSION: &str =
    r#"{"status":"logged_in","token":"jeton","user_id":7,"username":"Mimi","expires_at":99}"#;

#[test]
fn info_lit_les_moyens_de_connexion() {
    let hub = FakeHub::start(|_| {
        (
            200,
            r#"{"service":"raftup-hub","api_version":1,"name":"Rafthub","version":"0.1.0","login_methods":["password","discord"],"registration_mode":"invite"}"#
                .to_string(),
        )
    });
    let info = HubClient::new(hub.url.clone()).hub_info().unwrap();
    assert!(info.offers_discord());
    assert_eq!(info.registration_mode, "invite");
    assert_eq!(info.name, "Rafthub");
    assert_eq!(hub.seen()[0].path, "/v1/info");
}

#[test]
fn info_sans_discord() {
    let hub = FakeHub::start(|_| {
        (
            200,
            r#"{"service":"raftup-hub","api_version":1,"login_methods":["password"]}"#.to_string(),
        )
    });
    assert!(!HubClient::new(hub.url.clone())
        .hub_info()
        .unwrap()
        .offers_discord());
}

#[test]
fn info_d_un_hub_ancien() {
    // Pas de route `/v1/info` (404 sans corps JSON) : mot de passe seulement.
    let hub = FakeHub::start(|_| (404, String::new()));
    let info = HubClient::new(hub.url.clone()).hub_info().unwrap();
    assert!(!info.offers_discord());
}

#[test]
fn info_d_un_autre_service_refusee() {
    let hub = FakeHub::start(|_| (200, r#"{"service":"autre"}"#.to_string()));
    assert!(matches!(
        HubClient::new(hub.url.clone()).hub_info(),
        Err(HubClientError::UnexpectedResponse { .. })
    ));
}

#[test]
fn parcours_discord_compte_existant() {
    let hub = FakeHub::start(|seen| match seen.path.as_str() {
        "/v1/oauth/discord/start" => (200, START.to_string()),
        "/v1/oauth/discord/finish" => (200, SESSION.to_string()),
        _ => (404, String::new()),
    });
    let mut client = HubClient::new(hub.url.clone());
    let local = DiscordReturn::bind().unwrap();
    let port = local.port();
    let start = client.discord_start(Some(port)).unwrap();
    assert!(start.authorize_url.starts_with("https://discord.com/"));
    assert_eq!(hub.body_of("/v1/oauth/discord/start")["return_port"], port);

    // Le navigateur revient : d'abord l'icône du site (ignorée), puis un autre `state`
    // (refusé), enfin le bon retour.
    let browser = std::thread::spawn(move || {
        let favicon = browser_get(port, "/favicon.ico");
        let wrong = browser_get(port, "/discord?state=autre&ticket=t1");
        let good = browser_get(port, "/discord?state=ab12&ticket=t1c2");
        (favicon, wrong, good)
    });
    let cancel = AtomicBool::new(false);
    let ticket = local.wait(&start, DISCORD_WAIT, &cancel).unwrap();
    let (favicon, wrong, good) = browser.join().unwrap();
    assert!(favicon.contains("400"), "{favicon}");
    assert!(wrong.contains("400"), "{wrong}");
    assert!(good.contains("200"), "{good}");

    let outcome = client.discord_finish(&ticket).unwrap();
    let DiscordOutcome::LoggedIn(session) = outcome else {
        panic!("attendu : connecté, obtenu {outcome:?}");
    };
    assert_eq!(session.username, "Mimi");
    assert!(client.is_logged_in());
    let body = hub.body_of("/v1/oauth/discord/finish");
    assert_eq!(body["state"], "ab12");
    assert_eq!(body["secret"], "s3cr3t");
    assert_eq!(body["ticket"], "t1c2");
    // Ni `start` ni `finish` n'envoient de jeton.
    assert!(hub.seen().iter().all(|s| s.authorization.is_none()));
    assert!(hub.seen().iter().all(|s| s.method == "POST"));
}

#[test]
fn parcours_discord_nouveau_joueur_sur_invitation() {
    let hub = FakeHub::start(|seen| match seen.path.as_str() {
        "/v1/oauth/discord/start" => (200, START.to_string()),
        "/v1/oauth/discord/finish" => (
            200,
            r#"{"status":"needs_username","suggested_username":"Mimi","invite_required":true}"#
                .to_string(),
        ),
        "/v1/oauth/discord/register" => {
            // Premier essai : pseudo pris ; deuxième : compte créé.
            if seen.body.contains("\"Mimi\"") {
                (
                    409,
                    r#"{"error":"username_taken","message":"ce pseudo est déjà pris"}"#.to_string(),
                )
            } else {
                (201, SESSION.replace("Mimi", "Mimi2"))
            }
        }
        _ => (404, String::new()),
    });
    let mut client = HubClient::new(hub.url.clone());
    let local = DiscordReturn::bind().unwrap();
    let port = local.port();
    let start = client.discord_start(Some(port)).unwrap();
    let browser = std::thread::spawn(move || browser_get(port, "/discord?state=ab12&ticket=t9"));
    let ticket = local
        .wait(&start, DISCORD_WAIT, &AtomicBool::new(false))
        .unwrap();
    browser.join().unwrap();

    let outcome = client.discord_finish(&ticket).unwrap();
    assert_eq!(
        outcome,
        DiscordOutcome::NeedsUsername {
            suggested_username: Some("Mimi".to_string()),
            invite_required: true,
        }
    );
    assert!(!client.is_logged_in());

    let error = client
        .discord_register(&ticket, "Mimi", Some(" K7QX-2M9P "))
        .unwrap_err();
    assert_eq!(error.code(), Some(&ErrorCode::UsernameTaken));
    // Le même ticket resert avec un autre pseudo.
    let session = client
        .discord_register(&ticket, "Mimi2", Some("K7QX-2M9P"))
        .unwrap();
    assert_eq!(session.username, "Mimi2");
    assert!(client.is_logged_in());
    let body = hub.body_of("/v1/oauth/discord/register");
    assert_eq!(body["username"], "Mimi2");
    assert_eq!(body["invite_code"], "K7QX-2M9P");
    assert_eq!(body["ticket"], "t9");
}

#[test]
fn inscription_discord_sans_code_ne_l_envoie_pas() {
    let hub = FakeHub::start(|seen| match seen.path.as_str() {
        "/v1/oauth/discord/start" => (200, START.to_string()),
        _ => (201, SESSION.to_string()),
    });
    let mut client = HubClient::new(hub.url.clone());
    let start = client.discord_start(None).unwrap();
    assert_eq!(
        hub.body_of("/v1/oauth/discord/start"),
        serde_json::json!({})
    );
    let ticket = launcher_core::hub_client::DiscordTicket::new(&start, "t");
    client
        .discord_register(&ticket, "Mimi", Some("  "))
        .unwrap();
    assert!(hub
        .body_of("/v1/oauth/discord/register")
        .get("invite_code")
        .is_none());
    // Pseudo vide : rien n'est envoyé.
    assert!(matches!(
        client.discord_register(&ticket, "  ", None),
        Err(HubClientError::EmptyUsername)
    ));
}

#[test]
fn erreurs_discord_typees() {
    let hub = FakeHub::start(|seen| match seen.path.as_str() {
        "/v1/oauth/discord/start" => (
            503,
            r#"{"error":"discord_disabled","message":"connexion Discord non configurée"}"#
                .to_string(),
        ),
        _ => (
            400,
            r#"{"error":"discord_refused","message":"refusé"}"#.to_string(),
        ),
    });
    let mut client = HubClient::new(hub.url.clone());
    let error = client.discord_start(Some(1234)).unwrap_err();
    assert_eq!(error.code(), Some(&ErrorCode::DiscordDisabled));
    // Un `start` réussi, obtenu d'un second faux hub, pour fabriquer un ticket.
    let other = FakeHub::start(|_| (200, START.to_string()));
    let start = HubClient::new(other.url.clone())
        .discord_start(None)
        .unwrap();
    let ticket = launcher_core::hub_client::DiscordTicket::new(&start, "t");
    let error = client.discord_finish(&ticket).unwrap_err();
    assert_eq!(error.code(), Some(&ErrorCode::DiscordRefused));
}

#[test]
fn attente_du_retour_bornee() {
    let local = DiscordReturn::bind().unwrap();
    let hub = FakeHub::start(|_| (200, START.to_string()));
    let start = HubClient::new(hub.url.clone()).discord_start(None).unwrap();
    let result = local.wait(&start, Duration::from_millis(200), &AtomicBool::new(false));
    assert!(matches!(result, Err(HubClientError::DiscordTimeout)));
}
