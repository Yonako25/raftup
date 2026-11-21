//! Illustrations des jeux, prises chez IGDB et gardées en cache par le hub (ADR 0047).
//!
//! Le launcher ne parle jamais à IGDB : il demande au hub la fiche d'un jeu
//! (`/v1/art/steam/{appid}` ou `/v1/art/igdb/{id}`), puis ses images
//! (`/v1/art/images/{image_id}/{kind}`). Le hub :
//! 1. regarde son cache disque (fiches en JSON, images en JPEG), valable 30 jours ;
//! 2. sinon demande à IGDB, avec la clé de **son** application Twitch (réglages
//!    `igdb_client_id` et `igdb_client_secret`), au rythme de 4 requêtes par seconde ;
//! 3. range la réponse, et retire les fichiers les plus anciens au-delà du plafond.
//!
//! Si IGDB ne répond pas, une entrée périmée du cache sert encore. Les demandes qui
//! ratent le cache sont limitées pour tout le hub : il ne devient pas un proxy
//! ouvert vers IGDB.
//!
//! Le réseau passe par le trait [`ArtSource`] : [`IgdbSource`] pour de vrai,
//! [`FakeArt`] dans les tests.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::memory::RateLimiter;

/// Texte d'attribution demandé par IGDB, renvoyé au launcher.
pub const ATTRIBUTION: &str = "IGDB.com";
/// Lien d'attribution quand IGDB ne donne pas la page du jeu.
pub const ATTRIBUTION_URL: &str = "https://www.igdb.com";

/// Adresses d'IGDB et de Twitch (documentation lue le 21 novembre 2026).
const TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";
const API_URL: &str = "https://api.igdb.com/v4";
const IMAGE_URL: &str = "https://images.igdb.com/igdb/image/upload";

/// Source « Steam » des `external_games` d'IGDB (`external_game_source`, l'ancien
/// champ `category` valait aussi 1). À confirmer par le test réel.
pub const IGDB_STEAM_SOURCE: u32 = 1;

/// IGDB permet 4 requêtes par seconde : une toutes les 250 ms.
const MIN_INTERVAL: Duration = Duration::from_millis(250);
/// Réponse JSON d'IGDB ou de Twitch : 1 Mio au plus.
const MAX_JSON_BYTES: u64 = 1024 * 1024;
/// Image reçue d'IGDB : 4 Mio au plus.
pub const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;
/// Longueur maximale d'un identifiant d'image IGDB (ils font une vingtaine de lettres).
const MAX_IMAGE_ID_CHARS: usize = 64;

/// Clé de l'application Twitch qui donne accès à IGDB.
#[derive(Clone, PartialEq, Eq)]
pub struct IgdbCredentials {
    pub client_id: String,
    pub client_secret: String,
}

/// `Debug` écrit à la main : le secret n'apparaît jamais dans un journal.
impl fmt::Debug for IgdbCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IgdbCredentials")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// Sorte d'image demandée.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArtKind {
    /// Jaquette en portrait (environ 2:3).
    Cover,
    /// Image large de la fiche d'un jeu (16:9).
    Hero,
}

impl ArtKind {
    pub fn parse(text: &str) -> Option<ArtKind> {
        match text {
            "cover" => Some(ArtKind::Cover),
            "hero" => Some(ArtKind::Hero),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ArtKind::Cover => "cover",
            ArtKind::Hero => "hero",
        }
    }

    /// Taille d'image d'IGDB : `cover_big_2x` (528 × 748) et `screenshot_huge`
    /// (1280 × 720, recadrée au centre).
    fn igdb_size(self) -> &'static str {
        match self {
            ArtKind::Cover => "cover_big_2x",
            ArtKind::Hero => "screenshot_huge",
        }
    }
}

/// Fiche d'illustrations d'un jeu.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameArt {
    pub igdb_id: u64,
    /// Identifiant d'image IGDB de la jaquette.
    pub cover: Option<String>,
    /// Identifiant d'image IGDB de l'image large : premier « artwork », sinon
    /// première capture.
    pub hero: Option<String>,
    /// Page du jeu chez IGDB (lien d'attribution).
    pub url: Option<String>,
}

