//! Héberger une partie dans le launcher (F09, ADR 0010).
//!
//! Le serveur tourne dans le même process que le launcher. [`Host`] le
//! démarre, en donne une photo, garde ses événements pour l'interface, et
//! l'arrête.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use multidata::{load_archipelago, read_file_bounded, ArchiveLimits, MAX_SEED_FILE_BYTES};
use serde::{Deserialize, Serialize};
use server::save::{default_save_path, open_or_create};
use server::{GameEvent, Room, RunningServer, Server, ServerStatus, TlsConfig};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tracing::info;

use crate::error::LauncherError;
use crate::seed::SeedSummary;

/// Événements gardés au plus pour l'interface.
const EVENT_CAPACITY: usize = 2000;

/// Chiffrement choisi par l'hôte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Encryption {
    /// `wss`, avec un certificat et sa clé au format PEM (ADR 0009).
    Wss { cert: PathBuf, key: PathBuf },
    /// `ws` en clair : le mot de passe passe en clair (NF06, choix explicite).
    Plain,
}

/// Réglages de l'écran « Héberger une partie ».
///
/// `Debug` est écrit à la main (plus bas) : il masque le mot de passe, pour
/// qu'un `info!(?options)` ne l'écrive jamais dans le journal (NF09).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostOptions {
    /// Fichier `.archipelago`.
    pub seed: PathBuf,
    pub port: u16,
    /// Mot de passe de la salle ; vide ou absent : celui de la seed, s'il y en a un.
    #[serde(default)]
    pub password: Option<String>,
    pub encryption: Encryption,
    /// Nom à donner aux joueurs dans l'adresse (nom de domaine, IP…).
    #[serde(default = "default_public_host")]
    pub public_host: String,
    /// Vrai : pas de sauvegarde (la partie est perdue à l'arrêt). Par défaut,
    /// la partie est sauvegardée dans un `.raftup` à côté de la seed, et
    /// reprend si ce fichier existe déjà (ADR 0014).
    #[serde(default)]
    pub disable_save: bool,
}

impl std::fmt::Debug for HostOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Un mot de passe donné apparaît comme « *** », jamais en clair.
        let password = self.password.as_ref().map(|_| "***");
        f.debug_struct("HostOptions")
            .field("seed", &self.seed)
            .field("port", &self.port)
            .field("password", &password)
            .field("encryption", &self.encryption)
            .field("public_host", &self.public_host)
            .field("disable_save", &self.disable_save)
            .finish()
    }
}

fn default_public_host() -> String {
    "localhost".to_string()
}

/// Ce que l'écran « Partie en cours » affiche.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HostStatus {
    pub seed: SeedSummary,
    /// Adresse à donner aux joueurs : `wss://exemple.fr:38281`.
    pub address: String,
    pub encrypted: bool,
    pub has_password: bool,
    /// Heure du démarrage, en secondes depuis 1970.
    pub started_at: f64,
    /// Fichier de sauvegarde ; `None` si la sauvegarde est désactivée.
    pub save: Option<PathBuf>,
    /// Vrai si la partie a repris depuis une sauvegarde existante.
    pub resumed: bool,
    pub server: ServerStatus,
}

/// Un événement de la partie, numéroté pour que l'interface ne lise que les nouveaux.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NumberedEvent {
    pub number: u64,
    pub event: GameEvent,
}

#[derive(Debug, Default)]
struct EventLog {
    events: VecDeque<NumberedEvent>,
    next: u64,
}

/// La partie hébergée, s'il y en a une.
#[derive(Default)]
pub struct Host {
    hosted: Option<Hosted>,
}

struct Hosted {
    running: RunningServer,
    seed: SeedSummary,
    address: String,
    encrypted: bool,
    has_password: bool,
    started_at: f64,
    save: Option<PathBuf>,
    resumed: bool,
    /// Partagé avec la tâche qui recopie le fil d'événements du serveur.
    events: Arc<Mutex<EventLog>>,
    forwarder: JoinHandle<()>,
}

impl Host {
    pub fn new() -> Host {
        Host::default()
    }

    /// Vrai si une partie est hébergée.
    pub fn is_running(&self) -> bool {
        self.hosted.is_some()
    }

