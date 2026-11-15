//! `raftup-server` : sert une seed `.archipelago` aux clients compatibles avec Archipelago.
//!
//! ```text
//! raftup-server <seed.archipelago> --no-tls [--host 0.0.0.0] [--port 38281] [--password X] [--verbose]
//! ```
//!
//! Pour l'instant, seul le WebSocket en clair (`ws://`) est disponible : le
//! chiffrement (`wss://`) arrive à l'étape 8 de J1. Comme la spec impose wss
//! par défaut (NF06), il faut demander le clair explicitement avec `--no-tls`.

use std::path::PathBuf;

use anyhow::{bail, Context};
use multidata::{load_archipelago, ArchiveLimits};
use server::{Room, Server, DEFAULT_PORT};
use tracing::{info, Level};

const USAGE: &str = "usage : raftup-server <seed.archipelago> --no-tls \
[--host 0.0.0.0] [--port 38281] [--password MOT_DE_PASSE] [--verbose]";

/// Options de la ligne de commande.
struct Options {
    seed: PathBuf,
    host: String,
    port: u16,
    password: Option<String>,
    no_tls: bool,
    verbose: bool,
}

fn parse_args() -> anyhow::Result<Options> {
    let mut seed = None;
    let mut options = Options {
        seed: PathBuf::new(),
        // Toutes les interfaces, comme le serveur officiel.
        host: "0.0.0.0".to_string(),
        port: DEFAULT_PORT,
        password: None,
        no_tls: false,
        verbose: false,
    };
    // `skip(1)` : le premier argument est le nom du programme.
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => options.host = args.next().context("--host attend une adresse")?,
            "--port" => {
                let port = args.next().context("--port attend un numéro")?;
                options.port = port.parse().context("numéro de port invalide")?;
            }
            "--password" => options.password = args.next(),
            "--no-tls" => options.no_tls = true,
            "--verbose" => options.verbose = true,
            "-h" | "--help" => bail!("{USAGE}"),
            _ if arg.starts_with("--") => bail!("option inconnue : {arg}\n{USAGE}"),
            _ if seed.is_none() => seed = Some(PathBuf::from(arg)),
            _ => bail!("un seul fichier de seed attendu\n{USAGE}"),
        }
    }
    options.seed = seed.context(USAGE)?;
    Ok(options)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let options = parse_args()?;
    tracing_subscriber::fmt()
        .with_max_level(if options.verbose {
            Level::DEBUG
        } else {
            Level::INFO
        })
        .init();

    if !options.no_tls {
        bail!(
            "le chiffrement (wss) n'est pas encore disponible (J1, étape 8).\n\
             Pour servir en clair (ws), ajoutez --no-tls.\n{USAGE}"
        );
    }

    let bytes = std::fs::read(&options.seed)
        .with_context(|| format!("lecture de {}", options.seed.display()))?;
    let data = load_archipelago(&bytes, &ArchiveLimits::default())
        .with_context(|| format!("seed invalide : {}", options.seed.display()))?;
    info!(
        seed = %data.seed_name,
        slots = data.slots.len(),
        "seed chargée"
    );

    let mut room = Room::new(data);
    if options.password.is_some() {
        room = room.with_password(options.password);
    }
    let server = Server::bind((options.host.as_str(), options.port), room).await?;
    info!(
        "en écoute sur ws://{} (sans chiffrement)",
        server.local_addr()?
    );

    // `select!` : on attend le premier des deux, le serveur ou Ctrl+C.
    tokio::select! {
        result = server.run() => result?,
        _ = tokio::signal::ctrl_c() => info!("arrêt demandé"),
    }
    Ok(())
}