/// Ce qui empêche de rendre une illustration.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ArtError {
    /// Aucune application Twitch réglée sur ce hub.
    #[error("illustrations non configurées sur ce hub")]
    Disabled,
    /// IGDB ne connaît pas ce jeu (ou ce jeu n'a pas cette image).
    #[error("aucune illustration pour ce jeu")]
    NotFound,
    /// IGDB ou Twitch ne répond pas comme prévu, et rien en cache.
    /// Le texte ne contient jamais la clé.
    #[error("IGDB ne répond pas : {0}")]
    Unavailable(String),
    /// Trop de demandes hors cache en ce moment.
    #[error("trop de demandes d'illustrations, réessayer plus tard")]
    RateLimited,
    /// Identifiant d'image ou de jeu refusé.
    #[error("identifiant refusé")]
    BadId,
    /// Image plus grosse que [`MAX_IMAGE_BYTES`].
    #[error("image trop grosse")]
    TooLarge,
    /// Les octets reçus ne sont pas un JPEG.
    #[error("le fichier reçu n'est pas une image JPEG")]
    NotAnImage,
}

/// Ce que le hub demande à IGDB.
///
/// Synchrone (le client HTTP bloque le thread) : le hub l'appelle sur un thread à
/// part. `Send + Sync` : partagé entre les threads du serveur.
pub trait ArtSource: Send + Sync {
    /// Identifiant IGDB du jeu Steam `appid`, s'il est connu.
    fn steam_game(&self, appid: u32) -> Result<Option<u64>, ArtError>;
    /// Fiche d'illustrations du jeu IGDB `igdb_id`, s'il existe.
    fn game_art(&self, igdb_id: u64) -> Result<Option<GameArt>, ArtError>;
    /// Octets JPEG de l'image `image_id` dans la taille de `kind`.
    fn image(&self, image_id: &str, kind: ArtKind) -> Result<Vec<u8>, ArtError>;
}

/// Vrai si `id` peut servir d'identifiant d'image (et donc de nom de fichier) :
/// lettres minuscules et chiffres ASCII seulement.
pub fn valid_image_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_IMAGE_ID_CHARS
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Vrai si `bytes` commence comme un JPEG (`FF D8 FF`).
pub fn looks_like_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
}

// ---------------------------------------------------------------------------
// Requêtes et réponses d'IGDB (langage « Apicalypse »)
// ---------------------------------------------------------------------------

/// Corps de `POST /v4/external_games` : le jeu IGDB d'un appid Steam.
pub fn steam_query(appid: u32) -> String {
    format!("fields game; where uid = \"{appid}\" & external_game_source = {IGDB_STEAM_SOURCE}; limit 1;")
}

/// Corps de `POST /v4/games` : images et page d'un jeu.
pub fn game_query(igdb_id: u64) -> String {
    format!(
        "fields cover.image_id, artworks.image_id, screenshots.image_id, url; where id = {igdb_id}; limit 1;"
    )
}

#[derive(Deserialize)]
struct ExternalGameJson {
    #[serde(default)]
    game: Option<u64>,
}

#[derive(Deserialize)]
struct ImageRefJson {
    #[serde(default)]
    image_id: Option<String>,
}

#[derive(Deserialize)]
struct GameJson {
    id: u64,
    #[serde(default)]
    cover: Option<ImageRefJson>,
    #[serde(default)]
    artworks: Vec<ImageRefJson>,
    #[serde(default)]
    screenshots: Vec<ImageRefJson>,
    #[serde(default)]
    url: Option<String>,
}

/// Lit la réponse de `external_games` : le premier jeu trouvé.
pub fn parse_external_games(text: &str) -> Result<Option<u64>, ArtError> {
    let list: Vec<ExternalGameJson> = serde_json::from_str(text)
        .map_err(|_| ArtError::Unavailable("réponse external_games illisible".to_string()))?;
    Ok(list.into_iter().find_map(|entry| entry.game))
}

/// Lit la réponse de `games`. Un identifiant d'image hors `[a-z0-9]` ou une page
/// hors `https://www.igdb.com/` est ignoré : ils finissent dans des chemins et des
/// liens.
pub fn parse_games(text: &str) -> Result<Option<GameArt>, ArtError> {
    let list: Vec<GameJson> = serde_json::from_str(text)
        .map_err(|_| ArtError::Unavailable("réponse games illisible".to_string()))?;
    let Some(game) = list.into_iter().next() else {
        return Ok(None);
    };
    // `filter` sur une `Option` : garde la valeur seulement si elle est valide.
    let clean = |image: Option<ImageRefJson>| {
        image
            .and_then(|i| i.image_id)
            .filter(|id| valid_image_id(id))
    };
    let cover = clean(game.cover);
    let hero = game
        .artworks
        .into_iter()
        .chain(game.screenshots)
        .find_map(|image| clean(Some(image)));
    let url = game
        .url
        .filter(|u| u.starts_with("https://www.igdb.com/") && u.len() <= 256);
    Ok(Some(GameArt {
        igdb_id: game.id,
        cover,
        hero,
        url,
    }))
}

