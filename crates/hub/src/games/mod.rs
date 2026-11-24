//! Parties hébergées par le hub (ADR 0052, étape 1).
//!
//! L'hôte génère la seed sur son PC, puis l'envoie au hub (`POST /v1/games`). Le hub :
//! 1. la relit avec la lecture sécurisée de `multidata` (aucun code exécuté) ;
//! 2. la range avec sa sauvegarde dans `<données du hub>/parties/<id>/`
//!    (`partie.archipelago` et `partie.raftup`) ;
//! 3. prend un port libre de sa plage (38281 à 38380 par défaut) ;
//! 4. lance `server::Server` **dans son propre processus**, avec `server::Limits`.
//!
//! Le serveur de jeu est exactement celui du self-host : un mod d'Archipelago non
//! modifié s'y connecte avec l'adresse `hôte:port`, sans rien savoir du hub. La
//! sauvegarde est écrite par le serveur avant chaque réponse (ADR 0014), puis
//! compactée à l'arrêt ; au redémarrage, le hub relance chaque partie sur son port
//! et elle reprend là où elle en était ([`GameHost::restore`]).
//!
//! Veille (étape 3, point 6) : une partie sans connexion entrée dans un slot pendant
//! `idle_after` (2 h) s'arrête, mais le hub **garde son port ouvert** ([`guard_port`]).
//! La première connexion sur ce port la réveille : le serveur repart sur le même
//! écouteur et reçoit la connexion déjà acceptée, si bien qu'un mod non modifié ne
//! voit qu'un court délai. Les relevés d'activité et les réveils sont faits par une
//! tâche du hub (`supervise_games`, dans `lib.rs`).
//!
//! Pas encore fait (étapes 4 et 5 de l'ADR 0052) : page « Parties » de la console,
//! export RGPD, déploiement.

pub mod cert;
pub mod routes;
pub mod secret;

use std::collections::BTreeMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ap_protocol::SlotType;
use multidata::{load_archipelago, ArchiveLimits, MultiData};
use server::save::open_or_create;
use server::{Limits, Room, RunningServer, Server, ServerStatus, TlsConfig};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::error::HubError;
use crate::session::{new_invite_code, new_token};
use crate::store::{GameRow, Store};
use secret::GameKey;

/// Nom du fichier de la seed, dans le dossier de chaque partie.
pub const SEED_FILE: &str = "partie.archipelago";
/// Nom de la sauvegarde : celui de la seed avec `.raftup`, comme en self-host
/// (`server::save::default_save_path`). Un export se reprend donc tel quel.
pub const SAVE_FILE: &str = "partie.raftup";
/// Usage des mots de passe chiffrés (données associées du chiffrement).
const PASSWORD_PURPOSE: &str = "password";
const ADMIN_PURPOSE: &str = "admin_password";
/// Longueur maximale du nom d'une partie.
const MAX_NAME_CHARS: usize = 64;
/// Longueur maximale du mot de passe, en octets (UTF-8) : chiffré et écrit en
/// hexadécimal, il tient dans une colonne `VARCHAR(1024)` de MySQL
/// ((12 + 256 + 16) × 2 = 568 caractères).
const MAX_PASSWORD_BYTES: usize = 256;

/// Réglages de l'hébergement (ADR 0052, points 4 et 7). `Default` donne les valeurs
/// de production ; les tests changent le dossier, la plage et l'adresse d'écoute.
#[derive(Clone, Debug)]
pub struct GamesConfig {
    /// Dossier des parties (`<données du hub>/parties`).
    pub data_dir: PathBuf,
    /// Plage de ports réservée aux parties, bornes comprises.
    pub first_port: u16,
    pub last_port: u16,
    /// Adresse d'écoute des serveurs de jeu (`0.0.0.0` : toutes les interfaces).
    pub listen_host: String,
    /// Parties au plus par compte, et sur tout le hub.
    pub max_per_account: i64,
    pub max_total: i64,
    /// Joueurs au plus par partie (les groupes ne comptent pas).
    pub max_players: usize,
    /// Taille maximale de la seed et de la sauvegarde envoyées, en octets.
    pub max_seed_bytes: usize,
    pub max_save_bytes: usize,
    /// Limites de connexion et de débit de chaque serveur de jeu (ADR 0007).
    pub limits: Limits,
    /// Veille après ce délai sans connexion entrée dans un slot (2 h).
    pub idle_after: Duration,
    /// Après un réveil, délai pour qu'une connexion entre dans un slot avant de se
    /// rendormir (10 min) : un balayage de ports ne garde pas la partie allumée.
    pub wake_grace: Duration,
    /// Intervalle des relevés d'activité (1 min).
    pub activity_check: Duration,
}

