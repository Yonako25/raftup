//! Bout en bout : le **vrai client du launcher** (`launcher_core::hub_client`) contre le
//! **vrai hub** (routeur HTTP et serveurs de jeu) pour les parties hébergées
//! (ADR 0052). Tout reste sur `127.0.0.1` : aucun réseau extérieur.
//!
//! Sens des dépendances : le hub (AGPL) dépend de `launcher-core` (MIT / Apache) en
//! test seulement ; l'inverse est interdit (ADR 0003, test `licence.rs`).
//!
//! Le client du launcher est **bloquant** (ureq) : il tourne dans `spawn_blocking`,
//! jamais sur un thread de tokio qui sert aussi le hub.
//!
//! Plage de ports propre à ce fichier : 47400 et plus (voir `games.rs`).

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use ap_protocol::client::{ClientPacket, LocationChecks};
use client_core::{Client, ConnectOptions};
use hub::games::GamesConfig;
use hub::{AppState, EnvSettings, HubConfig};
use launcher_core::hub_client::{
    ErrorCode, GameUpload, HubClient, HubClientError, HubUrl, Password,
};
use launcher_core::invite::parse_invite;
use multidata::raftup::{seed_sha256, SaveFile};

mod common;

use common::*;

fn seed_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago")
}

/// Lance le vrai hub sur un port libre de `127.0.0.1`, avec ses parties dans `dir`
/// (dix ports à partir de `first_port`, écoute sur `127.0.0.1` seulement).
async fn serve(dir: &Path, first_port: u16, hosting: bool) -> (HubUrl, AppState) {
    serve_with(dir, first_port, hosting, GamesConfig::default()).await
}