// ---------------------------------------------------------------------------
// Vraie source : IGDB par HTTPS
// ---------------------------------------------------------------------------

/// Client d'IGDB. Garde le jeton Twitch en mémoire et espace les requêtes.
pub struct IgdbSource {
    credentials: IgdbCredentials,
    agent: ureq::Agent,
    /// Jeton d'accès et instant où il expire.
    token: Mutex<Option<(String, Instant)>>,
    /// Instant à partir duquel la prochaine requête à l'API peut partir.
    next_slot: Mutex<Instant>,
}

#[derive(Deserialize)]
struct TokenJson {
    access_token: String,
    expires_in: u64,
}

impl IgdbSource {
    pub fn new(credentials: IgdbCredentials) -> IgdbSource {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .http_status_as_error(true)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        IgdbSource {
            credentials,
            agent,
            token: Mutex::new(None),
            next_slot: Mutex::new(Instant::now()),
        }
    }

    pub fn credentials(&self) -> &IgdbCredentials {
        &self.credentials
    }

    /// Attend son tour : une requête toutes les [`MIN_INTERVAL`] au plus.
    fn pace(&self) {
        // Le bloc rend l'attente et relâche le verrou avant de dormir : les autres
        // threads réservent leur tour pendant ce temps.
        let wait = {
            let mut next = lock(&self.next_slot);
            let now = Instant::now();
            let slot = (*next).max(now);
            *next = slot + MIN_INTERVAL;
            slot - now
        };
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }

    /// Jeton d'accès valable, demandé à Twitch si besoin.
    fn access_token(&self) -> Result<String, ArtError> {
        if let Some((token, expires)) = lock(&self.token).as_ref() {
            if Instant::now() < *expires {
                return Ok(token.clone());
            }
        }
        // Identifiant et secret dans le corps, pas dans l'adresse : une adresse peut
        // finir dans un message d'erreur.
        let body = format!(
            "client_id={}&client_secret={}&grant_type=client_credentials",
            self.credentials.client_id, self.credentials.client_secret
        );
        self.pace();
        let mut response = self
            .agent
            .post(TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(body.as_str())
            .map_err(|e| ArtError::Unavailable(format!("Twitch : {}", describe(&e))))?;
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_JSON_BYTES)
            .read_to_string()
            .map_err(|e| ArtError::Unavailable(format!("Twitch : {}", describe(&e))))?;
        let token: TokenJson = serde_json::from_str(&text)
            .map_err(|_| ArtError::Unavailable("jeton Twitch illisible".to_string()))?;
        // Une minute de marge pour ne pas envoyer un jeton qui expire en route.
        let lifetime = Duration::from_secs(token.expires_in.saturating_sub(60));
        *lock(&self.token) = Some((token.access_token.clone(), Instant::now() + lifetime));
        Ok(token.access_token)
    }

    /// `POST /v4/<endpoint>` avec `query`. Un jeton refusé (401) est redemandé une fois.
    fn api(&self, endpoint: &str, query: &str) -> Result<String, ArtError> {
        for attempt in 0..2 {
            let token = self.access_token()?;
            self.pace();
            let result = self
                .agent
                .post(&format!("{API_URL}/{endpoint}"))
                .header("Client-ID", &self.credentials.client_id)
                .header("Authorization", &format!("Bearer {token}"))
                .header("Accept", "application/json")
                .send(query);
            match result {
                Ok(mut response) => {
                    return response
                        .body_mut()
                        .with_config()
                        .limit(MAX_JSON_BYTES)
                        .read_to_string()
                        .map_err(|e| ArtError::Unavailable(describe(&e)));
                }
                Err(ureq::Error::StatusCode(401)) if attempt == 0 => {
                    // Jeton révoqué ou expiré plus tôt que prévu : on l'oublie.
                    *lock(&self.token) = None;
                }
                Err(error) => return Err(ArtError::Unavailable(describe(&error))),
            }
        }
        Err(ArtError::Unavailable("jeton Twitch refusé".to_string()))
    }
}