impl Default for GamesConfig {
    fn default() -> Self {
        GamesConfig {
            data_dir: PathBuf::from("rafthub-data").join("parties"),
            first_port: 38281,
            last_port: 38380,
            listen_host: "0.0.0.0".to_string(),
            max_per_account: 2,
            max_total: 40,
            max_players: 16,
            max_seed_bytes: 32 * 1024 * 1024,
            max_save_bytes: 32 * 1024 * 1024,
            limits: Limits::default(),
            idle_after: Duration::from_secs(2 * 3600),
            wake_grace: Duration::from_secs(10 * 60),
            activity_check: Duration::from_secs(60),
        }
    }
}

impl GamesConfig {
    /// Lit une plage `38281-38380` (variable `RAFTUP_HUB_GAME_PORTS`).
    pub fn parse_ports(text: &str) -> Option<(u16, u16)> {
        let (first, last) = text.trim().split_once('-')?;
        let first: u16 = first.trim().parse().ok()?;
        let last: u16 = last.trim().parse().ok()?;
        (first > 0 && first <= last).then_some((first, last))
    }
}

/// Une partie envoyée par un hôte.
pub struct Upload {
    pub name: String,
    /// Mot de passe de la partie (vide ou absent : celui de la seed, s'il y en a un).
    pub password: Option<String>,
    /// Le fichier `.archipelago`.
    pub seed: Vec<u8>,
    /// Une sauvegarde `.raftup` de cette seed, pour continuer une partie commencée.
    pub save: Option<Vec<u8>>,
}

/// `Debug` écrit à la main : ni mot de passe, ni contenu des fichiers.
impl std::fmt::Debug for Upload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upload")
            .field("name", &self.name)
            .field("seed_bytes", &self.seed.len())
            .field("save_bytes", &self.save.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}

/// Résultat d'un envoi réussi.
pub struct Created {
    pub row: GameRow,
    /// Empreinte du certificat des parties (`fp` des liens d'invitation).
    pub fingerprint: String,
    /// Mot de passe d'admin (`!admin login …`) : montré à l'hôte seulement.
    pub admin_password: String,
    /// Vrai si une sauvegarde envoyée a été reprise.
    pub resumed: bool,
}

/// Une partie qui tourne.
struct Running {
    server: RunningServer,
    fingerprint: String,
    /// Sans connexion entrée dans un slot d'ici là, la partie se met en veille.
    /// Repoussé de `idle_after` à chaque relevé où quelqu'un joue ; `wake_grace`
    /// seulement après un réveil.
    sleep_at: Instant,
}

/// Une partie en veille : plus de serveur, seulement une tâche qui garde le port
/// ouvert et attend la connexion qui la réveillera (ADR 0052, point 6).
struct Asleep {
    fingerprint: String,
    /// Envoyer ici (ou lâcher ce côté du canal) finit la garde sans connexion.
    cancel: oneshot::Sender<()>,
    guard: JoinHandle<GuardExit>,
}

/// Ce que rend la garde du port en finissant.
struct GuardExit {
    listener: TcpListener,
    /// La connexion qui a réveillé la partie ; `None` : réveil demandé autrement.
    pending: Option<(TcpStream, SocketAddr)>,
}

/// Une partie connue du hub, éveillée ou en veille. Une partie absente de la table
/// est « arrêtée » (elle n'a pas pu repartir).
enum Game {
    Awake(Running),
    Asleep(Asleep),
}

/// État d'une partie, vu par l'API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameState {
    /// Le serveur tourne.
    Active,
    /// En veille : le port est gardé, la première connexion la réveille.
    Sleeping,
    /// Ni l'un ni l'autre : elle n'a pas pu repartir (voir le journal).
    Stopped,
}

impl GameState {
    pub fn as_str(self) -> &'static str {
        match self {
            GameState::Active => "active",
            GameState::Sleeping => "sleeping",
            GameState::Stopped => "stopped",
        }
    }
}

/// Comment ouvrir le port d'un serveur de jeu.
enum Bind<'a> {
    /// Le port de la partie (reprise au démarrage, relance d'une partie arrêtée).
    Port(u16),
    /// Le premier port libre de la plage, sauf ceux-là (réservés par d'autres parties).
    Free(&'a [i64]),
    /// L'écouteur gardé pendant la veille, et peut-être la connexion qui réveille.
    Listener(TcpListener, Option<(TcpStream, SocketAddr)>),
}

