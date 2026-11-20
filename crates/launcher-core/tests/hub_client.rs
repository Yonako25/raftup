//! Tests du client du hub (`launcher_core::hub_client`) contre un **faux serveur HTTP**.
//!
//! Le vrai hub est en AGPL (ADR 0003) : `launcher-core` n'en dépend pas, même en test.
//! Le faux serveur ci-dessous lit chaque requête, la note, et répond ce que le test lui
//! dit. Le dernier test (`#[ignore]`) parle à un vrai `raftup-hub` lancé à part :
//!
//! ```text
//! cargo run -p hub --bin raftup-hub -- --db sqlite::memory: --listen 127.0.0.1:8787
//! RAFTUP_HUB_TEST_URL=http://127.0.0.1:8787 cargo test -p launcher-core --test hub_client -- --ignored
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use launcher_core::hub_client::{
    ErrorCode, HubClient, HubClientError, HubUrl, MemoryStore, Password, Presence, RequestOutcome,
    SessionToken, Status, TokenStore,
};

/// Une requête reçue par le faux serveur.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    body: String,
}

/// Réponse choisie par le test : code, en-têtes en plus, corps.
type Reply = (u16, Vec<(&'static str, String)>, String);

/// Faux hub : un thread accepte les connexions et répond avec `route`.
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

fn write_reply(mut stream: TcpStream, (status, headers, body): Reply) {
    let mut text = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if !body.is_empty() {
        text.push_str("Content-Type: application/json\r\n");
    }
    for (name, value) in headers {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    text.push_str("\r\n");
    text.push_str(&body);
    let _ = stream.write_all(text.as_bytes());
}

fn ok(status: u16, body: &str) -> Reply {
    (status, Vec::new(), body.to_string())
}

fn error(status: u16, code: &str, message: &str) -> Reply {
    let body = serde_json::json!({ "error": code, "message": message }).to_string();
    (status, Vec::new(), body)
}

fn body_json(seen: &Seen) -> serde_json::Value {
    serde_json::from_str(&seen.body).unwrap()
}

const SESSION: &str = r#"{"token":"abc123","user_id":7,"username":"Mimi","expires_at":1900000000}"#;

#[test]
fn inscription_envoie_les_trois_champs() {
    let hub = FakeHub::start(|_| ok(201, r#"{"user_id":7,"username":"Mimi"}"#));
    let client = HubClient::new(hub.url.clone());
    let account = client
        .register(
            "mimi@example.org",
            &Password::new("motdepasse"),
            "Mimi",
            None,
        )
        .unwrap();
    assert_eq!(account.username, "Mimi");
    assert_eq!(account.user_id, 7);
    let seen = hub.seen();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].path, "/v1/accounts");
    assert_eq!(seen[0].authorization, None);
    assert_eq!(
        body_json(&seen[0]),
        serde_json::json!({"email": "mimi@example.org", "password": "motdepasse", "username": "Mimi"})
    );
}

#[test]
fn inscription_avec_code_d_invitation() {
    let hub = FakeHub::start(|_| ok(201, r#"{"user_id":7,"username":"Mimi"}"#));
    let client = HubClient::new(hub.url.clone());
    client
        .register(
            "mimi@example.org",
            &Password::new("motdepasse"),
            "Mimi",
            Some("  K7QX-2M9P-ABCD-EFGH "),
        )
        .unwrap();
    // Espaces retirés ; casse et tirets laissés au hub, qui les ignore.
    assert_eq!(
        body_json(&hub.seen()[0])["invite_code"],
        serde_json::json!("K7QX-2M9P-ABCD-EFGH")
    );
}

#[test]
fn code_d_invitation_vide_pas_envoye() {
    let hub = FakeHub::start(|_| ok(201, r#"{"user_id":7,"username":"Mimi"}"#));
    let client = HubClient::new(hub.url.clone());
    for code in [None, Some(""), Some("   ")] {
        client
            .register(
                "mimi@example.org",
                &Password::new("motdepasse"),
                "Mimi",
                code,
            )
            .unwrap();
    }
    let seen = hub.seen();
    assert_eq!(seen.len(), 3);
    for request in seen {
        assert!(
            body_json(&request).get("invite_code").is_none(),
            "{}",
            request.body
        );
    }
}

#[test]
fn erreurs_d_inscription_sur_invitation_typees() {
    for (text, expected) in [
        ("registration_closed", ErrorCode::RegistrationClosed),
        ("invite_required", ErrorCode::InviteRequired),
        ("invalid_invite", ErrorCode::InvalidInvite),
    ] {
        // `move` : la fermeture garde sa propre copie de `text` dans le thread du serveur.
        let hub = FakeHub::start(move |_| error(403, text, "refusé"));
        let client = HubClient::new(hub.url.clone());
        let err = client
            .register(
                "a@example.org",
                &Password::new("motdepasse"),
                "Mimi",
                Some("X"),
            )
            .unwrap_err();
        match err {
            HubClientError::Api { status, code, .. } => {
                assert_eq!(status, 403);
                assert_eq!(code, expected);
            }
            other => panic!("erreur inattendue : {other:?}"),
        }
    }
}

#[test]
fn connexion_garde_le_jeton_pour_les_appels_suivants() {
    let hub = FakeHub::start(|request| match request.path.as_str() {
        "/v1/sessions" => ok(200, SESSION),
        "/v1/friends" => ok(
            200,
            r#"[{"username":"Bob","presence":{"status":"in_game","game":"APQuest"}},
                {"username":"Zoé","presence":{"status":"offline"}}]"#,
        ),
        _ => ok(404, ""),
    });
    let mut client = HubClient::new(hub.url.clone());
    assert!(!client.is_logged_in());
    let session = client
        .login("mimi@example.org", &Password::new("motdepasse"))
        .unwrap();
    assert_eq!(session.username, "Mimi");
    assert_eq!(session.token, SessionToken::new("abc123"));
    assert!(client.is_logged_in());

    let friends = client.friends().unwrap();
    assert_eq!(friends.len(), 2);
    assert_eq!(friends[0].presence, Presence::in_game("APQuest"));
    assert_eq!(friends[1].username, "Zoé");
    assert_eq!(friends[1].presence.status, Status::Offline);

    let seen = hub.seen();
    assert_eq!(seen[1].authorization.as_deref(), Some("Bearer abc123"));
}

#[test]
fn erreur_du_hub_typee() {
    let hub = FakeHub::start(|_| error(409, "username_taken", "ce pseudo est déjà pris"));
    let client = HubClient::new(hub.url.clone());
    let err = client
        .register("a@example.org", &Password::new("motdepasse"), "Mimi", None)
        .unwrap_err();
    match &err {
        HubClientError::Api {
            status,
            code,
            message,
        } => {
            assert_eq!(*status, 409);
            assert_eq!(*code, ErrorCode::UsernameTaken);
            assert_eq!(message, "ce pseudo est déjà pris");
        }
        other => panic!("erreur inattendue : {other:?}"),
    }
    // Le message affiché est celui du hub, en français.
    assert_eq!(err.to_string(), "ce pseudo est déjà pris");
}

#[test]
fn reponse_sans_json_inattendue() {
    let hub = FakeHub::start(|_| ok(404, "pas ici"));
    let client = HubClient::with_token(hub.url.clone(), SessionToken::new("t"));
    assert!(matches!(
        client.friends(),
        Err(HubClientError::UnexpectedResponse { status: 404, .. })
    ));
}

#[test]
fn redirection_jamais_suivie() {
    let hub = FakeHub::start(|_| {
        (
            302,
            vec![("Location", "http://127.0.0.1:1/vol".to_string())],
            String::new(),
        )
    });
    let client = HubClient::with_token(hub.url.clone(), SessionToken::new("t"));
    let err = client.friends().unwrap_err();
    assert!(
        matches!(err, HubClientError::UnexpectedResponse { status: 302, .. }),
        "{err:?}"
    );
    // Une seule requête : la redirection n'a pas été suivie.
    assert_eq!(hub.seen().len(), 1);
}

#[test]
fn hub_injoignable() {
    // Un port réservé puis relâché : personne n'écoute.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = HubUrl::parse(&format!("http://127.0.0.1:{port}")).unwrap();
    let err = HubClient::new(url).api_info().unwrap_err();
    assert!(matches!(err, HubClientError::Network(_)), "{err:?}");
}

#[test]
fn version_et_service_verifies() {
    let hub = FakeHub::start(|_| ok(200, r#"{"service":"raftup-hub","api_version":1}"#));
    assert_eq!(
        HubClient::new(hub.url.clone())
            .api_info()
            .unwrap()
            .api_version,
        1
    );

    let other = FakeHub::start(|_| ok(200, r#"{"service":"autre","api_version":1}"#));
    assert!(matches!(
        HubClient::new(other.url.clone()).api_info(),
        Err(HubClientError::UnexpectedResponse { .. })
    ));

    let newer = FakeHub::start(|_| ok(200, r#"{"service":"raftup-hub","api_version":2}"#));
    assert!(matches!(
        HubClient::new(newer.url.clone()).api_info(),
        Err(HubClientError::UnexpectedResponse { .. })
    ));
}

#[test]
fn deconnexion_oublie_le_jeton_meme_si_deja_expire() {
    let hub = FakeHub::start(|_| error(401, "unauthorized", "session absente"));
    let mut client = HubClient::with_token(hub.url.clone(), SessionToken::new("vieux"));
    client.logout().unwrap();
    assert!(!client.is_logged_in());
    let seen = hub.seen();
    assert_eq!(seen[0].method, "DELETE");
    assert_eq!(seen[0].path, "/v1/sessions/current");
}

#[test]
fn deconnexion_hors_ligne_oublie_quand_meme() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = HubUrl::parse(&format!("http://127.0.0.1:{port}")).unwrap();
    let mut client = HubClient::with_token(url, SessionToken::new("t"));
    assert!(client.logout().is_err());
    assert!(!client.is_logged_in());
}

#[test]
fn reprise_de_session_rangee() {
    let hub = FakeHub::start(|request| {
        if request.authorization.as_deref() == Some("Bearer bon") {
            ok(
                200,
                r#"{"user_id":7,"username":"Mimi","email":"m@example.org"}"#,
            )
        } else {
            error(401, "unauthorized", "session absente")
        }
    });
    let store = MemoryStore::new();
    // Rien de rangé : pas de session, pas d'appel.
    assert!(HubClient::restore(hub.url.clone(), &store)
        .unwrap()
        .is_none());
    assert!(hub.seen().is_empty());

    store.save(&hub.url, &SessionToken::new("bon")).unwrap();
    let (client, me) = HubClient::restore(hub.url.clone(), &store)
        .unwrap()
        .unwrap();
    assert_eq!(me.username, "Mimi");
    assert!(client.is_logged_in());

    // Jeton révoqué : oublié dans le magasin.
    store.save(&hub.url, &SessionToken::new("revoque")).unwrap();
    assert!(HubClient::restore(hub.url.clone(), &store)
        .unwrap()
        .is_none());
    assert_eq!(store.load(&hub.url).unwrap(), None);
}

#[test]
fn se_souvenir_range_le_jeton() {
    let hub = FakeHub::start(|_| ok(200, SESSION));
    let store = MemoryStore::new();
    let mut client = HubClient::new(hub.url.clone());
    assert!(matches!(
        client.remember(&store),
        Err(HubClientError::NotLoggedIn)
    ));
    client.login("m@example.org", &Password::new("x")).unwrap();
    client.remember(&store).unwrap();
    assert_eq!(
        store.load(&hub.url).unwrap(),
        Some(SessionToken::new("abc123"))
    );
}

#[test]
fn demandes_d_ami_et_chemins_encodes() {
    let hub = FakeHub::start(
        |request| match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/v1/friends/requests") => ok(201, r#"{"status":"pending"}"#),
            ("GET", "/v1/friends/requests") => ok(
                200,
                r#"{"incoming":[{"username":"Bob"}],"outgoing":[{"username":"a.b"}]}"#,
            ),
            _ => (204, Vec::new(), String::new()),
        },
    );
    let client = HubClient::with_token(hub.url.clone(), SessionToken::new("t"));
    assert_eq!(
        client.send_request("  Bob ").unwrap(),
        RequestOutcome::Pending
    );
    let requests = client.requests().unwrap();
    assert_eq!(requests.incoming[0].username, "Bob");
    assert_eq!(requests.outgoing[0].username, "a.b");
    client.accept_request("a.b").unwrap();
    client.decline_request("Bob").unwrap();
    client.cancel_request("Zoé").unwrap();
    client.remove_friend("x/y").unwrap();
    assert!(matches!(
        client.send_request(""),
        Err(HubClientError::EmptyUsername)
    ));

    let seen = hub.seen();
    assert_eq!(body_json(&seen[0]), serde_json::json!({"username": "Bob"}));
    let calls: Vec<(String, String)> = seen[2..]
        .iter()
        .map(|s| (s.method.clone(), s.path.clone()))
        .collect();
    assert_eq!(
        calls,
        [
            ("POST".into(), "/v1/friends/requests/a%2Eb/accept".into()),
            ("POST".into(), "/v1/friends/requests/Bob/decline".into()),
            ("DELETE".into(), "/v1/friends/requests/Zo%C3%A9".into()),
            ("DELETE".into(), "/v1/friends/x%2Fy".into()),
        ]
    );
}

#[test]
fn demande_croisee_acceptee() {
    let hub = FakeHub::start(|_| ok(200, r#"{"status":"accepted"}"#));
    let client = HubClient::with_token(hub.url.clone(), SessionToken::new("t"));
    assert_eq!(
        client.send_request("Bob").unwrap(),
        RequestOutcome::Accepted
    );
}

#[test]
fn presence_envoyee_comme_le_hub_l_attend() {
    let hub = FakeHub::start(|_| (204, Vec::new(), String::new()));
    let client = HubClient::with_token(hub.url.clone(), SessionToken::new("t"));
    client.set_presence(&Presence::in_game("APQuest")).unwrap();
    client.set_presence(&Presence::online()).unwrap();
    let seen = hub.seen();
    assert_eq!(seen[0].method, "PUT");
    assert_eq!(seen[0].path, "/v1/presence");
    assert_eq!(
        body_json(&seen[0]),
        serde_json::json!({"status": "in_game", "game": "APQuest"})
    );
    assert_eq!(body_json(&seen[1]), serde_json::json!({"status": "online"}));
}

/// Scénario complet contre un vrai `raftup-hub` (voir l'en-tête du fichier).
#[test]
#[ignore = "demande un raftup-hub lancé à part (RAFTUP_HUB_TEST_URL)"]
fn reel_scenario_complet() {
    let url = std::env::var("RAFTUP_HUB_TEST_URL").expect("RAFTUP_HUB_TEST_URL");
    let url = HubUrl::parse(&url).unwrap();
    // Pseudos uniques à chaque lancement : la base peut déjà contenir les précédents.
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        % 1_000_000_000;
    let (alice, bob) = (format!("alice{suffix}"), format!("bob{suffix}"));
    let password = Password::new("motdepasse-long");

    let mut a = HubClient::new(url.clone());
    let mut b = HubClient::new(url.clone());
    a.api_info().unwrap();
    a.register(&format!("{alice}@example.org"), &password, &alice, None)
        .unwrap();
    b.register(&format!("{bob}@example.org"), &password, &bob, None)
        .unwrap();
    let err = a
        .register(&format!("{alice}@example.org"), &password, &alice, None)
        .unwrap_err();
    assert_eq!(err.code(), Some(&ErrorCode::UsernameTaken));

    a.login(&format!("{alice}@example.org"), &password).unwrap();
    b.login(&format!("{bob}@example.org"), &password).unwrap();
    assert_eq!(a.me().unwrap().username, alice);

    assert_eq!(a.send_request(&bob).unwrap(), RequestOutcome::Pending);
    assert_eq!(b.requests().unwrap().incoming[0].username, alice);
    b.accept_request(&alice).unwrap();
    b.set_presence(&Presence::in_game("APQuest")).unwrap();
    let friends = a.friends().unwrap();
    assert_eq!(friends[0].username, bob);
    assert_eq!(friends[0].presence, Presence::in_game("APQuest"));

    a.remove_friend(&bob).unwrap();
    assert!(a.friends().unwrap().is_empty());
    a.logout().unwrap();
    b.logout().unwrap();
}