impl ArtSource for IgdbSource {
    fn steam_game(&self, appid: u32) -> Result<Option<u64>, ArtError> {
        parse_external_games(&self.api("external_games", &steam_query(appid))?)
    }

    fn game_art(&self, igdb_id: u64) -> Result<Option<GameArt>, ArtError> {
        parse_games(&self.api("games", &game_query(igdb_id))?)
    }

    fn image(&self, image_id: &str, kind: ArtKind) -> Result<Vec<u8>, ArtError> {
        if !valid_image_id(image_id) {
            return Err(ArtError::BadId);
        }
        let url = format!("{IMAGE_URL}/t_{}/{image_id}.jpg", kind.igdb_size());
        let mut response = self.agent.get(&url).call().map_err(|e| match e {
            ureq::Error::StatusCode(404) => ArtError::NotFound,
            other => ArtError::Unavailable(describe(&other)),
        })?;
        response
            .body_mut()
            .with_config()
            .limit(MAX_IMAGE_BYTES)
            .read_to_vec()
            .map_err(|e| match e {
                ureq::Error::BodyExceedsLimit(_) => ArtError::TooLarge,
                other => ArtError::Unavailable(describe(&other)),
            })
    }
}

/// Erreur réseau en quelques mots, sans adresse ni corps (rien de secret).
fn describe(error: &ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(code) => format!("code HTTP {code}"),
        ureq::Error::Timeout(_) => "délai dépassé".to_string(),
        ureq::Error::BodyExceedsLimit(_) => "réponse trop grosse".to_string(),
        _ => "connexion impossible".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Fausse source pour les tests
// ---------------------------------------------------------------------------

/// Source en mémoire, sans réseau. Compte les appels pour vérifier le cache.
#[derive(Default)]
pub struct FakeArt {
    pub steam: BTreeMap<u32, u64>,
    pub games: BTreeMap<u64, GameArt>,
    /// Images par (identifiant, sorte).
    pub images: BTreeMap<(String, ArtKind), Vec<u8>>,
    /// Vrai : chaque appel répond « IGDB ne répond pas ».
    pub down: AtomicBool,
    /// Nombre d'appels reçus.
    pub calls: AtomicUsize,
}

impl FakeArt {
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
    }

    fn call(&self) -> Result<(), ArtError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.down.load(Ordering::SeqCst) {
            Err(ArtError::Unavailable("panne simulée".to_string()))
        } else {
            Ok(())
        }
    }
}

impl ArtSource for FakeArt {
    fn steam_game(&self, appid: u32) -> Result<Option<u64>, ArtError> {
        self.call()?;
        Ok(self.steam.get(&appid).copied())
    }

    fn game_art(&self, igdb_id: u64) -> Result<Option<GameArt>, ArtError> {
        self.call()?;
        Ok(self.games.get(&igdb_id).cloned())
    }

    fn image(&self, image_id: &str, kind: ArtKind) -> Result<Vec<u8>, ArtError> {
        self.call()?;
        self.images
            .get(&(image_id.to_string(), kind))
            .cloned()
            .ok_or(ArtError::NotFound)
    }
}

// ---------------------------------------------------------------------------
// Service : cache disque + source
// ---------------------------------------------------------------------------

/// Horloge en secondes Unix. `Arc<dyn Fn …>` : une fonction partagée entre threads,
/// remplacée par une fausse horloge dans les tests.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Réglages des illustrations.
#[derive(Clone)]
pub struct ArtConfig {
    /// Dossier du cache (créé au premier besoin).
    pub cache_dir: PathBuf,
    /// Plafond du cache, en octets.
    pub max_cache_bytes: u64,
    /// Durée de validité d'une entrée trouvée (30 jours).
    pub fresh_secs: i64,
    /// Durée pendant laquelle un jeu introuvable n'est pas redemandé (7 jours).
    pub missing_secs: i64,
    /// Demandes hors cache permises par minute, pour tout le hub.
    pub misses_per_minute: u32,
    /// Source fixée (tests). `None` : IGDB, avec la clé des réglages.
    pub source: Option<Arc<dyn ArtSource>>,
    pub clock: Clock,
}

impl Default for ArtConfig {
    fn default() -> Self {
        ArtConfig {
            cache_dir: PathBuf::from("rafthub-cache").join("art"),
            max_cache_bytes: 256 * 1024 * 1024,
            fresh_secs: 30 * 24 * 3600,
            missing_secs: 7 * 24 * 3600,
            misses_per_minute: 120,
            source: None,
            clock: Arc::new(crate::now_unix),
        }
    }
}