/// Les serveurs de jeu du hub.
pub struct GameHost {
    config: GamesConfig,
    /// Parties éveillées ou en veille, par identifiant.
    ///
    /// Mutex de tokio (et pas de `std`) : il reste pris pendant des `.await` (ouvrir
    /// un port, arrêter un serveur). Il sérialise aussi les créations : deux envois
    /// simultanés ne prennent pas le même port et ne dépassent pas les limites.
    games: Mutex<BTreeMap<String, Game>>,
    /// Les gardes de port envoient ici l'identifiant de la partie à réveiller ; la
    /// tâche de veille du hub ([`GameHost::take_wake_receiver`]) fait le réveil.
    wake_tx: mpsc::UnboundedSender<String>,
    /// Récepteur de `wake_tx`, pris une seule fois par la tâche de veille.
    /// Mutex de `std` : jamais pris pendant un `.await`.
    wake_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<String>>>,
}

impl GameHost {
    pub fn new(config: GamesConfig) -> GameHost {
        // Canal sans borne : au plus un message par garde de port, qui finit aussitôt.
        let (wake_tx, wake_rx) = mpsc::unbounded_channel();
        GameHost {
            config,
            games: Mutex::new(BTreeMap::new()),
            wake_tx,
            wake_rx: std::sync::Mutex::new(Some(wake_rx)),
        }
    }

    pub fn config(&self) -> &GamesConfig {
        &self.config
    }

    /// Le récepteur des demandes de réveil des gardes de port, pour la tâche de
    /// veille du hub. `None` s'il a déjà été pris.
    pub fn take_wake_receiver(&self) -> Option<mpsc::UnboundedReceiver<String>> {
        self.wake_rx.lock().ok()?.take()
    }

    /// Dossier d'une partie. L'identifiant est vérifié avant (voir [`valid_id`]).
    fn game_dir(&self, id: &str) -> PathBuf {
        self.config.data_dir.join(id)
    }

    /// Certificat des parties (le même pour toutes), créé au premier besoin.
    fn certificate(&self) -> Result<cert::GameCertificate, HubError> {
        cert::ensure(&self.config.data_dir.join("tls"))
    }

    /// Relance toutes les parties de la base (démarrage du hub). Une partie en
    /// veille retrouve seulement la garde de son port. Une partie qui ne repart pas
    /// (fichier abîmé, port pris par un autre programme) reste arrêtée, avec une
    /// erreur au journal ; les autres repartent quand même.
    pub async fn restore(&self, store: &Store) -> Result<(), HubError> {
        let rows = store.all_games().await?;
        if rows.is_empty() {
            return Ok(());
        }
        let key = GameKey::load_or_create(&self.config.data_dir)?;
        let mut games = self.games.lock().await;
        for row in rows {
            if games.contains_key(&row.id) {
                continue;
            }
            let restored = if row.asleep {
                self.guard_port(&row).await.map(Game::Asleep)
            } else {
                match port_of(&row) {
                    Ok(port) => self
                        .relaunch(&key, &row, Bind::Port(port))
                        .await
                        .map(Game::Awake),
                    Err(error) => Err(error),
                }
            };
            match restored {
                Ok(game) => {
                    tracing::info!(game = %row.id, port = row.port, asleep = row.asleep, "partie hébergée reprise");
                    games.insert(row.id.clone(), game);
                }
                Err(error) => {
                    tracing::error!(game = %row.id, %error, "partie hébergée non reprise");
                }
            }
        }
        Ok(())
    }

    /// Rouvre le port d'une partie en veille (démarrage du hub) et le garde.
    async fn guard_port(&self, row: &GameRow) -> Result<Asleep, HubError> {
        let port = port_of(row)?;
        let listener = TcpListener::bind((self.config.listen_host.as_str(), port))
            .await
            .map_err(|e| HubError::Internal(format!("port {port} : {e}")))?;
        let fingerprint = self.certificate()?.fingerprint;
        Ok(self.spawn_guard(&row.id, listener, fingerprint))
    }

    /// Lance la garde d'un port : elle attend une connexion, prévient la tâche de
    /// veille, puis rend l'écouteur et la connexion à [`GameHost::wake`].
    fn spawn_guard(&self, id: &str, listener: TcpListener, fingerprint: String) -> Asleep {
        let (cancel, cancelled) = oneshot::channel();
        let guard = tokio::spawn(guard_port(
            id.to_string(),
            listener,
            cancelled,
            self.wake_tx.clone(),
        ));
        Asleep {
            fingerprint,
            cancel,
            guard,
        }
    }

