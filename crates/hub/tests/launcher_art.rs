//! De bout en bout : le client des illustrations du launcher
//! (`launcher_core::art`) contre le vrai routeur du hub, sur un port de ce PC, avec
//! un faux IGDB (ADR 0047). Vérifie que les deux côtés parlent le même JSON.
//!
//! Sens permis : le hub (AGPL) dépend du launcher (MIT / Apache) en test, jamais
//! l'inverse.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use hub::art::{ArtConfig, ArtKind, ArtSource, FakeArt, GameArt};
use hub::{AppState, HubConfig};
use launcher_core::art::{ArtClient, ArtError, ArtKind as LauncherKind, ArtRef};
use launcher_core::hub_client::HubUrl;

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 4, 2];

async fn serve(config: HubConfig) -> HubUrl {
    let state = AppState::new(common::test_store().await, config)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, hub::router(state)).await;
    });
    HubUrl::parse(&format!("http://127.0.0.1:{port}")).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn launcher_recoit_la_jaquette_du_hub() {
    let mut fake = FakeArt::default();
    fake.steam.insert(433340, 1234);
    fake.games.insert(
        1234,
        GameArt {
            igdb_id: 1234,
            cover: Some("co1abc".to_string()),
            hero: None,
            url: Some("https://www.igdb.com/games/slime-rancher".to_string()),
        },
    );
    fake.images
        .insert(("co1abc".to_string(), ArtKind::Cover), JPEG.to_vec());
    let hub_cache = tempfile::tempdir().unwrap();
    let mut config = HubConfig::for_tests();
    config.art = ArtConfig {
        cache_dir: hub_cache.path().join("art"),
        source: Some(Arc::new(fake) as Arc<dyn ArtSource>),
        ..ArtConfig::default()
    };
    let url = serve(config).await;

    // `spawn_blocking` : le client du launcher est bloquant (ureq).
    tokio::task::spawn_blocking(move || {
        let root = tempfile::tempdir().unwrap();
        let client = ArtClient::new(url, root.path());
        let image = client
            .fetch(ArtRef::Steam(433340), LauncherKind::Cover)
            .unwrap()
            .expect("jaquette");
        assert_eq!(image.bytes, JPEG);
        assert_eq!(image.attribution, "IGDB.com");
        assert_eq!(
            image.attribution_url,
            "https://www.igdb.com/games/slime-rancher"
        );
        // Pas d'image large : rien, sans erreur.
        assert_eq!(
            client.fetch(ArtRef::Steam(433340), LauncherKind::Hero),
            Ok(None)
        );
        // Jeu inconnu d'IGDB : rien, sans erreur.
        assert_eq!(client.fetch(ArtRef::Igdb(5), LauncherKind::Cover), Ok(None));
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn launcher_voit_un_hub_sans_cle() {
    let hub_cache = tempfile::tempdir().unwrap();
    let mut config = HubConfig::for_tests();
    config.art.cache_dir = hub_cache.path().join("art");
    let url = serve(config).await;
    tokio::task::spawn_blocking(move || {
        let root = tempfile::tempdir().unwrap();
        let client = ArtClient::new(url, root.path());
        assert_eq!(
            client.fetch(ArtRef::Steam(433340), LauncherKind::Cover),
            Err(ArtError::Disabled)
        );
    })
    .await
    .unwrap();
}
