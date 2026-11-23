//! Tests des parties hébergées par Rafthub, côté client (`launcher_core::hub_client`,
//! ADR 0052, étape 2), contre un **faux serveur HTTP**.
//!
//! Le vrai hub est en AGPL (ADR 0003) : `launcher-core` n'en dépend pas, même en test.
//! Le faux serveur note chaque requête et répond ce que le test lui dit, dans la forme
//! du JSON de `crates/hub/src/games/routes.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use base64::Engine;
use launcher_core::hub_client::{
    ErrorCode, GameUpload, HubClient, HubClientError, HubUrl, SessionToken, MAX_SEED_BYTES,
};

/// Une requête reçue par le faux serveur.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    body: String,
}

/// Réponse choisie par le test : code, corps.
type Reply = (u16, String);

struct FakeHub {
    url: HubUrl,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl FakeHub {
    /// `route` reçoit chaque requête et rend la réponse. `Send + 'static` : la fermeture
    /// part dans le thread du serveur.
    fn start(route: impl Fn(&Seen) -> Reply + Send + 'static) -> FakeHub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_thread = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                if let Some(request) = read_request(&stream) {
                    seen_thread.lock().unwrap().push(request.clone());
                    write_reply(stream, route(&request));
                }
            }
        });
        FakeHub {
            url: HubUrl::parse(&format!("http://127.0.0.1:{port}")).unwrap(),
            seen,
        }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// Client déjà connecté (jeton `abc123`).
    fn client(&self) -> HubClient {
        HubClient::with_token(self.url.clone(), SessionToken::new("abc123"))
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

fn write_reply(mut stream: TcpStream, (status, body): Reply) {
    let mut text = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if body.starts_with('{') {
        text.push_str("Content-Type: application/json\r\n");
    }
    text.push_str("\r\n");
    text.push_str(&body);
    let _ = stream.write_all(text.as_bytes());
}

fn error(status: u16, code: &str) -> Reply {
    let body = serde_json::json!({ "error": code, "message": "refusé" }).to_string();
    (status, body)
}

const ID: &str = "0123456789abcdef0123456789abcdef";

/// Une partie, comme le hub la décrit (`GameView`).
fn game_json() -> serde_json::Value {
    serde_json::json!({
        "id": ID,
        "name": "Partie de Mimi",
        "seed_name": "4242",
        "state": "active",
        "address": "hub.example.org:38281",
        "host": "hub.example.org",
        "port": 38281,
        "fingerprint": "ab".repeat(32),
        "players": 2,
        "has_password": true,
        "created_at": 1_900_000_000,
    })
}

fn upload() -> GameUpload {
    GameUpload {
        name: "Partie de Mimi".into(),
        password: Some("secret".into()),
        seed: vec![1, 2, 3, 4],
        save: None,
    }
}

#[test]
fn envoi_de_la_seed_en_base64() {
    let hub = FakeHub::start(|_| {
        let mut body = game_json();
        body["admin_password"] = "ADMINPASS".into();
        body["resumed"] = false.into();
        (201, body.to_string())
    });
    let dir = tempfile::tempdir().unwrap();
    let seed = dir.path().join("AP_4242.archipelago");
    std::fs::write(&seed, b"seed-bytes").unwrap();
    let upload = GameUpload::from_files(" Partie de Mimi ", Some(""), &seed, None).unwrap();

    let created = hub.client().create_game(&upload).unwrap();
    assert_eq!(created.game.id, ID);
    assert_eq!(created.admin_password, "ADMINPASS");
    assert!(created.game.is_active());
    assert_eq!(created.game.address, "hub.example.org:38281");

    let seen = hub.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].path, "/v1/games");
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer abc123"));
    let body: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["name"], "Partie de Mimi");
    // Mot de passe vide : partie ouverte, le champ n'est pas envoyé.
    assert!(body.get("password").is_none());
    assert!(body.get("save").is_none());
    let sent = base64::engine::general_purpose::STANDARD
        .decode(body["seed"].as_str().unwrap())
        .unwrap();
    assert_eq!(sent, b"seed-bytes");
}

#[test]
fn mot_de_passe_et_sauvegarde_envoyes() {
    let hub = FakeHub::start(|_| error(400, "invalid_save"));
    let mut upload = upload();
    upload.save = Some(vec![9, 9]);
    let error = hub.client().create_game(&upload).unwrap_err();
    assert_eq!(error.code(), Some(&ErrorCode::InvalidSave));
    let body: serde_json::Value = serde_json::from_str(&hub.seen()[0].body).unwrap();
    assert_eq!(body["password"], "secret");
    assert_eq!(body["save"], "CQk=");
}

