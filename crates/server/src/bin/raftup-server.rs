//! `raftup-server` : sert une seed `.archipelago` aux clients compatibles avec Archipelago.
//!
//! ```text
//! raftup-server <seed.archipelago> --cert fullchain.pem --cert_key privkey.pem
//!               [--host 0.0.0.0] [--port 38281] [--password X] [--verbose]
//!               [--savefile partie.raftup | --disable_save] [--async_follow]
//! raftup-server <seed.archipelago> --no-tls [...]
//! ```
//!
//! `--async_follow` active le suivi asynchrone (ADR 0031) : `!away`, `!back`,
//! `!summary`, `!vote_release` et résumé de retour. Sans lui, le serveur se
//! comporte comme le serveur officiel.
//!
//! Par défaut, le serveur parle `wss://` (NF06, ADR 0009) : il lui faut un
//! certificat et sa clé privée, au format PEM. Le clair (`ws://`) se demande
//! explicitement avec `--no-tls`.
//!
//! La partie est sauvegardée (ADR 0014) dans un `.raftup` à côté de la seed,
//! ou dans `--savefile`. S'il existe déjà, la partie reprend là où elle
//! s'était arrêtée. `--disable_save` désactive la sauvegarde.

use std::path::PathBuf;

use anyhow::{bail, Context};
use multidata::{load_archipelago, ArchiveLimits};
use server::save::{default_save_path, open_or_create};
use server::{Room, Server, TlsConfig, DEFAULT_PORT};
use tracing::{info, warn, Level};

const USAGE: &str = "usage : raftup-server <seed.archipelago> \
(--cert CERTIFICAT.pem --cert_key CLE.pem | --no-tls) \
[--host 0.0.0.0] [--port 38281] [--password MOT_DE_PASSE] [--verbose] \
[--savefile PARTIE.raftup | --disable_save] [--async_follow]";

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
    /// `None` : à côté de la seed, avec l'extension `.raftup`.
    savefile: Option<PathBuf>,
    disable_save: bool,
    /// Suivi asynchrone (ADR 0031) : `!away`, `!vote_release`, résumé de retour.
    async_follow: bool,
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
        savefile: None,
        disable_save: false,
        async_follow: false,
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
            // Mêmes noms que le serveur officiel.
            "--savefile" => {
                let path = args.next().context("--savefile attend un chemin")?;
                options.savefile = Some(PathBuf::from(path));
            }
            "--disable_save" => options.disable_save = true,
            "--async_follow" => options.async_follow = true,
            "-h" | "--help" => bail!("{USAGE}"),
            _ if arg.starts_with("--") => bail!("option inconnue : {arg}\n{USAGE}"),
            _ if seed.is_none() => seed = Some(PathBuf::from(arg)),
            _ => bail!("un seul fichier de seed attendu\n{USAGE}"),
        }
    }
    options.seed = seed.context(USAGE)?;
    if options.disable_save && options.savefile.is_some() {
        bail!("--disable_save ne va pas avec --savefile : choisissez l'un ou l'autre");
    }
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

    let bytes = multidata::read_file_bounded(&options.seed, multidata::MAX_SEED_FILE_BYTES)
        .with_context(|| format!("lecture de {}", options.seed.display()))?;
    let data = load_archipelago(&bytes, &ArchiveLimits::default())
        .with_context(|| format!("seed invalide : {}", options.seed.display()))?;
    info!(
        seed = %data.seed_name,
        slots = data.slots.len(),
        "seed chargée"
    );

    // Lue avant de donner la seed à la salle : la sauvegarde a besoin de son nom.
    let save = if options.disable_save {
        warn!("sauvegarde désactivée (--disable_save) : la partie sera perdue à l'arrêt");
        None
    } else {
        let path = options
            .savefile
            .clone()
            .unwrap_or_else(|| default_save_path(&options.seed));
        Some(open_or_create(&path, &bytes, &data.seed_name)?)
    };

    let mut room = Room::new(data).with_async_follow(options.async_follow);
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
    if let Some(save) = save {
        server = server.with_save(save.file, save.snapshot);
    }

    // Le serveur tourne en arrière-plan jusqu'à Ctrl+C ; `stop` laisse alors
    // la salle compacter sa sauvegarde avant de quitter.
    let running = server.start()?;
    tokio::signal::ctrl_c()
        .await
        .context("attente de Ctrl+C impossible")?;
    info!("arrêt demandé");
    running.stop().await;
    Ok(())
}