    /// Relance une partie déjà enregistrée.
    async fn relaunch(
        &self,
        key: &GameKey,
        row: &GameRow,
        bind: Bind<'_>,
    ) -> Result<Running, HubError> {
        let dir = self.game_dir(&row.id);
        let seed = std::fs::read(dir.join(SEED_FILE))
            .map_err(|e| HubError::Internal(format!("seed illisible : {e}")))?;
        let data = load_seed(seed.clone()).await?;
        let password = row
            .password_enc
            .as_deref()
            .map(|sealed| key.open(&row.id, PASSWORD_PURPOSE, sealed))
            .transpose()?;
        let admin_password = key.open(&row.id, ADMIN_PURPOSE, &row.admin_password_enc)?;
        let (running, _) = self
            .launch(&dir, data, &seed, password, admin_password, bind)
            .await?;
        Ok(running)
    }

    /// Ouvre un port (ou reprend l'écouteur gardé), la sauvegarde, puis démarre le
    /// serveur de jeu. Rend le serveur lancé et `true` si une sauvegarde existante a
    /// repris.
    async fn launch(
        &self,
        dir: &Path,
        mut data: MultiData,
        seed: &[u8],
        password: Option<String>,
        admin_password: String,
        bind: Bind<'_>,
    ) -> Result<(Running, bool), HubError> {
        let certificate = self.certificate()?;
        let tls = TlsConfig::from_pem(
            certificate.cert_pem.as_bytes(),
            certificate.key_pem.as_bytes(),
        )
        .map_err(|e| HubError::Internal(format!("certificat des parties : {e}")))?
        // Port mixte (ADR 0043) : `wss` pour le launcher, `ws` pour les mods.
        .with_plain_fallback();

        let seed_name = data.seed_name.clone();
        // Mot de passe d'admin (`!admin login …`) : tiré par le hub, jamais celui de la
        // seed (ADR 0052, point 8).
        data.server_options.server_password = Some(admin_password);
        // Suivi asynchrone (ADR 0031), comme dans le launcher : utile surtout pour une
        // partie longue sur le hub, et sans effet pour un mod existant.
        let mut room = Room::new(data).with_async_follow(true);
        if password.is_some() {
            room = room.with_password(password);
        }

        let host = self.config.listen_host.as_str();
        let server = match bind {
            Bind::Port(port) => Server::bind((host, port), room)
                .await
                .map_err(|e| HubError::Internal(format!("port {port} : {e}")))?,
            Bind::Free(reserved) => self.bind_free_port(host, room, reserved).await?,
            Bind::Listener(listener, pending) => {
                let server = Server::from_listener(listener, room);
                match pending {
                    Some((stream, peer)) => server.with_pending_connection(stream, peer),
                    None => server,
                }
            }
        };
        // La sauvegarde est ouverte APRÈS l'ouverture du port (comme le launcher) : si
        // aucun port n'est libre, aucun `.raftup` n'est créé.
        let opened = open_or_create(&dir.join(SAVE_FILE), seed, &seed_name)
            .map_err(|e| HubError::InvalidSave(e.to_string()))?;
        let resumed = opened.resumed;
        let server = server
            .with_tls(tls)
            .with_save(opened.file, opened.snapshot)
            .with_limits(self.config.limits)
            .start()
            .map_err(|e| HubError::Internal(format!("démarrage du serveur de jeu : {e}")))?;
        Ok((
            Running {
                server,
                fingerprint: certificate.fingerprint,
                sleep_at: Instant::now() + self.config.idle_after,
            },
            resumed,
        ))
    }

    /// Premier port libre de la plage. Un port réservé par une autre partie est
    /// sauté ; un port pris par un autre programme aussi (l'ouverture échoue).
    async fn bind_free_port(
        &self,
        host: &str,
        room: Room,
        reserved: &[i64],
    ) -> Result<Server, HubError> {
        for port in self.config.first_port..=self.config.last_port {
            if reserved.contains(&i64::from(port)) {
                continue;
            }
            // `room.clone()` : `bind` garde la salle ; en cas d'échec il faut la
            // redonner au port suivant.
            match Server::bind((host, port), room.clone()).await {
                Ok(server) => return Ok(server),
                Err(error) => tracing::debug!(port, %error, "port de partie occupé"),
            }
        }
        Err(HubError::NoFreePort)
    }

