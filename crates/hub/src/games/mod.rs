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
//! Pas encore fait (étapes 2 à 5 de l'ADR 0052) : veille et réveil, page « Parties »
//! de la console, export RGPD, déploiement.

pub mod cert;
pub mod routes;
pub mod secret;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use ap_protocol::SlotType;
use multidata::{load_archipelago, ArchiveLimits, MultiData};
use server::save::open_or_create;
use server::{Limits, Room, RunningServer, Server, ServerStatus, TlsConfig};
use tokio::sync::Mutex;

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
}

/// Les serveurs de jeu du hub.
pub struct GameHost {
    config: GamesConfig,
    /// Parties qui tournent, par identifiant.
    ///
    /// Mutex de tokio (et pas de `std`) : il reste pris pendant des `.await` (ouvrir
    /// un port, arrêter un serveur). Il sérialise aussi les créations : deux envois
    /// simultanés ne prennent pas le même port et ne dépassent pas les limites.
    running: Mutex<BTreeMap<String, Running>>,
}

impl GameHost {
    pub fn new(config: GamesConfig) -> GameHost {
        GameHost {
            config,
            running: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn config(&self) -> &GamesConfig {
        &self.config
    }

    /// Dossier d'une partie. L'identifiant est vérifié avant (voir [`valid_id`]).
    fn game_dir(&self, id: &str) -> PathBuf {
        self.config.data_dir.join(id)
    }

    /// Relance toutes les parties de la base (démarrage du hub). Une partie qui ne
    /// repart pas (fichier abîmé, port pris par un autre programme) reste arrêtée,
    /// avec une erreur au journal ; les autres repartent quand même.
    pub async fn restore(&self, store: &Store) -> Result<(), HubError> {
        let rows = store.all_games().await?;
        if rows.is_empty() {
            return Ok(());
        }
        let key = GameKey::load_or_create(&self.config.data_dir)?;
        let mut running = self.running.lock().await;
        for row in rows {
            if running.contains_key(&row.id) {
                continue;
            }
            match self.relaunch(&key, &row).await {
                Ok(game) => {
                    tracing::info!(game = %row.id, port = row.port, "partie hébergée reprise");
                    running.insert(row.id.clone(), game);
                }
                Err(error) => {
                    tracing::error!(game = %row.id, %error, "partie hébergée non reprise");
                }
            }
        }
        Ok(())
    }

    /// Relance une partie déjà enregistrée, sur son port.
    async fn relaunch(&self, key: &GameKey, row: &GameRow) -> Result<Running, HubError> {
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
        let port = u16::try_from(row.port)
            .map_err(|_| HubError::Internal("port de partie invalide".to_string()))?;
        let (running, _) = self
            .launch(&dir, data, &seed, password, admin_password, Some(port), &[])
            .await?;
        Ok(running)
    }

    /// Ouvre un port, la sauvegarde, puis démarre le serveur de jeu.
    ///
    /// `port` : celui de la partie (reprise), ou `None` pour prendre le premier port
    /// libre de la plage, sauf ceux de `reserved` (réservés par d'autres parties).
    /// Rend le serveur lancé et `true` si une sauvegarde existante a repris.
    #[allow(clippy::too_many_arguments)]
    async fn launch(
        &self,
        dir: &Path,
        mut data: MultiData,
        seed: &[u8],
        password: Option<String>,
        admin_password: String,
        port: Option<u16>,
        reserved: &[i64],
    ) -> Result<(Running, bool), HubError> {
        let certificate = cert::ensure(&self.config.data_dir.join("tls"))?;
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
        let server = match port {
            Some(port) => Server::bind((host, port), room)
                .await
                .map_err(|e| HubError::Internal(format!("port {port} : {e}")))?,
            None => self.bind_free_port(host, room, reserved).await?,
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
        let mut running = self.running.lock().await;
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
                    None,
                    &reserved,
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
                running.insert(id, game);
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

    /// Empreinte du certificat de la partie, si elle tourne.
    pub async fn fingerprint(&self, id: &str) -> Option<String> {
        let running = self.running.lock().await;
        running.get(id).map(|game| game.fingerprint.clone())
    }

    /// Photo de la partie (joueurs connectés, avancement), si elle tourne.
    pub async fn status(&self, id: &str) -> Option<ServerStatus> {
        let running = self.running.lock().await;
        // La photo est demandée à la salle sous le verrou : elle répond vite (une
        // lecture de son état), et le serveur ne peut pas être arrêté entre-temps.
        running.get(id)?.server.status().await
    }

    /// Arrête une partie (la sauvegarde est compactée), sans la supprimer.
    async fn stop(&self, id: &str) {
        let game = self.running.lock().await.remove(id);
        if let Some(game) = game {
            game.server.stop().await;
        }
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
    /// les parties repartent au prochain lancement ([`GameHost::restore`]).
    pub async fn shutdown(&self) {
        // `std::mem::take` vide la table sous le verrou, puis on arrête sans lui.
        let games = std::mem::take(&mut *self.running.lock().await);
        for (id, game) in games {
            game.server.stop().await;
            tracing::info!(game = %id, "partie hébergée arrêtée");
        }
    }
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
