//! Illustrations des jeux côté launcher (`launcher_core::art`, ADR 0047) contre un
//! **faux hub HTTP** : fiche, image, cache disque, hub en panne, jeu inconnu.
//!
//! Le vrai hub est en AGPL : `launcher-core` n'en dépend pas, même en test. Le test
//! de bout en bout avec le vrai routeur du hub est dans `crates/hub/tests/launcher_art.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use launcher_core::art::{ArtClient, ArtError, ArtKind, ArtRef};
use launcher_core::hub_client::HubUrl;
use launcher_core::profile::Profile;

const START: i64 = 1_800_000_000;
const DAY: i64 = 24 * 3600;
const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 9, 8, 7];

/// Réponse : code, type, corps.
type Reply = (u16, &'static str, Vec<u8>);

/// Faux hub : note les chemins demandés et répond avec `route`.
struct FakeHub {
    url: HubUrl,
    paths: Arc<Mutex<Vec<String>>>,
    down: Arc<AtomicBool>,
}

impl FakeHub {
    fn start(route: impl Fn(&str) -> Reply + Send + 'static) -> FakeHub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let paths = Arc::new(Mutex::new(Vec::new()));
        let down = Arc::new(AtomicBool::new(false));
        let (paths_thread, down_thread) = (Arc::clone(&paths), Arc::clone(&down));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let Some(path) = read_path(&stream) else {
                    continue;
                };
                paths_thread.lock().unwrap().push(path.clone());
                let reply = if down_thread.load(Ordering::SeqCst) {
                    json(502, r#"{"error":"art_unavailable","message":"panne"}"#)
                } else {
                    route(&path)
                };
                write_reply(stream, reply);
            }
        });
        FakeHub {
            url: HubUrl::parse(&format!("http://127.0.0.1:{port}")).unwrap(),
            paths,
            down,
        }
    }

    fn count(&self) -> usize {
        self.paths.lock().unwrap().len()
    }
}

fn read_path(stream: &TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let path = line.split_whitespace().nth(1)?.to_string();
    // Saute les en-têtes (un GET n'a pas de corps).
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        if header.trim_end().is_empty() {
            break;
        }
    }
    Some(path)
}

fn write_reply(mut stream: TcpStream, (status, kind, body): Reply) {
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nContent-Type: {kind}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
}

fn json(status: u16, body: &str) -> Reply {
    (status, "application/json", body.as_bytes().to_vec())
}

const FICHE: &str = r#"{"igdb_id":1234,"cover":"co1abc","hero":"ar2xyz","attribution":"IGDB.com","attribution_url":"https://www.igdb.com/games/slime-rancher"}"#;

/// Hub qui connaît Slime Rancher (Steam 433340) et ses deux images.
fn slime_hub() -> FakeHub {
    FakeHub::start(|path| match path {
        "/v1/art/steam/433340" | "/v1/art/igdb/1234" => json(200, FICHE),
        "/v1/art/images/co1abc/cover" | "/v1/art/images/ar2xyz/hero" => {
            (200, "image/jpeg", JPEG.to_vec())
        }
        _ => json(404, r#"{"error":"art_not_found","message":"aucune"}"#),
    })
}

struct Setup {
    client: ArtClient,
    clock: Arc<AtomicI64>,
    _root: tempfile::TempDir,
}

fn client(hub: &FakeHub) -> Setup {
    let root = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicI64::new(START));
    let reader = Arc::clone(&clock);
    let client = ArtClient::new(hub.url.clone(), root.path())
        .with_clock(Arc::new(move || reader.load(Ordering::SeqCst)));
    Setup {
        client,
        clock,
        _root: root,
    }
}

#[test]
fn reference_tiree_du_profil() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../profiles/slime-rancher.toml"
    ))
    .unwrap();
    let profile = Profile::parse("slime-rancher.toml", &text).unwrap();
    assert_eq!(ArtRef::of(&profile), Some(ArtRef::Steam(433340)));

    // `igdb_id` passe avant l'appid Steam, sans changer `format_version`.
    let text = text.replace(
        "status = \"available\"",
        "status = \"available\"\nigdb_id = 9999",
    );
    let profile = Profile::parse("slime-rancher.toml", &text).unwrap();
    assert_eq!(profile.format_version, 1);
    assert_eq!(ArtRef::of(&profile), Some(ArtRef::Igdb(9999)));
}