    /// Reçoit une partie et la lance (`POST /v1/games`).
    pub async fn create(
        &self,
        store: &Store,
        owner_id: i64,
        upload: Upload,
        now: i64,
    ) -> Result<Created, HubError> {
        let name = check_name(&upload.name)?;
        let password = check_password(upload.password)?;
        if upload.seed.len() > self.config.max_seed_bytes {
            return Err(HubError::SeedTooLarge);
        }
        if upload
            .save
            .as_ref()
            .is_some_and(|save| save.len() > self.config.max_save_bytes)
        {
            return Err(HubError::SeedTooLarge);
        }
        let seed_size = upload.seed.len();
        let data = load_seed(upload.seed.clone()).await?;
        let players = data
            .slots
            .values()
            .filter(|slot| slot.slot_type == SlotType::PLAYER)
            .count();
        if players > self.config.max_players {
            return Err(HubError::TooManyPlayers(self.config.max_players));
        }

        // Tout ce qui suit est sous le verrou : limites, choix du port, écriture.
        let mut games = self.games.lock().await;
        if store.count_games(Some(owner_id)).await? >= self.config.max_per_account {
            return Err(HubError::GameLimitReached(self.config.max_per_account));
        }
        if store.count_games(None).await? >= self.config.max_total {
            return Err(HubError::HubFull);
        }
        let key = GameKey::load_or_create(&self.config.data_dir)?;
        // 16 octets aléatoires suffisent (128 bits) : l'identifiant sert de laissez-passer
        // dans les liens d'invitation, il ne doit pas se deviner.
        let id: String = new_token().chars().take(32).collect();
        let admin_password = new_invite_code();
        let dir = self.game_dir(&id);
        let reserved = store.game_ports().await?;
        // Copié avant que `data` parte dans la salle du serveur.
        let seed_name = data.seed_name.clone();

        // En cas d'échec plus loin, le dossier de la partie est retiré.
        let outcome = async {
            write_new_file(&dir, SEED_FILE, &upload.seed)?;
            if let Some(save) = &upload.save {
                write_new_file(&dir, SAVE_FILE, save)?;
            }
            let (game, resumed) = self
                .launch(
                    &dir,
                    data,
                    &upload.seed,
                    password.clone(),
                    admin_password.clone(),
                    Bind::Free(&reserved),
                )
                .await?;
            let port = game.server.local_addr().port();
            let row = GameRow {
                id: id.clone(),
                owner_id,
                name: name.clone(),
                port: i64::from(port),
                seed_name,
                player_count: i64::try_from(players).unwrap_or(i64::MAX),
                seed_size: i64::try_from(seed_size).unwrap_or(i64::MAX),
                password_enc: password
                    .as_deref()
                    .map(|p| key.seal(&id, PASSWORD_PURPOSE, p))
                    .transpose()?,
                admin_password_enc: key.seal(&id, ADMIN_PURPOSE, &admin_password)?,
                created_at: now,
                last_activity_at: None,
                asleep: false,
            };
            if let Err(error) = store.insert_game(&row).await {
                game.server.stop().await;
                return Err(error);
            }
            Ok((game, row, resumed))
        }
        .await;

        match outcome {
            Ok((game, row, resumed)) => {
                tracing::info!(game = %id, port = row.port, owner = owner_id, "partie hébergée créée");
                let fingerprint = game.fingerprint.clone();
                games.insert(id, Game::Awake(game));
                Ok(Created {
                    row,
                    fingerprint,
                    admin_password,
                    resumed,
                })
            }
            Err(error) => {
                remove_game_dir(&dir);
                // Une sauvegarde refusée n'est une « sauvegarde invalide » que si
                // l'hôte en a envoyé une ; sinon, c'est une panne du hub.
                Err(match error {
                    HubError::InvalidSave(detail) if upload.save.is_none() => {
                        HubError::Internal(detail)
                    }
                    other => other,
                })
            }
        }
    }

    /// Mot de passe d'admin de la partie, déchiffré (pour l'hôte seulement).
    pub fn admin_password(&self, row: &GameRow) -> Result<String, HubError> {
        let key = GameKey::load_or_create(&self.config.data_dir)?;
        key.open(&row.id, ADMIN_PURPOSE, &row.admin_password_enc)
    }

