//! `raftup-server` : sert une seed `.archipelago` aux clients compatibles avec Archipelago.
//!
//! ```text
//! raftup-server <seed.archipelago> --cert fullchain.pem --cert_key privkey.pem
//!               [--host 0.0.0.0] [--port 38281] [--password X] [--verbose]
//! raftup-server <seed.archipelago> --no-tls [...]
//! ```
//!
//! Par défaut, le serveur parle `wss://` (NF06, ADR 0009) : il lui faut un
//! certificat et sa clé privée, au format PEM. Le clair (`ws://`) se demande
//! explicitement avec `--no-tls`.

use std::path::PathBuf;

use anyhow::{bail, Context};
use multidata::{load_archipelago, ArchiveLimits};
use server::{Room, Server, TlsConfig, DEFAULT_PORT};
use tracing::{info, warn, Level};

const USAGE: &str = "usage : raftup-server <seed.archipelago> \
(--cert CERTIFICAT.pem --cert_key CLE.pem | --no-tls) \
[--host 0.0.0.0] [--port 38281] [--password MOT_DE_PASSE] [--verbose]";

/// Options de la ligne de commande.
struct Options {
    seed: PathBuf,
    host: String,
    port: u16,
    password: Option<String>,
    cert: Option<PathBuf>,
    cert_key: Option<PathBuf>,
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
        cert: None,
        cert_key: None,
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
            // Mêmes noms que le serveur officiel (`MultiServer.py`).
            "--cert" => {
                let path = args.next().context("--cert attend un fichier PEM")?;
                options.cert = Some(PathBuf::from(path));
            }
            "--cert_key" => {
                let path = args.next().context("--cert_key attend un fichier PEM")?;
                options.cert_key = Some(PathBuf::from(path));
            }
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

/// Charge le certificat, ou `None` avec `--no-tls`. Refuse les mélanges.
fn tls_config(options: &Options) -> anyhow::Result<Option<TlsConfig>> {
    match (&options.cert, &options.cert_key, options.no_tls) {
        (None, None, true) => Ok(None),
        (Some(_), _, true) | (_, Some(_), true) => {
            bail!("--no-tls ne va pas avec --cert ou --cert_key : choisissez l'un ou l'autre")
        }
        (Some(cert), Some(key), false) => {
            let config = TlsConfig::from_pem_files(cert, key)
                .context("impossible de préparer le chiffrement (wss)")?;
            Ok(Some(config))
        }
        (Some(_), None, false) => bail!("--cert demande aussi --cert_key (la clé privée)"),
        (None, Some(_), false) => bail!("--cert_key demande aussi --cert (le certificat)"),
        (None, None, false) => bail!(
            "le serveur chiffre ses connexions (wss) par défaut : donnez un certificat \
             avec --cert et --cert_key (fichiers PEM).\n\
             Pour servir en clair (ws), ajoutez --no-tls : le mot de passe de la salle \
             passera alors en clair.\n{USAGE}"
        ),
    }
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

    // Le certificat d'abord : une erreur de configuration se voit tout de suite,
    // avant de lire la seed.
    let tls = tls_config(&options)?;

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
    let mut server = Server::bind((options.host.as_str(), options.port), room).await?;
    let address = server.local_addr()?;
    match tls {
        Some(tls) => {
            server = server.with_tls(tls);
            info!("en écoute sur wss://{address}");
        }
        None => warn!("en écoute sur ws://{address}, sans chiffrement (--no-tls)"),
    }

    // `select!` : on attend le premier des deux, le serveur ou Ctrl+C.
    tokio::select! {
        result = server.run() => result?,
        _ = tokio::signal::ctrl_c() => info!("arrêt demandé"),
    }
    Ok(())
}
