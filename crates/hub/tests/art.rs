//! Illustrations des jeux (ADR 0047) : routes `/v1/art/…`, cache disque, pannes
//! d'IGDB, limites. IGDB est remplacé par [`FakeArt`] ; l'horloge est fausse.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use hub::art::{ArtConfig, ArtKind, ArtSource, FakeArt, GameArt};
use hub::{AppState, HubConfig};
use tower::ServiceExt;

use common::{call, error_code, test_store};

const START: i64 = 1_800_000_000;
const DAY: i64 = 24 * 3600;
/// Un « JPEG » : l'en-tête suffit aux contrôles du hub.
const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4];

struct Hub {
    app: Router,
    fake: Arc<FakeArt>,
    clock: Arc<AtomicI64>,
    cache: tempfile::TempDir,
}

impl Hub {
    fn advance(&self, secs: i64) {
        self.clock.fetch_add(secs, Ordering::SeqCst);
    }
}

/// Fausse source : Slime Rancher (Steam 433340 → IGDB 1234) avec jaquette et image
/// large ; un jeu IGDB 77 sans image.
fn fake() -> FakeArt {
    let mut fake = FakeArt::default();
    fake.steam.insert(433340, 1234);
    fake.games.insert(
        1234,
        GameArt {
            igdb_id: 1234,
            cover: Some("co1abc".to_string()),
            hero: Some("ar2xyz".to_string()),
            url: Some("https://www.igdb.com/games/slime-rancher".to_string()),
        },
    );
    fake.games.insert(
        77,
        GameArt {
            igdb_id: 77,
            cover: None,
            hero: None,
            url: None,
        },
    );
    fake.images
        .insert(("co1abc".to_string(), ArtKind::Cover), JPEG.to_vec());
    fake.images
        .insert(("ar2xyz".to_string(), ArtKind::Hero), JPEG.to_vec());
    fake.images
        .insert(("pasjpeg".to_string(), ArtKind::Cover), b"<html>".to_vec());
    fake
}

async fn start_with(fake: FakeArt, tune: impl FnOnce(&mut ArtConfig)) -> Hub {
    let fake = Arc::new(fake);
    let clock = Arc::new(AtomicI64::new(START));
    let cache = tempfile::tempdir().unwrap();
    let mut config = HubConfig::for_tests();
    let reader = Arc::clone(&clock);
    config.art = ArtConfig {
        cache_dir: cache.path().join("art"),
        // `as Arc<dyn ArtSource>` : on range la fausse source derrière le trait.
        source: Some(Arc::clone(&fake) as Arc<dyn ArtSource>),
        clock: Arc::new(move || reader.load(Ordering::SeqCst)),
        ..ArtConfig::default()
    };
    tune(&mut config.art);
    let state = AppState::new(test_store().await, config).await.unwrap();
    Hub {
        app: hub::router(state),
        fake,
        clock,
        cache,
    }
}

async fn start() -> Hub {
    start_with(fake(), |_| {}).await
}

async fn get(app: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
    call(app, Method::GET, uri, None, None).await
}

/// Image : code, type et octets.
async fn image(app: &Router, uri: &str) -> (StatusCode, Option<String>, Vec<u8>) {
    let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let kind = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap()
        .to_vec();
    (status, kind, bytes)
}

#[tokio::test]
async fn sans_cle_illustrations_desactivees() {
    // Ni source fixée ni application Twitch dans les réglages.
    let mut config = HubConfig::for_tests();
    let cache = tempfile::tempdir().unwrap();
    config.art.cache_dir = cache.path().join("art");
    let app = common::app_with(config).await;
    let (status, body) = get(&app, "/v1/art/steam/433340").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(&body), "art_disabled");
    let (status, _, _) = image(&app, "/v1/art/images/co1abc/cover").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn fiche_par_steam_puis_cache() {
    let hub = start().await;
    let (status, body) = get(&hub.app, "/v1/art/steam/433340").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["igdb_id"], 1234);
    assert_eq!(body["cover"], "co1abc");
    assert_eq!(body["hero"], "ar2xyz");
    assert_eq!(body["attribution"], "IGDB.com");
    assert_eq!(
        body["attribution_url"],
        "https://www.igdb.com/games/slime-rancher"
    );
    // Deux appels : appid → jeu, puis fiche du jeu.
    assert_eq!(hub.fake.calls(), 2);

    // Deuxième demande, 29 jours plus tard : tout vient du cache.
    hub.advance(29 * DAY);
    let (status, again) = get(&hub.app, "/v1/art/steam/433340").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again, body);
    assert_eq!(hub.fake.calls(), 2);

    // Au-delà de 30 jours, le hub redemande à IGDB.
    hub.advance(2 * DAY);
    get(&hub.app, "/v1/art/steam/433340").await;
    assert_eq!(hub.fake.calls(), 4);
}

