//! Aides partagées par les tests d'intégration du hub.
//!
//! Par défaut, chaque test a sa base SQLite en mémoire. Si la variable
//! `RAFTUP_HUB_TEST_POSTGRES` contient l'URL d'un PostgreSQL de test
//! (`postgres://utilisateur@localhost:5432/base`), chaque test y reçoit à la place
//! son propre schéma, neuf, pour que les tests lancés en parallèle ne se gênent pas.

// Chaque fichier de test n'utilise pas toutes les aides.
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use hub::Store;

/// Variable d'environnement qui active les tests sur PostgreSQL.
pub const POSTGRES_ENV: &str = "RAFTUP_HUB_TEST_POSTGRES";

/// Base de test : SQLite en mémoire, ou un schéma neuf de PostgreSQL.
pub async fn test_store() -> Store {
    match std::env::var(POSTGRES_ENV) {
        Ok(url) if !url.trim().is_empty() => postgres_store(url.trim()).await,
        _ => Store::in_memory().await.unwrap(),
    }
}

/// Crée un schéma au nom unique, puis ouvre le hub dessus (`search_path`).
pub async fn postgres_store(url: &str) -> Store {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!(
        "raftup_test_{}_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        nanos % 1_000_000_000
    );
    sqlx::any::install_default_drivers();
    let admin = sqlx::AnyPool::connect(url).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    // `options=-c search_path=…` (encodé dans l'URL) : les tables du hub vont dans ce schéma.
    let separator = if url.contains('?') { '&' } else { '?' };
    let url = format!("{url}{separator}options=-c%20search_path%3D{schema}");
    Store::open(&url).await.unwrap()
}