/// `Debug` écrit à la main : la source et l'horloge n'en ont pas.
impl fmt::Debug for ArtConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArtConfig")
            .field("cache_dir", &self.cache_dir)
            .field("max_cache_bytes", &self.max_cache_bytes)
            .field("fixed_source", &self.source.is_some())
            .finish_non_exhaustive()
    }
}

/// Fiche rangée dans le cache. `None` : jeu introuvable chez IGDB.
#[derive(Serialize, Deserialize)]
struct SteamEntry {
    igdb_id: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct GameEntry {
    art: Option<GameArt>,
}

/// Ce que le cache sait d'une clé.
enum Cached<T> {
    /// Entrée assez récente : on la sert telle quelle.
    Fresh(T),
    /// Entrée périmée : servie seulement si IGDB ne répond pas.
    Stale(T),
    Missing,
}

/// Illustrations : cache disque devant la source.
pub struct ArtService {
    config: ArtConfig,
    /// Client IGDB construit depuis les réglages, gardé tant que la clé ne change
    /// pas (il garde son jeton et son rythme).
    live: Mutex<Option<Arc<IgdbSource>>>,
    misses: RateLimiter,
    /// Une seule éviction à la fois.
    evicting: Mutex<()>,
}

const MISS_KEY: &str = "igdb";

impl ArtService {
    pub fn new(config: ArtConfig) -> ArtService {
        let misses = RateLimiter::new(config.misses_per_minute, Duration::from_secs(60));
        ArtService {
            config,
            live: Mutex::new(None),
            misses,
            evicting: Mutex::new(()),
        }
    }

    /// Source à utiliser, ou [`ArtError::Disabled`] sans clé ni source fixée.
    fn source(&self, credentials: Option<IgdbCredentials>) -> Result<Arc<dyn ArtSource>, ArtError> {
        if let Some(source) = &self.config.source {
            return Ok(Arc::clone(source));
        }
        let credentials = credentials.ok_or(ArtError::Disabled)?;
        let mut live = lock(&self.live);
        let reuse = live
            .as_ref()
            .filter(|source| *source.credentials() == credentials)
            .cloned();
        let source = match reuse {
            Some(source) => source,
            None => {
                let source = Arc::new(IgdbSource::new(credentials));
                *live = Some(Arc::clone(&source));
                source
            }
        };
        // Conversion d'un `Arc<IgdbSource>` en `Arc<dyn ArtSource>` : même pointeur,
        // vu à travers le trait.
        Ok(source as Arc<dyn ArtSource>)
    }

    /// Compte une demande hors cache ; [`ArtError::RateLimited`] au-delà de la limite.
    fn count_miss(&self) -> Result<(), ArtError> {
        if self.misses.hit(MISS_KEY) {
            Ok(())
        } else {
            Err(ArtError::RateLimited)
        }
    }

    /// Fiche du jeu Steam `appid`.
    pub fn by_steam(
        &self,
        credentials: Option<IgdbCredentials>,
        appid: u32,
    ) -> Result<GameArt, ArtError> {
        let source = self.source(credentials)?;
        let path = self.games_dir().join(format!("steam-{appid}.json"));
        let entry: SteamEntry = match self.read_entry(&path, |e: &SteamEntry| e.igdb_id.is_some()) {
            Cached::Fresh(entry) => entry,
            cached => {
                let fetched = self
                    .count_miss()
                    .and_then(|()| source.steam_game(appid))
                    .map(|igdb_id| SteamEntry { igdb_id });
                match (fetched, cached) {
                    (Ok(entry), _) => {
                        self.write_entry(&path, &entry);
                        entry
                    }
                    (Err(ArtError::Unavailable(_)), Cached::Stale(entry)) => entry,
                    (Err(error), _) => return Err(error),
                }
            }
        };
        let igdb_id = entry.igdb_id.ok_or(ArtError::NotFound)?;
        self.game(&*source, igdb_id)
    }

    /// Fiche du jeu IGDB `igdb_id`.
    pub fn by_igdb(
        &self,
        credentials: Option<IgdbCredentials>,
        igdb_id: u64,
    ) -> Result<GameArt, ArtError> {
        let source = self.source(credentials)?;
        self.game(&*source, igdb_id)
    }