#[tokio::test]
async fn fiche_par_identifiant_igdb() {
    let hub = start().await;
    let (status, body) = get(&hub.app, "/v1/art/igdb/1234").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["cover"], "co1abc");
    // Jeu sans image : fiche quand même, avec le lien d'IGDB par défaut.
    let (status, body) = get(&hub.app, "/v1/art/igdb/77").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["cover"].is_null());
    assert_eq!(body["attribution_url"], "https://www.igdb.com");
}

#[tokio::test]
async fn jeu_inconnu_retenu_sept_jours() {
    let hub = start().await;
    let (status, body) = get(&hub.app, "/v1/art/steam/999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "art_not_found");
    assert_eq!(hub.fake.calls(), 1);
    // Pas redemandé pendant 7 jours…
    hub.advance(6 * DAY);
    get(&hub.app, "/v1/art/steam/999").await;
    assert_eq!(hub.fake.calls(), 1);
    // … puis redemandé.
    hub.advance(2 * DAY);
    get(&hub.app, "/v1/art/steam/999").await;
    assert_eq!(hub.fake.calls(), 2);
    // Jeu IGDB inconnu.
    let (status, _) = get(&hub.app, "/v1/art/igdb/5").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn identifiants_refuses() {
    let hub = start().await;
    for uri in [
        "/v1/art/steam/abc",
        "/v1/art/steam/-1",
        "/v1/art/steam/99999999999",
        "/v1/art/igdb/1.5",
    ] {
        let (status, body) = get(&hub.app, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(error_code(&body), "bad_request");
    }
    for uri in [
        "/v1/art/images/CO1ABC/cover",
        "/v1/art/images/co1abc/poster",
        "/v1/art/images/..%2Fsecret/cover",
    ] {
        let (status, _, _) = image(&hub.app, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
    }
    assert_eq!(hub.fake.calls(), 0);
}

#[tokio::test]
async fn image_servie_puis_cache() {
    let hub = start().await;
    let (status, kind, bytes) = image(&hub.app, "/v1/art/images/co1abc/cover").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kind.as_deref(), Some("image/jpeg"));
    assert_eq!(bytes, JPEG);
    assert!(hub
        .cache
        .path()
        .join("art/images/co1abc-cover.jpg")
        .is_file());
    image(&hub.app, "/v1/art/images/co1abc/cover").await;
    assert_eq!(hub.fake.calls(), 1);
    // La même image dans l'autre taille est une autre entrée.
    let (status, _, _) = image(&hub.app, "/v1/art/images/co1abc/hero").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn fichier_qui_n_est_pas_un_jpeg_refuse() {
    let hub = start().await;
    let (status, _, _) = image(&hub.app, "/v1/art/images/pasjpeg/cover").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(!hub
        .cache
        .path()
        .join("art/images/pasjpeg-cover.jpg")
        .exists());
}

#[tokio::test]
async fn panne_d_igdb_cache_perime_servi() {
    let hub = start().await;
    get(&hub.app, "/v1/art/steam/433340").await;
    image(&hub.app, "/v1/art/images/co1abc/cover").await;
    hub.advance(40 * DAY);
    hub.fake.set_down(true);
    // Périmé, IGDB en panne : l'ancienne entrée sert encore.
    let (status, body) = get(&hub.app, "/v1/art/steam/433340").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["cover"], "co1abc");
    let (status, _, bytes) = image(&hub.app, "/v1/art/images/co1abc/cover").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, JPEG);
    // Rien en cache : 502.
    let (status, body) = get(&hub.app, "/v1/art/igdb/77").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(error_code(&body), "art_unavailable");
}

#[tokio::test]
async fn demandes_hors_cache_limitees() {
    let hub = start_with(fake(), |art| art.misses_per_minute = 3).await;
    // Fiche Slime Rancher : 2 demandes hors cache.
    assert_eq!(
        get(&hub.app, "/v1/art/steam/433340").await.0,
        StatusCode::OK
    );
    // 3e : l'image.
    assert_eq!(
        image(&hub.app, "/v1/art/images/co1abc/cover").await.0,
        StatusCode::OK
    );
    // 4e : refusée.
    let (status, body) = get(&hub.app, "/v1/art/igdb/77").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error_code(&body), "art_rate_limited");
    // Ce qui est en cache reste servi.
    assert_eq!(
        get(&hub.app, "/v1/art/steam/433340").await.0,
        StatusCode::OK
    );
    assert_eq!(
        image(&hub.app, "/v1/art/images/co1abc/cover").await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn cache_plafonne_les_plus_anciens_partent() {
    let mut source = fake();
    let big = |n: u8| {
        let mut bytes = JPEG.to_vec();
        bytes.resize(1000, n);
        bytes
    };
    for (id, n) in [("img1", 1), ("img2", 2), ("img3", 3)] {
        source
            .images
            .insert((id.to_string(), ArtKind::Cover), big(n));
    }
    let hub = start_with(source, |art| art.max_cache_bytes = 2500).await;
    let dir = hub.cache.path().join("art/images");
    for id in ["img1", "img2", "img3"] {
        let (status, _, _) = image(&hub.app, &format!("/v1/art/images/{id}/cover")).await;
        assert_eq!(status, StatusCode::OK);
        hub.advance(60);
    }
    // 3000 octets > 2500 : la plus ancienne est retirée.
    assert!(!dir.join("img1-cover.jpg").exists());
    assert!(dir.join("img2-cover.jpg").exists());
    assert!(dir.join("img3-cover.jpg").exists());
}

#[tokio::test]
async fn secret_twitch_jamais_reaffiche() {
    let store = test_store().await;
    let state = AppState::new(store, HubConfig::for_tests()).await.unwrap();
    state
        .update_setting(None, "igdb_client_id", Some("abc123"))
        .await
        .unwrap();
    state
        .update_setting(None, "igdb_client_secret", Some("tressecret42"))
        .await
        .unwrap();
    let settings = state.settings().await.unwrap();
    assert!(settings.igdb_credentials().is_some());
    let public = state.public_settings().await.unwrap();
    let shown = serde_json::to_string(&public).unwrap();
    assert!(!shown.contains("tressecret42"));
    assert!(shown.contains("abc123"));
    assert!(!format!("{settings:?}").contains("tressecret42"));
    // Valeur refusée : espace, caractère spécial.
    assert!(state
        .update_setting(None, "igdb_client_secret", Some("a&b"))
        .await
        .is_err());
}

/// Test réel contre IGDB (réseau, application Twitch) :
/// `RAFTUP_IGDB_CLIENT_ID=… RAFTUP_IGDB_CLIENT_SECRET=… cargo test -p hub --test art -- --ignored`
/// Vérifie la requête Steam (`external_game_source = 1`) avec Slime Rancher (433340)
/// et la lecture d'une vraie jaquette.
#[test]
#[ignore = "réseau : demande une application Twitch (RAFTUP_IGDB_CLIENT_ID / _SECRET)"]
fn reel_igdb_slime_rancher() {
    use hub::art::{IgdbCredentials, IgdbSource};
    let (Ok(client_id), Ok(client_secret)) = (
        std::env::var("RAFTUP_IGDB_CLIENT_ID"),
        std::env::var("RAFTUP_IGDB_CLIENT_SECRET"),
    ) else {
        panic!("RAFTUP_IGDB_CLIENT_ID et RAFTUP_IGDB_CLIENT_SECRET manquent");
    };
    let source = IgdbSource::new(IgdbCredentials {
        client_id,
        client_secret,
    });
    let igdb_id = source
        .steam_game(433340)
        .unwrap()
        .expect("Slime Rancher sur IGDB");
    let art = source.game_art(igdb_id).unwrap().expect("fiche du jeu");
    println!("{art:?}");
    let cover = art.cover.expect("jaquette");
    let bytes = source.image(&cover, ArtKind::Cover).unwrap();
    assert!(hub::art::looks_like_jpeg(&bytes));
    if let Some(hero) = art.hero {
        assert!(hub::art::looks_like_jpeg(
            &source.image(&hero, ArtKind::Hero).unwrap()
        ));
    }
}