    /// État de la partie, et l'empreinte de son certificat si elle est éveillée ou
    /// en veille (les liens d'invitation restent bons pendant la veille).
    pub async fn state(&self, id: &str) -> (GameState, Option<String>) {
        let games = self.games.lock().await;
        match games.get(id) {
            Some(Game::Awake(game)) => (GameState::Active, Some(game.fingerprint.clone())),
            Some(Game::Asleep(game)) => (GameState::Sleeping, Some(game.fingerprint.clone())),
            None => (GameState::Stopped, None),
        }
    }

    /// Empreinte du certificat de la partie, si elle est éveillée ou en veille.
    pub async fn fingerprint(&self, id: &str) -> Option<String> {
        self.state(id).await.1
    }

    /// Photo de la partie (joueurs connectés, avancement), si elle est éveillée.
    pub async fn status(&self, id: &str) -> Option<ServerStatus> {
        let games = self.games.lock().await;
        // La photo est demandée à la salle sous le verrou : elle répond vite (une
        // lecture de son état), et le serveur ne peut pas être arrêté entre-temps.
        match games.get(id)? {
            Game::Awake(game) => game.server.status().await,
            Game::Asleep(_) => None,
        }
    }

    /// Relevé d'activité (appelé chaque `activity_check` par la tâche de veille) :
    /// une partie où une connexion est entrée dans un slot repousse sa veille de
    /// `idle_after` et note l'heure `now` en base ; une partie vide depuis son
    /// échéance s'endort.
    pub async fn check_activity(&self, store: &Store, now: i64) {
        let mut to_sleep = Vec::new();
        let mut active = Vec::new();
        {
            let mut games = self.games.lock().await;
            for (id, game) in games.iter_mut() {
                let Game::Awake(game) = game else { continue };
                let playing = game
                    .server
                    .status()
                    .await
                    .is_some_and(|status| status.slots.iter().any(|slot| slot.connections > 0));
                if playing {
                    game.sleep_at = Instant::now() + self.config.idle_after;
                    active.push(id.clone());
                } else if Instant::now() >= game.sleep_at {
                    to_sleep.push(id.clone());
                }
            }
        }
        // La base est écrite sans le verrou des parties.
        for id in active {
            if let Err(error) = store.touch_game(&id, now).await {
                tracing::warn!(game = %id, %error, "dernière activité non écrite");
            }
        }
        for id in to_sleep {
            if let Err(error) = self.sleep(store, &id).await {
                tracing::error!(game = %id, %error, "mise en veille impossible");
            }
        }
    }

    /// Met une partie éveillée en veille : le serveur s'arrête (sauvegarde
    /// compactée), le port reste ouvert et gardé. Sans effet si elle n'est pas
    /// éveillée.
    pub async fn sleep(&self, store: &Store, id: &str) -> Result<(), HubError> {
        let mut games = self.games.lock().await;
        let game = match games.remove(id) {
            Some(Game::Awake(game)) => game,
            Some(other) => {
                games.insert(id.to_string(), other);
                return Ok(());
            }
            None => return Ok(()),
        };
        let address = game.server.local_addr();
        let listener = match game.server.stop_keeping_listener().await {
            Some(listener) => listener,
            // Ne devrait pas arriver : on rouvre le port à la main.
            None => TcpListener::bind(address)
                .await
                .map_err(|e| HubError::Internal(format!("port {} : {e}", address.port())))?,
        };
        games.insert(
            id.to_string(),
            Game::Asleep(self.spawn_guard(id, listener, game.fingerprint)),
        );
        drop(games);
        store.set_game_asleep(id, true).await?;
        tracing::info!(game = %id, "partie hébergée en veille");
        Ok(())
    }

    /// Réveille une partie (`POST /v1/games/{id}/wake`, ou connexion sur son port).
    /// En veille : le serveur repart sur le port gardé, avec la connexion qui l'a
    /// réveillée s'il y en a une, et `wake_grace` pour qu'un joueur entre. Arrêtée :
    /// nouvel essai sur son port. Déjà éveillée : rien à faire.
    pub async fn wake(&self, store: &Store, id: &str) -> Result<(), HubError> {
        let mut games = self.games.lock().await;
        let listener = match games.remove(id) {
            Some(Game::Awake(game)) => {
                games.insert(id.to_string(), Game::Awake(game));
                return Ok(());
            }
            Some(Game::Asleep(asleep)) => {
                // La garde a peut-être déjà fini (connexion reçue) : l'envoi échoue
                // alors, ce qui ne change rien.
                let _ = asleep.cancel.send(());
                let exit = asleep
                    .guard
                    .await
                    .map_err(|e| HubError::Internal(format!("garde du port : {e}")))?;
                Some((exit.listener, exit.pending))
            }
            None => None,
        };
        let Some(row) = store.game(id).await? else {
            // Supprimée entre-temps : l'écouteur éventuel est lâché, le port se ferme.
            return Err(HubError::GameNotFound);
        };
        let bind = match listener {
            Some((listener, pending)) => Bind::Listener(listener, pending),
            None => Bind::Port(port_of(&row)?),
        };
        let key = GameKey::load_or_create(&self.config.data_dir)?;
        let mut game = self.relaunch(&key, &row, bind).await?;
        game.sleep_at = Instant::now() + self.config.wake_grace;
        games.insert(id.to_string(), Game::Awake(game));
        drop(games);
        store.set_game_asleep(id, false).await?;
        tracing::info!(game = %id, "partie hébergée réveillée");
        Ok(())
    }