    fn game(&self, source: &dyn ArtSource, igdb_id: u64) -> Result<GameArt, ArtError> {
        let path = self.games_dir().join(format!("igdb-{igdb_id}.json"));
        let entry: GameEntry = match self.read_entry(&path, |e: &GameEntry| e.art.is_some()) {
            Cached::Fresh(entry) => entry,
            cached => {
                let fetched = self
                    .count_miss()
                    .and_then(|()| source.game_art(igdb_id))
                    .map(|art| GameEntry { art });
                match (fetched, cached) {
                    (Ok(entry), _) => {
                        self.write_entry(&path, &entry);
                        entry
                    }
                    (Err(ArtError::Unavailable(_)), Cached::Stale(entry)) => entry,
                    (Err(error), _) => return Err(error),
                }
            }
        };
        entry.art.ok_or(ArtError::NotFound)
    }

    /// Octets JPEG de l'image `image_id`.
    pub fn image(
        &self,
        credentials: Option<IgdbCredentials>,
        image_id: &str,
        kind: ArtKind,
    ) -> Result<Vec<u8>, ArtError> {
        if !valid_image_id(image_id) {
            return Err(ArtError::BadId);
        }
        let source = self.source(credentials)?;
        let path = self
            .images_dir()
            .join(format!("{image_id}-{}.jpg", kind.as_str()));
        let stale = match fs::read(&path) {
            Ok(bytes) if looks_like_jpeg(&bytes) => {
                if self.is_fresh(&path, self.config.fresh_secs) {
                    return Ok(bytes);
                }
                Some(bytes)
            }
            _ => None,
        };
        let fetched = self
            .count_miss()
            .and_then(|()| source.image(image_id, kind))
            .and_then(|bytes| {
                if bytes.len() as u64 > MAX_IMAGE_BYTES {
                    Err(ArtError::TooLarge)
                } else if !looks_like_jpeg(&bytes) {
                    Err(ArtError::NotAnImage)
                } else {
                    Ok(bytes)
                }
            });
        match (fetched, stale) {
            (Ok(bytes), _) => {
                self.write_cached(&path, &bytes);
                Ok(bytes)
            }
            (Err(ArtError::Unavailable(_)), Some(bytes)) => Ok(bytes),
            (Err(error), _) => Err(error),
        }
    }

    fn games_dir(&self) -> PathBuf {
        self.config.cache_dir.join("games")
    }

    fn images_dir(&self) -> PathBuf {
        self.config.cache_dir.join("images")
    }

    /// Lit une entrée JSON du cache. `found` dit si l'entrée a trouvé quelque chose :
    /// une entrée « introuvable » vit moins longtemps.
    ///
    /// `impl Fn(&T) -> bool` : n'importe quelle fonction qui regarde l'entrée.
    fn read_entry<T: DeserializeOwned>(
        &self,
        path: &Path,
        found: impl Fn(&T) -> bool,
    ) -> Cached<T> {
        let Ok(text) = fs::read_to_string(path) else {
            return Cached::Missing;
        };
        let Ok(entry) = serde_json::from_str::<T>(&text) else {
            return Cached::Missing;
        };
        let lifetime = if found(&entry) {
            self.config.fresh_secs
        } else {
            self.config.missing_secs
        };
        if self.is_fresh(path, lifetime) {
            Cached::Fresh(entry)
        } else {
            Cached::Stale(entry)
        }
    }

    fn write_entry<T: Serialize>(&self, path: &Path, entry: &T) {
        match serde_json::to_vec(entry) {
            Ok(bytes) => self.write_cached(path, &bytes),
            Err(error) => tracing::warn!(%error, "fiche d'illustration non rangée"),
        }
    }

    /// Vrai si le fichier a été écrit il y a moins de `lifetime` secondes (selon
    /// l'horloge du service : sa date de modification est celle de l'écriture).
    fn is_fresh(&self, path: &Path, lifetime: i64) -> bool {
        let Some(written) = modified_secs(path) else {
            return false;
        };
        (self.config.clock)() - written < lifetime
    }

    /// Range un fichier dans le cache (écriture atomique), daté par l'horloge du
    /// service, puis fait de la place si le plafond est dépassé. Un échec ne laisse
    /// qu'un avertissement : la réponse part quand même.
    fn write_cached(&self, path: &Path, bytes: &[u8]) {
        let now = (self.config.clock)();
        if let Err(error) = write_dated(path, bytes, now) {
            tracing::warn!(%error, path = %path.display(), "cache des illustrations : écriture impossible");
            return;
        }
        self.evict();
    }