#[test]
fn codes_d_erreur_du_hub() {
    for (status, code, expected) in [
        (409, "game_limit_reached", ErrorCode::GameLimitReached),
        (503, "hub_full", ErrorCode::HubFull),
        (400, "too_many_players", ErrorCode::TooManyPlayers),
        (413, "seed_too_large", ErrorCode::SeedTooLarge),
        (503, "hosting_disabled", ErrorCode::HostingDisabled),
        (503, "no_free_port", ErrorCode::NoFreePort),
        (400, "invalid_seed", ErrorCode::InvalidSeed),
    ] {
        let hub = FakeHub::start(move |_| error(status, code));
        let error = hub.client().create_game(&upload()).unwrap_err();
        assert_eq!(error.code(), Some(&expected), "{code}");
    }
}

/// La limite de corps de la route répond 413 sans le JSON habituel : c'est quand même
/// « seed trop grosse ».
#[test]
fn refus_de_taille_sans_corps_json() {
    let hub = FakeHub::start(|_| (413, "length limit exceeded".to_string()));
    let error = hub.client().create_game(&upload()).unwrap_err();
    assert_eq!(error.code(), Some(&ErrorCode::SeedTooLarge));
}

#[test]
fn seed_trop_grosse_refusee_avant_envoi() {
    let dir = tempfile::tempdir().unwrap();
    let seed = dir.path().join("grosse.archipelago");
    // Fichier creux : 32 Mio + 1 octet sans rien écrire sur le disque.
    std::fs::File::create(&seed)
        .unwrap()
        .set_len(MAX_SEED_BYTES + 1)
        .unwrap();
    let error = GameUpload::from_files("P", None, &seed, None).unwrap_err();
    assert!(
        matches!(error, HubClientError::TooLarge { .. }),
        "{error:?}"
    );
    let missing = GameUpload::from_files("P", None, &dir.path().join("absente"), None);
    assert!(matches!(missing, Err(HubClientError::File { .. })));
}

#[test]
fn etat_d_une_partie_avec_ses_slots() {
    let hub = FakeHub::start(|_| {
        let mut body = game_json();
        body["is_host"] = true.into();
        body["admin_password"] = "ADMINPASS".into();
        body["connections"] = 1.into();
        body["slots"] = serde_json::json!([{
            "team": 0, "slot": 1, "name": "Mimi", "game": "APQuest", "is_group": false,
            "connections": 1, "tags": ["AP"], "checked": 3, "total": 10, "status": 20,
            "last_activity": 1.5, "away": null, "release_votes": 0
        }]);
        (200, body.to_string())
    });
    let status = hub.client().game_status(ID).unwrap();
    assert!(status.is_host);
    assert_eq!(status.admin_password.as_deref(), Some("ADMINPASS"));
    assert_eq!(status.slots.len(), 1);
    assert_eq!(status.slots[0].name, "Mimi");
    assert_eq!(status.slots[0].checked, 3);
    assert_eq!(hub.seen()[0].path, format!("/v1/games/{ID}"));
    assert_eq!(hub.seen()[0].method, "GET");
}

#[test]
fn identifiant_mal_forme_jamais_envoye() {
    let hub = FakeHub::start(|_| (200, "{}".to_string()));
    for id in ["../admin", "", "xyz"] {
        assert!(matches!(
            hub.client().game_status(id),
            Err(HubClientError::BadGameId)
        ));
        assert!(matches!(
            hub.client().delete_game(id),
            Err(HubClientError::BadGameId)
        ));
    }
    assert!(hub.seen().is_empty());
}

#[test]
fn liste_et_suppression() {
    let hub = FakeHub::start(|seen| match seen.method.as_str() {
        "GET" => (
            200,
            serde_json::json!({ "games": [game_json()] }).to_string(),
        ),
        _ => (204, String::new()),
    });
    let games = hub.client().games().unwrap();
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].name, "Partie de Mimi");
    hub.client().delete_game(ID).unwrap();
    let seen = hub.seen();
    assert_eq!(seen[0].path, "/v1/games");
    assert_eq!(seen[1].method, "DELETE");
    assert_eq!(seen[1].path, format!("/v1/games/{ID}"));
}

#[test]
fn hebergement_annonce_par_le_hub() {
    let hub = FakeHub::start(|_| {
        (
            200,
            r#"{"service":"raftup-hub","login_methods":["password"],"hosting":true}"#.into(),
        )
    });
    assert!(HubClient::new(hub.url.clone())
        .hub_info()
        .unwrap()
        .offers_hosting());

    // Un hub plus ancien ne dit rien : pas d'hébergement.
    let old = FakeHub::start(|_| (200, r#"{"service":"raftup-hub"}"#.into()));
    assert!(!HubClient::new(old.url.clone())
        .hub_info()
        .unwrap()
        .offers_hosting());
}

#[test]
fn sans_session_pas_d_envoi() {
    let hub = FakeHub::start(|_| (200, "{}".to_string()));
    let client = HubClient::new(hub.url.clone());
    assert!(matches!(
        client.create_game(&upload()),
        Err(HubClientError::NotLoggedIn)
    ));
    assert!(hub.seen().is_empty());
}