    /// Retire une partie de la table et l'arrête : serveur arrêté (sauvegarde
    /// compactée) ou garde du port finie. Le port est fermé au retour.
    async fn stop(&self, id: &str) {
        let game = self.games.lock().await.remove(id);
        stop_game(game).await;
    }

    /// Arrête et supprime une partie : serveur, ligne de la base et dossier.
    pub async fn delete(&self, store: &Store, id: &str) -> Result<(), HubError> {
        self.stop(id).await;
        store.delete_game(id).await?;
        remove_game_dir(&self.game_dir(id));
        tracing::info!(game = %id, "partie hébergée supprimée");
        Ok(())
    }

    /// Supprime toutes les parties d'un compte (avant la suppression du compte).
    pub async fn delete_all_of(&self, store: &Store, owner_id: i64) -> Result<(), HubError> {
        for row in store.games_of(owner_id).await? {
            self.delete(store, &row.id).await?;
        }
        Ok(())
    }

    /// Zip de la seed et de la sauvegarde, pour reprendre la partie en self-host
    /// (`GET /v1/games/{id}/export`). La sauvegarde peut être lue pendant que le
    /// serveur y écrit : le format `.raftup` tolère une dernière ligne coupée.
    pub async fn export(&self, id: &str) -> Result<Vec<u8>, HubError> {
        let dir = self.game_dir(id);
        tokio::task::spawn_blocking(move || export_zip(&dir))
            .await
            .map_err(|e| HubError::Internal(format!("tâche d'export : {e}")))?
    }

    /// Arrête toutes les parties (arrêt du hub). Chaque sauvegarde est compactée ;
    /// les parties repartent au prochain lancement ([`GameHost::restore`]), en veille
    /// pour celles qui l'étaient.
    pub async fn shutdown(&self) {
        // `std::mem::take` vide la table sous le verrou, puis on arrête sans lui.
        let games = std::mem::take(&mut *self.games.lock().await);
        for (id, game) in games {
            stop_game(Some(game)).await;
            tracing::info!(game = %id, "partie hébergée arrêtée");
        }
    }
}

/// Arrête une partie sortie de la table (éveillée ou en veille).
async fn stop_game(game: Option<Game>) {
    match game {
        Some(Game::Awake(game)) => game.server.stop().await,
        Some(Game::Asleep(asleep)) => {
            let _ = asleep.cancel.send(());
            // L'écouteur rendu est lâché ici : le port se ferme.
            let _ = asleep.guard.await;
        }
        None => {}
    }
}

/// Garde le port d'une partie en veille : attend une connexion (et prévient alors
/// la tâche de veille par `wake`), ou la fin de la garde (`cancelled`).
async fn guard_port(
    id: String,
    listener: TcpListener,
    cancelled: oneshot::Receiver<()>,
    wake: mpsc::UnboundedSender<String>,
) -> GuardExit {
    // `select!` attend le premier des deux qui arrive ; l'autre est abandonné. Le
    // résultat est rangé dans `pending` avant de rendre l'écouteur, qu'`accept_one`
    // emprunte pendant l'attente.
    let pending = tokio::select! {
        connection = accept_one(&listener) => Some(connection),
        // Envoi ou canal fermé (partie supprimée, hub arrêté) : on rend la main.
        _ = cancelled => None,
    };
    if pending.is_some() {
        // Personne n'écoute plus (hub arrêté) : la connexion sera simplement fermée.
        let _ = wake.send(id);
    }
    GuardExit { listener, pending }
}

