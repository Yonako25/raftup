//! Binaire `raftup-hub` : sert l'API du hub en HTTP.
//!
//! ```text
//! raftup-hub [--db sqlite://raftup-hub.db] [--listen 127.0.0.1:8787]
//! ```
//!
//! Le hub parle HTTP en clair : en production, il se place derrière un proxy
//! qui fait le TLS (ADR 0033). Pas de déploiement pour l'instant.

use anyhow::{bail, Context};
use hub::{AppState, HubConfig, Store};

struct Args {
    db: String,
    listen: String,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = Args {
        db: "sqlite://raftup-hub.db".to_string(),
        listen: "127.0.0.1:8787".to_string(),
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--db" => args.db = iter.next().context("--db attend une URL")?,
            "--listen" => args.listen = iter.next().context("--listen attend une adresse")?,
            "--help" | "-h" => {
                println!("raftup-hub [--db sqlite://raftup-hub.db] [--listen 127.0.0.1:8787]");
                std::process::exit(0);
            }
            other => bail!("argument inconnu : {other}"),
        }
    }
    Ok(args)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = parse_args()?;

    let store = Store::open(&args.db)
        .await
        .with_context(|| format!("ouverture de la base {}", args.db))?;
    let state = AppState::new(store.clone(), HubConfig::default()).await?;

    // Ménage des sessions expirées, une fois par heure, dans une tâche à part.
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            if let Err(e) = store.purge_expired_sessions(hub::now_unix()).await {
                tracing::warn!(error = %e, "ménage des sessions expirées raté");
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(&args.listen)
        .await
        .with_context(|| format!("écoute sur {}", args.listen))?;
    tracing::info!(adresse = %args.listen, "raftup-hub démarré");

    axum::serve(listener, hub::router(state))
        .with_graceful_shutdown(async {
            // Ctrl+C arrête le serveur proprement ; une erreur ici l'arrête aussi.
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
