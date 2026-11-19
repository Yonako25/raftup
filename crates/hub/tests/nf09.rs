//! NF09 : le mot de passe n'apparaît jamais en clair, ni dans la base, ni dans
//! le journal. On inscrit un compte sur une vraie base SQLite (fichier), on se
//! connecte, on rate une connexion, puis on cherche le mot de passe partout.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used)]

use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use hub::{AppState, HubConfig, Store};
use serde_json::json;
use tower::ServiceExt;

const PASSWORD: &str = "Secret-NF09-tres-reconnaissable";
const WRONG: &str = "Mauvais-NF09-tres-reconnaissable";

/// Tampon partagé où le journal de test écrit.
#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

async fn post(app: &axum::Router, uri: &str, body: serde_json::Value) -> StatusCode {
    let request = Request::post(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    app.clone().oneshot(request).await.unwrap().status()
}

// Exécuteur à un seul thread : le journal capturé ci-dessous est celui du thread
// du test, où tournent les routes.
#[tokio::test(flavor = "current_thread")]
async fn mot_de_passe_jamais_en_clair() {
    let buffer = Buffer::default();
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    // `set_default` : ce journal ne vaut que pour ce thread, jusqu'à la fin du test.
    let _guard = tracing::subscriber::set_default(subscriber);

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("hub.db");
    let url = format!("sqlite://{}", db.display());
    let store = Store::open(&url).await.unwrap();
    let state = AppState::new(store.clone(), HubConfig::for_tests())
        .await
        .unwrap();
    let app = hub::router(state);

    let account = json!({ "email": "nf09@example.org", "username": "Nina", "password": PASSWORD });
    assert_eq!(
        post(&app, "/v1/accounts", account).await,
        StatusCode::CREATED
    );
    let good = json!({ "email": "nf09@example.org", "password": PASSWORD });
    assert_eq!(post(&app, "/v1/sessions", good).await, StatusCode::OK);
    let bad = json!({ "email": "nf09@example.org", "password": WRONG });
    assert_eq!(
        post(&app, "/v1/sessions", bad).await,
        StatusCode::UNAUTHORIZED
    );

    // Le hash stocké est un argon2id, pas le mot de passe.
    let (_, phc) = store
        .email_identity("nf09@example.org")
        .await
        .unwrap()
        .unwrap();
    assert!(phc.starts_with("$argon2id$"), "{phc}");
    assert!(!phc.contains(PASSWORD));

    // Ni la base ni son journal WAL ne contiennent le mot de passe.
    drop(app);
    drop(store);
    let mut found_files = 0;
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        found_files += 1;
        for secret in [PASSWORD, WRONG] {
            assert!(
                !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                "mot de passe trouvé dans un fichier de la base"
            );
        }
    }
    assert!(found_files >= 1);

    // Le journal a bien écrit quelque chose, mais jamais le mot de passe ni l'adresse.
    let logs = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("compte créé"), "{logs}");
    assert!(!logs.contains(PASSWORD));
    assert!(!logs.contains(WRONG));
    assert!(!logs.contains("nf09@example.org"));
}