/// Accepte une connexion ; une erreur passagère (trop de fichiers ouverts…) fait
/// seulement attendre un peu.
async fn accept_one(listener: &TcpListener) -> (TcpStream, SocketAddr) {
    loop {
        match listener.accept().await {
            Ok(connection) => return connection,
            Err(error) => {
                tracing::warn!(%error, "connexion refusée sur un port en veille");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// Port d'une partie de la base.
fn port_of(row: &GameRow) -> Result<u16, HubError> {
    u16::try_from(row.port).map_err(|_| HubError::Internal("port de partie invalide".to_string()))
}

/// Relit une seed sur un thread à part : le pickle peut faire 32 Mio.
async fn load_seed(bytes: Vec<u8>) -> Result<MultiData, HubError> {
    tokio::task::spawn_blocking(move || load_archipelago(&bytes, &ArchiveLimits::default()))
        .await
        .map_err(|e| HubError::Internal(format!("tâche de lecture de la seed : {e}")))?
        .map_err(|e| HubError::InvalidSeed(e.to_string()))
}

/// Un identifiant de partie bien formé : 32 chiffres hexadécimaux en minuscules.
/// Toute autre valeur (`..`, `/`…) est refusée avant de toucher au disque.
pub fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn check_name(raw: &str) -> Result<String, HubError> {
    let name = raw.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(HubError::InvalidGameName(MAX_NAME_CHARS));
    }
    Ok(name.to_string())
}

fn check_password(raw: Option<String>) -> Result<Option<String>, HubError> {
    let Some(password) = raw.filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    if password.len() > MAX_PASSWORD_BYTES || password.chars().any(char::is_control) {
        return Err(HubError::BadRequest(format!(
            "mot de passe de la partie : {MAX_PASSWORD_BYTES} octets au plus, sans caractère de contrôle"
        )));
    }
    Ok(Some(password))
}

/// Écrit un fichier neuf dans le dossier de la partie (créé s'il manque).
fn write_new_file(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), HubError> {
    let error = |e: std::io::Error| HubError::Internal(format!("écriture de {name} : {e}"));
    std::fs::create_dir_all(dir).map_err(error)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(name))
        .map_err(error)?;
    file.write_all(bytes).map_err(error)?;
    file.sync_all().map_err(error)
}

/// Retire le dossier d'une partie. Un échec ne laisse qu'un avertissement : la
/// partie n'existe déjà plus pour le hub.
fn remove_game_dir(dir: &Path) {
    if dir.exists() {
        if let Err(error) = std::fs::remove_dir_all(dir) {
            tracing::warn!(dir = %dir.display(), %error, "dossier de partie non supprimé");
        }
    }
}

fn export_zip(dir: &Path) -> Result<Vec<u8>, HubError> {
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;
    let zip_error = |e: zip::result::ZipError| HubError::Internal(format!("zip : {e}"));
    let read_error = |e: std::io::Error| HubError::Internal(format!("lecture : {e}"));
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for name in [SEED_FILE, SAVE_FILE] {
        let path = dir.join(name);
        // Pas encore de sauvegarde : seulement la seed.
        if name == SAVE_FILE && !path.exists() {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(read_error)?;
        writer.start_file(name, options).map_err(zip_error)?;
        writer.write_all(&bytes).map_err(read_error)?;
    }
    Ok(writer.finish().map_err(zip_error)?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiants_et_plages() {
        assert!(valid_id("0123456789abcdef0123456789abcdef"));
        assert!(!valid_id("0123456789ABCDEF0123456789abcdef"));
        assert!(!valid_id("../../etc/passwd"));
        assert!(!valid_id(""));
        assert_eq!(
            GamesConfig::parse_ports("38281-38380"),
            Some((38281, 38380))
        );
        assert_eq!(GamesConfig::parse_ports(" 5 - 5 "), Some((5, 5)));
        assert_eq!(GamesConfig::parse_ports("38380-38281"), None);
        assert_eq!(GamesConfig::parse_ports("0-10"), None);
        assert_eq!(GamesConfig::parse_ports("abc"), None);
    }

    #[test]
    fn noms_et_mots_de_passe() {
        assert_eq!(
            check_name("  Ma partie ").ok().as_deref(),
            Some("Ma partie")
        );
        assert!(check_name("   ").is_err());
        assert!(check_name(&"x".repeat(65)).is_err());
        assert!(check_name("a\nb").is_err());
        assert_eq!(check_password(Some(String::new())).ok(), Some(None));
        assert!(check_password(Some("p".repeat(256))).is_ok());
        assert!(check_password(Some("é".repeat(129))).is_err());
    }
}