    /// Démarre le serveur sur toutes les interfaces, au port demandé.
    pub async fn start(&mut self, options: HostOptions) -> Result<HostStatus, LauncherError> {
        if self.hosted.is_some() {
            return Err(LauncherError::AlreadyRunning);
        }
        // Le certificat d'abord : une erreur de configuration se voit tout de suite.
        let tls = match &options.encryption {
            Encryption::Wss { cert, key } => Some(TlsConfig::from_pem_files(cert, key)?),
            Encryption::Plain => None,
        };
        // Les octets de la seed servent deux fois : la lecture, et l'empreinte
        // qui lie la sauvegarde à cette seed.
        let bytes = read_file_bounded(&options.seed, MAX_SEED_FILE_BYTES).map_err(|source| {
            LauncherError::Read {
                path: options.seed.display().to_string(),
                source,
            }
        })?;
        let data = load_archipelago(&bytes, &ArchiveLimits::default())?;
        let seed = SeedSummary::of(&data);
        // Le nom de la seed est copié avant que `data` parte dans la salle.
        let seed_name = data.seed_name.clone();
        let mut room = Room::new(data);
        let password = options.password.clone().filter(|p| !p.is_empty());
        if password.is_some() {
            room = room.with_password(password);
        }
        let has_password = room.password().is_some();

        let mut server = Server::bind(("0.0.0.0", options.port), room).await?;
        let encrypted = tls.is_some();
        if let Some(tls) = tls {
            server = server.with_tls(tls);
        }
        // La sauvegarde est ouverte APRÈS le bind : si le port est pris, aucun
        // `.raftup` n'est créé, et un réessai ne « reprend » pas une sauvegarde vide.
        let (save, resumed) = if options.disable_save {
            (None, false)
        } else {
            let path = default_save_path(&options.seed);
            let opened = open_or_create(&path, &bytes, &seed_name)?;
            server = server.with_save(opened.file, opened.snapshot);
            (Some(path), opened.resumed)
        };
        let running = server.start()?;
        let port = running.local_addr().port();
        let scheme = if encrypted { "wss" } else { "ws" };
        let address = format!("{scheme}://{}:{port}", options.public_host);
        info!(%address, seed = %seed.seed_name, "partie hébergée");

        // Une tâche recopie les événements du serveur dans un journal borné.
        let events = Arc::new(Mutex::new(EventLog::default()));
        let mut receiver = running.subscribe();
        let log = Arc::clone(&events);
        let forwarder = tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let mut log = log.lock().unwrap_or_else(|e| e.into_inner());
                        let number = log.next;
                        log.next += 1;
                        log.events.push_back(NumberedEvent { number, event });
                        while log.events.len() > EVENT_CAPACITY {
                            log.events.pop_front();
                        }
                    }
                    // Trop lent : des événements sont perdus, on continue.
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                }
            }
        });

        self.hosted = Some(Hosted {
            running,
            seed,
            address,
            encrypted,
            has_password,
            started_at: now(),
            save,
            resumed,
            events,
            forwarder,
        });
        match self.status().await {
            Some(status) => Ok(status),
            // La salle ne répond pas (arrêtée aussitôt) : on libère le port
            // au lieu de laisser un serveur à moitié lancé.
            None => {
                let _ = self.stop().await;
                Err(LauncherError::NotRunning)
            }
        }
    }

    /// Photo de la partie hébergée ; `None` s'il n'y en a pas.
    pub async fn status(&self) -> Option<HostStatus> {
        let hosted = self.hosted.as_ref()?;
        let server = hosted.running.status().await?;
        Some(HostStatus {
            seed: hosted.seed.clone(),
            address: hosted.address.clone(),
            encrypted: hosted.encrypted,
            has_password: hosted.has_password,
            started_at: hosted.started_at,
            save: hosted.save.clone(),
            resumed: hosted.resumed,
            server,
        })
    }

    /// Événements dont le numéro est au moins `from`.
    pub fn events_since(&self, from: u64) -> Vec<NumberedEvent> {
        let Some(hosted) = &self.hosted else {
            return Vec::new();
        };
        let log = hosted.events.lock().unwrap_or_else(|e| e.into_inner());
        log.events
            .iter()
            .filter(|e| e.number >= from)
            .cloned()
            .collect()
    }

    /// Message de l'hôte à tous les joueurs.
    pub async fn announce(&self, text: String) -> Result<(), LauncherError> {
        let hosted = self.hosted.as_ref().ok_or(LauncherError::NotRunning)?;
        let text = text.trim().to_string();
        if !text.is_empty() {
            hosted.running.announce(text).await?;
        }
        Ok(())
    }

    /// Arrête la partie hébergée ; le port est libre au retour.
    pub async fn stop(&mut self) -> Result<(), LauncherError> {
        let hosted = self.hosted.take().ok_or(LauncherError::NotRunning)?;
        hosted.forwarder.abort();
        hosted.running.stop().await;
        Ok(())
    }
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