#[test]
fn jaquette_puis_cache() {
    let hub = slime_hub();
    let setup = client(&hub);
    let image = setup
        .client
        .fetch(ArtRef::Steam(433340), ArtKind::Cover)
        .unwrap()
        .expect("jaquette");
    assert_eq!(image.bytes, JPEG);
    assert_eq!(image.attribution, "IGDB.com");
    assert_eq!(
        image.attribution_url,
        "https://www.igdb.com/games/slime-rancher"
    );
    assert_eq!(
        *hub.paths.lock().unwrap(),
        ["/v1/art/steam/433340", "/v1/art/images/co1abc/cover"]
    );
    // 29 jours plus tard : tout vient du disque.
    setup.clock.fetch_add(29 * DAY, Ordering::SeqCst);
    let again = setup
        .client
        .fetch(ArtRef::Steam(433340), ArtKind::Cover)
        .unwrap();
    assert_eq!(again, Some(image));
    assert_eq!(hub.count(), 2);
    // L'image large est une autre entrée.
    let hero = setup
        .client
        .fetch(ArtRef::Steam(433340), ArtKind::Hero)
        .unwrap();
    assert!(hero.is_some());
    assert_eq!(hub.count(), 3);
}

#[test]
fn hub_en_panne_cache_perime_servi() {
    let hub = slime_hub();
    let setup = client(&hub);
    setup
        .client
        .fetch(ArtRef::Igdb(1234), ArtKind::Cover)
        .unwrap()
        .expect("jaquette");
    setup.clock.fetch_add(40 * DAY, Ordering::SeqCst);
    hub.down.store(true, Ordering::SeqCst);
    let stale = setup
        .client
        .fetch(ArtRef::Igdb(1234), ArtKind::Cover)
        .unwrap();
    assert_eq!(stale.map(|i| i.bytes), Some(JPEG.to_vec()));
    // Le hub a bien été interrogé (fiche et image périmées), puis on a pris le cache.
    assert_eq!(hub.count(), 4);
    // Rien en cache pour l'image large : erreur.
    assert!(setup
        .client
        .fetch(ArtRef::Igdb(1234), ArtKind::Hero)
        .is_err());
}

#[test]
fn hub_injoignable_sans_cache() {
    // Port fermé : personne n'écoute.
    let root = tempfile::tempdir().unwrap();
    let client = ArtClient::new(HubUrl::parse("http://127.0.0.1:1").unwrap(), root.path());
    let result = client.fetch(ArtRef::Steam(433340), ArtKind::Cover);
    assert!(matches!(result, Err(ArtError::Network(_))), "{result:?}");
}

#[test]
fn jeu_inconnu_retenu_sept_jours() {
    let hub = slime_hub();
    let setup = client(&hub);
    assert_eq!(
        setup.client.fetch(ArtRef::Steam(1), ArtKind::Cover),
        Ok(None)
    );
    setup.clock.fetch_add(6 * DAY, Ordering::SeqCst);
    assert_eq!(
        setup.client.fetch(ArtRef::Steam(1), ArtKind::Cover),
        Ok(None)
    );
    assert_eq!(hub.count(), 1);
    setup.clock.fetch_add(2 * DAY, Ordering::SeqCst);
    setup
        .client
        .fetch(ArtRef::Steam(1), ArtKind::Cover)
        .unwrap();
    assert_eq!(hub.count(), 2);
}

#[test]
fn hub_sans_illustrations() {
    let hub = FakeHub::start(|_| {
        json(
            503,
            r#"{"error":"art_disabled","message":"illustrations non configurées"}"#,
        )
    });
    let setup = client(&hub);
    assert_eq!(
        setup.client.fetch(ArtRef::Steam(433340), ArtKind::Cover),
        Err(ArtError::Disabled)
    );
    // Hub plus ancien, qui ne connaît pas la route : même chose.
    let old = FakeHub::start(|_| (404, "text/plain", Vec::new()));
    let setup = client(&old);
    assert_eq!(
        setup.client.fetch(ArtRef::Steam(433340), ArtKind::Cover),
        Err(ArtError::Disabled)
    );
}

#[test]
fn reponses_douteuses_refusees() {
    let hub = FakeHub::start(|path| match path {
        // Identifiant d'image qui sortirait du dossier du cache : ignoré.
        "/v1/art/igdb/1" => json(
            200,
            r#"{"igdb_id":1,"cover":"../../evil","hero":"ok1","attribution":"IGDB.com","attribution_url":"https://www.igdb.com"}"#,
        ),
        // Pas un JPEG.
        "/v1/art/images/ok1/hero" => (200, "image/jpeg", b"<html>".to_vec()),
        _ => json(404, r#"{"error":"art_not_found","message":""}"#),
    });
    let setup = client(&hub);
    assert_eq!(
        setup.client.fetch(ArtRef::Igdb(1), ArtKind::Cover),
        Ok(None)
    );
    assert!(matches!(
        setup.client.fetch(ArtRef::Igdb(1), ArtKind::Hero),
        Err(ArtError::Unexpected(_))
    ));
    // Seuls la fiche et l'image refusée ont été demandées ; rien de rangé pour l'image.
    assert_eq!(hub.count(), 2);
    let images = setup.client.cache_dir().join("images");
    assert!(!images.join("ok1-hero.jpg").exists());
}