    /// Retire les fichiers les plus anciens tant que le cache dépasse son plafond
    /// (jusqu'à 90 % du plafond, pour ne pas recommencer à chaque écriture).
    fn evict(&self) {
        let _guard = lock(&self.evicting);
        let mut files = Vec::new();
        for dir in [self.games_dir(), self.images_dir()] {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                if !meta.is_file() {
                    continue;
                }
                let written = modified_secs(&path).unwrap_or(0);
                files.push((written, path, meta.len()));
            }
        }
        let mut total: u64 = files.iter().map(|(_, _, size)| size).sum();
        if total <= self.config.max_cache_bytes {
            return;
        }
        // Plus anciens d'abord ; à date égale, par chemin (ordre stable).
        files.sort();
        let target = self.config.max_cache_bytes / 10 * 9;
        for (_, path, size) in files {
            if total <= target {
                break;
            }
            match fs::remove_file(&path) {
                Ok(()) => total = total.saturating_sub(size),
                Err(error) => {
                    tracing::warn!(%error, path = %path.display(), "cache des illustrations : fichier non retiré")
                }
            }
        }
    }
}

/// Date de modification d'un fichier, en secondes Unix.
fn modified_secs(path: &Path) -> Option<i64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let secs = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    i64::try_from(secs).ok()
}

/// Écrit `bytes` dans `path` par un fichier temporaire renommé (jamais de fichier à
/// moitié écrit), et lui donne la date `at` (secondes Unix).
fn write_dated(path: &Path, bytes: &[u8], at: i64) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    {
        // Bloc : le fichier est fermé à la sortie, avant le renommage.
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        let date = UNIX_EPOCH + Duration::from_secs(u64::try_from(at).unwrap_or(0));
        file.set_modified(date)?;
    }
    fs::rename(&temp, path)
}

/// Verrouille un `Mutex` même si un autre thread a paniqué en le tenant.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiants_d_image() {
        assert!(valid_image_id("co1abc"));
        assert!(!valid_image_id(""));
        assert!(!valid_image_id("../secret"));
        assert!(!valid_image_id("CO1ABC"));
        assert!(!valid_image_id(&"a".repeat(65)));
    }

    #[test]
    fn requetes_apicalypse() {
        assert_eq!(
            steam_query(433340),
            "fields game; where uid = \"433340\" & external_game_source = 1; limit 1;"
        );
        assert!(game_query(42).contains("where id = 42;"));
    }

    #[test]
    fn lecture_des_reponses() {
        assert_eq!(
            parse_external_games(r#"[{"id":9,"game":1234}]"#),
            Ok(Some(1234))
        );
        assert_eq!(parse_external_games("[]"), Ok(None));
        assert!(parse_external_games("pas du json").is_err());

        let text = r#"[{"id":1234,"cover":{"id":1,"image_id":"co1abc"},
            "artworks":[{"id":2,"image_id":"../mal"},{"id":3,"image_id":"ar2xyz"}],
            "screenshots":[{"id":4,"image_id":"sc3def"}],
            "url":"https://www.igdb.com/games/slime-rancher"}]"#;
        let art = parse_games(text).ok().flatten();
        assert_eq!(
            art,
            Some(GameArt {
                igdb_id: 1234,
                cover: Some("co1abc".to_string()),
                // L'identifiant douteux est sauté.
                hero: Some("ar2xyz".to_string()),
                url: Some("https://www.igdb.com/games/slime-rancher".to_string()),
            })
        );
        // Sans artwork : première capture ; page hors IGDB ignorée.
        let text =
            r#"[{"id":5,"screenshots":[{"image_id":"sc9"}],"url":"https://ailleurs.example/"}]"#;
        let art = parse_games(text).ok().flatten();
        assert_eq!(art.as_ref().and_then(|a| a.hero.as_deref()), Some("sc9"));
        assert_eq!(art.and_then(|a| a.url), None);
        assert_eq!(parse_games("[]"), Ok(None));
    }

    #[test]
    fn secret_masque() {
        let credentials = IgdbCredentials {
            client_id: "id".to_string(),
            client_secret: "chut".to_string(),
        };
        assert!(!format!("{credentials:?}").contains("chut"));
    }
}