/// Comme [`serve`], avec d'autres réglages des parties (délais de veille).
async fn serve_with(
    dir: &Path,
    first_port: u16,
    hosting: bool,
    games: GamesConfig,
) -> (HubUrl, AppState) {
    let config = HubConfig {
        settings_env: EnvSettings::from_lookup(|name| {
            (name == "RAFTUP_HUB_HOSTING").then(|| hosting.to_string())
        }),
        games: GamesConfig {
            data_dir: dir.to_path_buf(),
            first_port,
            last_port: first_port + 9,
            listen_host: "127.0.0.1".to_string(),
            ..games
        },
        ..HubConfig::for_tests()
    };
    let state = AppState::new(test_store().await, config).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = hub::router(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let url = HubUrl::parse(&format!("http://127.0.0.1:{port}")).unwrap();
    (url, state)
}

/// Crée un compte avec le client du launcher, puis s'y connecte.
fn account(url: &HubUrl, username: &str) -> HubClient {
    let mut client = HubClient::new(url.clone());
    let email = format!("{}@example.org", username.to_lowercase());
    let password = Password::new("un mot de passe solide 42");
    client.register(&email, &password, username, None).unwrap();
    client.login(&email, &password).unwrap();
    client
}

/// Le code d'erreur du hub, ou panique si l'erreur n'en a pas.
fn code(error: HubClientError) -> ErrorCode {
    error.code().cloned().unwrap_or_else(|| panic!("{error}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn launcher_heberge_suit_exporte_et_supprime() {
    let dir = tempfile::tempdir().unwrap();
    let (url, _state) = serve(dir.path(), 47400, true).await;
    // `Handle` : permet au thread bloquant de lancer à son tour du code async
    // (le client Archipelago `client-core`) sur le runtime du test.
    let runtime = tokio::runtime::Handle::current();

    tokio::task::spawn_blocking(move || {
        let host = account(&url, "Hote");
        let other = account(&url, "Autre");
        assert!(host.hub_info().unwrap().hosting);

        // Envoi de la seed, comme « Créer une partie » en mode Rafthub.
        let upload =
            GameUpload::from_files("Partie du jeudi", Some("secret"), &seed_path(), None).unwrap();
        let created = host.create_game(&upload).unwrap();
        let game = created.game.clone();
        assert!(game.is_active());
        assert!(game.pinned().is_some());
        assert!((47400..=47409).contains(&game.port));
        assert_eq!(game.address, format!("127.0.0.1:{}", game.port));
        assert!(game.has_password);
        assert!(!created.resumed);
        assert!(created.admin_password.len() >= 16);

        // L'invitation d'un slot, relue comme par « Rejoindre » : `wss` épinglé.
        let link = game
            .invite_link(Some("Alice"), Some("ChecksFinder"))
            .unwrap();
        let invite = parse_invite(&link).unwrap();
        assert_eq!(invite.address_for_game(), game.address);
        let mut options = ConnectOptions::new(&invite.address.to_url(), "Alice", "ChecksFinder");
        options.pinned = invite.fingerprint;
        options.password = Some("secret".to_string());
        runtime.block_on(async {
            let (mut alice, session) = Client::connect(&options).await.unwrap();
            assert!(session.url.starts_with("wss://"), "{}", session.url);
            alice
                .send(&[ClientPacket::LocationChecks(LocationChecks {
                    locations: vec![81001],
                })])
                .await
                .unwrap();
            // La première réponse arrive après l'écriture de la sauvegarde (NF07).
            tokio::time::timeout(Duration::from_secs(5), alice.recv())
                .await
                .unwrap()
                .unwrap();
            alice.close().await.unwrap();
        });

        // Suivi, comme l'écran de la partie hébergée.
        let status = host.game_status(&game.id).unwrap();
        assert!(status.is_host);
        assert_eq!(
            status.admin_password.as_deref(),
            Some(created.admin_password.as_str())
        );
        let alice = status.slots.iter().find(|s| s.name == "Alice").unwrap();
        assert_eq!(alice.checked, 1);
        let mine = host.games().unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].id, game.id);
        assert!(other.games().unwrap().is_empty());

        // Un autre compte qui connaît l'identifiant voit l'état, sans le mot de passe
        // d'admin, et ne peut ni exporter ni supprimer.
        let seen = other.game_status(&game.id).unwrap();
        assert!(!seen.is_host);
        assert_eq!(seen.admin_password, None);
        assert_eq!(
            code(other.export_game(&game.id).unwrap_err()),
            ErrorCode::NotGameHost
        );
        assert_eq!(
            code(other.delete_game(&game.id).unwrap_err()),
            ErrorCode::NotGameHost
        );

        // Export par l'hôte : la seed telle quelle et la sauvegarde avec le lieu visité.
        let export = host.export_game(&game.id).unwrap();
        assert_eq!(export.seed, std::fs::read(seed_path()).unwrap());
        let save = export.save.clone().unwrap();
        let save_path = dir.path().join("reprise.raftup");
        std::fs::write(&save_path, &save).unwrap();
        let (_, snapshot) = SaveFile::open(&save_path, &seed_sha256(&export.seed)).unwrap();
        let checks = snapshot
            .location_checks
            .iter()
            .find(|entry| entry.slot == 1)
            .unwrap();
        assert_eq!(checks.locations, vec![81001]);

        // Sens inverse : la partie commencée repart sur le hub avec sa sauvegarde.
        let again = GameUpload {
            name: "Suite".to_string(),
            password: None,
            seed: export.seed.clone(),
            save: Some(save),
        };
        let resumed = other.create_game(&again).unwrap();
        assert!(resumed.resumed);
        assert_ne!(resumed.game.port, game.port);

        // Suppression par l'hôte : la partie disparaît du hub.
        host.delete_game(&game.id).unwrap();
        assert_eq!(
            code(host.game_status(&game.id).unwrap_err()),
            ErrorCode::GameNotFound
        );
        assert!(host.games().unwrap().is_empty());
        // Un identifiant mal formé ne part même pas vers le hub.
        assert!(matches!(
            host.game_status("../x"),
            Err(HubClientError::BadGameId)
        ));
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn launcher_voit_les_refus_du_hub() {
    let dir = tempfile::tempdir().unwrap();
    let upload = GameUpload::from_files("P", None, &seed_path(), None).unwrap();

    // Hébergement coupé : `hosting` faux, et l'envoi refusé avec son code.
    let (url, _state) = serve(&dir.path().join("coupe"), 47420, false).await;
    let blocked = upload.clone();
    tokio::task::spawn_blocking(move || {
        let host = account(&url, "Hote");
        assert!(!host.hub_info().unwrap().hosting);
        assert_eq!(
            code(host.create_game(&blocked).unwrap_err()),
            ErrorCode::HostingDisabled
        );
    })
    .await
    .unwrap();

    // Hébergement allumé : deux parties par compte, la troisième est refusée.
    let (url, _state) = serve(&dir.path().join("ouvert"), 47440, true).await;
    tokio::task::spawn_blocking(move || {
        let host = account(&url, "Hote");
        host.create_game(&upload).unwrap();
        host.create_game(&upload).unwrap();
        assert_eq!(
            code(host.create_game(&upload).unwrap_err()),
            ErrorCode::GameLimitReached
        );
        // Une sauvegarde qui ne va pas avec la seed : refusée avant de démarrer.
        let foreign = GameUpload {
            save: Some(b"pas une sauvegarde".to_vec()),
            ..upload
        };
        let other = account(&url, "Autre");
        assert_eq!(
            code(other.create_game(&foreign).unwrap_err()),
            ErrorCode::InvalidSave
        );
        assert!(other.games().unwrap().is_empty());
    })
    .await
    .unwrap();
}

/// Veille vue du launcher : `state` passe à `sleeping`, `wake_game` la réveille, et
/// la dernière activité est rendue après qu'un joueur s'est connecté.
#[tokio::test(flavor = "multi_thread")]
async fn launcher_voit_la_veille_et_reveille_la_partie() {
    let dir = tempfile::tempdir().unwrap();
    let games = GamesConfig {
        idle_after: Duration::from_millis(300),
        wake_grace: Duration::from_millis(300),
        activity_check: Duration::from_millis(50),
        ..GamesConfig::default()
    };
    let (url, _state) = serve_with(dir.path(), 47460, true, games).await;
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let host = account(&url, "Hote");
        let upload = GameUpload::from_files("Veille", None, &seed_path(), None).unwrap();
        let game = host.create_game(&upload).unwrap().game;
        assert_eq!(game.last_activity_at, None);

        // Un joueur passe un moment, puis s'en va.
        let options = ConnectOptions::new(&game.address, "Alice", "ChecksFinder");
        runtime.block_on(async {
            let (alice, _) = Client::connect(&options).await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
            alice.close().await.unwrap();
        });

        // La partie s'endort ; le lien d'invitation reste bon (même empreinte).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let asleep = loop {
            let status = host.game_status(&game.id).unwrap();
            if status.game.is_sleeping() {
                break status;
            }
            assert!(std::time::Instant::now() < deadline, "{status:?}");
            std::thread::sleep(Duration::from_millis(25));
        };
        assert!(asleep.slots.is_empty());
        assert_eq!(asleep.game.fingerprint, game.fingerprint);
        assert!(asleep.game.last_activity_at.is_some());
        assert!(host.games().unwrap()[0].is_sleeping());

        // « Réveiller » dans le launcher.
        let woken = host.wake_game(&game.id).unwrap();
        assert!(woken.is_active());
        assert!(!host.game_status(&game.id).unwrap().slots.is_empty());
    })
    .await
    .unwrap();
}
