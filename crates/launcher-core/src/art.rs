//! Illustrations des jeux, demandées au hub (ADR 0047).
//!
//! Le hub les prend chez IGDB et les garde en cache ; le launcher ne parle jamais à
//! IGDB. Ce module :
//! - trouve la référence d'un jeu dans son profil (`igdb_id`, sinon `steam_appid`) ;
//! - demande la fiche du jeu (`GET /v1/art/steam/{appid}` ou `/v1/art/igdb/{id}`),
//!   puis l'image (`GET /v1/art/images/{image_id}/{cover|hero}`) ;
//! - garde fiches et images dans un cache disque par hub, valable 30 jours. Hors
//!   ligne, une entrée périmée sert encore.
//!
//! Aucune dépendance au hub (AGPL, ADR 0003) : les types JSON sont redéfinis ici.
//! Le client est bloquant (`ureq`) : l'interface l'appelle dans un thread à part.
//! Sans hub, sans illustration ou sur une erreur, l'interface garde ses aplats.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::hub_client::HubUrl;
use crate::profile::Profile;

/// Délai d'un appel au hub.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Fiche JSON : 64 Kio au plus.
const MAX_JSON_BYTES: u64 = 64 * 1024;
/// Image : 4 Mio au plus (comme le hub).
pub const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;
/// Entrée trouvée valable 30 jours.
pub const FRESH_SECS: i64 = 30 * 24 * 3600;
/// Jeu inconnu du hub : pas redemandé pendant 7 jours.
pub const MISSING_SECS: i64 = 7 * 24 * 3600;

/// Comment le hub retrouve un jeu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArtRef {
    Igdb(u64),
    Steam(u32),
}

impl ArtRef {
    /// Référence d'un profil : `igdb_id` d'abord, sinon l'appid Steam. `None` : le jeu
    /// garde son aplat.
    pub fn of(profile: &Profile) -> Option<ArtRef> {
        profile.game.igdb_id.map(ArtRef::Igdb).or(profile
            .game
            .detect
            .steam_appid
            .map(ArtRef::Steam))
    }

    fn route(self) -> String {
        match self {
            ArtRef::Igdb(id) => format!("/v1/art/igdb/{id}"),
            ArtRef::Steam(appid) => format!("/v1/art/steam/{appid}"),
        }
    }

    fn cache_name(self) -> String {
        match self {
            ArtRef::Igdb(id) => format!("igdb-{id}.json"),
            ArtRef::Steam(appid) => format!("steam-{appid}.json"),
        }
    }
}

/// Sorte d'image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArtKind {
    /// Jaquette en portrait (environ 2:3), pour les cartes de la Bibliothèque.
    Cover,
    /// Image large (16:9), pour le haut de la fiche d'un jeu.
    Hero,
}

impl ArtKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArtKind::Cover => "cover",
            ArtKind::Hero => "hero",
        }
    }
}

/// Fiche d'illustrations d'un jeu, telle que le hub la renvoie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameArt {
    pub igdb_id: u64,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub hero: Option<String>,
    /// Texte d'attribution à afficher avec les images (« IGDB.com »).
    pub attribution: String,
    pub attribution_url: String,
}

impl GameArt {
    /// Identifiant d'image de cette sorte, s'il y en a un.
    pub fn image_id(&self, kind: ArtKind) -> Option<&str> {
        match kind {
            ArtKind::Cover => self.cover.as_deref(),
            ArtKind::Hero => self.hero.as_deref(),
        }
    }

    /// Retire ce qui ne peut pas servir sans risque : identifiants d'image hors
    /// `[a-z0-9]` (ils deviennent des noms de fichiers), lien qui n'est pas en
    /// `https://`, textes trop longs.
    fn sanitized(mut self) -> GameArt {
        self.cover = self.cover.filter(|id| valid_image_id(id));
        self.hero = self.hero.filter(|id| valid_image_id(id));
        if self.attribution.chars().count() > 64 {
            self.attribution = self.attribution.chars().take(64).collect();
        }
        if !self.attribution_url.starts_with("https://") || self.attribution_url.len() > 256 {
            self.attribution_url = String::new();
        }
        self
    }
}

/// Une image prête à afficher, avec son attribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtImage {
    /// Octets JPEG.
    pub bytes: Vec<u8>,
    pub attribution: String,
    pub attribution_url: String,
}

/// Ce qui empêche d'avoir une illustration. L'interface n'affiche rien de tout ça :
/// elle garde l'aplat (et le journal garde la raison).
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ArtError {
    #[error("illustrations non configurées sur ce hub")]
    Disabled,
    #[error("hub injoignable : {0}")]
    Network(String),
    #[error("réponse inattendue du hub : {0}")]
    Unexpected(String),
    #[error("cache des illustrations : {0}")]
    Io(String),
}

/// Vrai si `id` peut servir d'identifiant d'image : lettres minuscules et chiffres.
pub fn valid_image_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Vrai si `bytes` commence comme un JPEG (`FF D8 FF`).
pub fn looks_like_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
}

/// Horloge en secondes Unix, remplaçable dans les tests.
///
/// `Arc<dyn Fn …>` : une fonction partagée (le client part dans un thread).
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

fn system_clock() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Fiche rangée dans le cache. `None` : jeu inconnu du hub.
#[derive(Serialize, Deserialize)]
struct Entry {
    art: Option<GameArt>,
}

/// Ce que le cache sait d'une fiche ou d'une image.
enum Cached<T> {
    Fresh(T),
    Stale(T),
    Missing,
}

/// Client des illustrations d'un hub, avec son cache disque.
///
/// `Clone` : bon marché (l'agent `ureq` et l'horloge sont partagés), pour envoyer
/// une copie dans chaque thread.
#[derive(Clone)]
pub struct ArtClient {
    url: HubUrl,
    agent: ureq::Agent,
    dir: PathBuf,
    clock: Clock,
}

impl std::fmt::Debug for ArtClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtClient")
            .field("url", &self.url)
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl ArtClient {
    /// Client du hub `url`. Le cache va dans `<cache_root>/art/<hôte du hub>/` : deux
    /// hubs n'y mélangent pas leurs fiches.
    pub fn new(url: HubUrl, cache_root: &Path) -> ArtClient {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            // Comme `hub_client` : aucune redirection, les codes d'erreur sont lus par nous.
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        let dir = cache_root.join("art").join(folder_name(&url));
        ArtClient {
            url,
            agent,
            dir,
            clock: Arc::new(system_clock),
        }
    }

    /// Même client avec une autre horloge (tests).
    pub fn with_clock(mut self, clock: Clock) -> ArtClient {
        self.clock = clock;
        self
    }

    /// Dossier du cache de ce hub.
    pub fn cache_dir(&self) -> &Path {
        &self.dir
    }

    /// Jaquette ou image large du jeu, avec son attribution. `Ok(None)` : le hub ne
    /// connaît pas le jeu, ou le jeu n'a pas cette image.
    pub fn fetch(&self, reference: ArtRef, kind: ArtKind) -> Result<Option<ArtImage>, ArtError> {
        let Some(art) = self.game_art(reference)? else {
            return Ok(None);
        };
        let Some(image_id) = art.image_id(kind) else {
            return Ok(None);
        };
        let Some(bytes) = self.image(image_id, kind)? else {
            return Ok(None);
        };
        Ok(Some(ArtImage {
            bytes,
            attribution: art.attribution,
            attribution_url: art.attribution_url,
        }))
    }

    /// Fiche du jeu : cache, sinon hub, sinon cache périmé.
    pub fn game_art(&self, reference: ArtRef) -> Result<Option<GameArt>, ArtError> {
        let path = self.dir.join("games").join(reference.cache_name());
        let cached = self.read_entry(&path);
        let stale = match cached {
            Cached::Fresh(entry) => return Ok(entry.art),
            Cached::Stale(entry) => Some(entry),
            Cached::Missing => None,
        };
        match self.get_json(&reference.route()) {
            Ok(art) => {
                let art = art.map(GameArt::sanitized);
                self.write_entry(&path, &Entry { art: art.clone() });
                Ok(art)
            }
            // Hub injoignable ou qui répond mal : l'ancienne fiche fait l'affaire.
            Err(error) => match stale {
                Some(entry) => {
                    tracing::debug!(%error, "illustrations : fiche périmée servie");
                    Ok(entry.art)
                }
                None => Err(error),
            },
        }
    }

    /// Octets JPEG d'une image : cache, sinon hub, sinon cache périmé. `Ok(None)` :
    /// le hub n'a pas cette image.
    pub fn image(&self, image_id: &str, kind: ArtKind) -> Result<Option<Vec<u8>>, ArtError> {
        if !valid_image_id(image_id) {
            return Ok(None);
        }
        let path = self
            .dir
            .join("images")
            .join(format!("{image_id}-{}.jpg", kind.as_str()));
        let stale = match self.read_image(&path) {
            Cached::Fresh(bytes) => return Ok(Some(bytes)),
            Cached::Stale(bytes) => Some(bytes),
            Cached::Missing => None,
        };
        let route = format!("/v1/art/images/{image_id}/{}", kind.as_str());
        match self.get_image(&route) {
            Ok(Some(bytes)) => {
                if let Err(error) = write_dated(&path, &bytes, (self.clock)()) {
                    tracing::warn!(%error, "illustrations : image non rangée dans le cache");
                }
                Ok(Some(bytes))
            }
            Ok(None) => Ok(None),
            Err(error) => match stale {
                Some(bytes) => Ok(Some(bytes)),
                None => Err(error),
            },
        }
    }

    /// `GET` d'une fiche. `Ok(None)` sur 404 `art_not_found`.
    fn get_json(&self, route: &str) -> Result<Option<GameArt>, ArtError> {
        let mut response = self
            .agent
            .get(&self.url.join(route))
            .call()
            .map_err(|e| ArtError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_JSON_BYTES)
            .read_to_string()
            .unwrap_or_default();
        match status {
            200 => serde_json::from_str::<GameArt>(&text)
                .map(Some)
                .map_err(|e| ArtError::Unexpected(format!("fiche illisible : {e}"))),
            _ => match api_error(status, &text) {
                // Jeu inconnu : ce n'est pas une erreur, le jeu garde son aplat.
                ApiError::NotFound => Ok(None),
                ApiError::Disabled => Err(ArtError::Disabled),
                ApiError::Other(message) => Err(ArtError::Unexpected(message)),
            },
        }
    }

    /// `GET` d'une image. `Ok(None)` sur 404.
    fn get_image(&self, route: &str) -> Result<Option<Vec<u8>>, ArtError> {
        let mut response = self
            .agent
            .get(&self.url.join(route))
            .call()
            .map_err(|e| ArtError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        if status != 200 {
            let text = response
                .body_mut()
                .with_config()
                .limit(MAX_JSON_BYTES)
                .read_to_string()
                .unwrap_or_default();
            return match api_error(status, &text) {
                ApiError::NotFound => Ok(None),
                ApiError::Disabled => Err(ArtError::Disabled),
                ApiError::Other(message) => Err(ArtError::Unexpected(message)),
            };
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_IMAGE_BYTES)
            .read_to_vec()
            .map_err(|e| ArtError::Unexpected(format!("image : {e}")))?;
        if !looks_like_jpeg(&bytes) {
            return Err(ArtError::Unexpected(
                "ce n'est pas une image JPEG".to_string(),
            ));
        }
        Ok(Some(bytes))
    }

    fn read_entry(&self, path: &Path) -> Cached<Entry> {
        let Some(entry) = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Entry>(&text).ok())
        else {
            return Cached::Missing;
        };
        let lifetime = if entry.art.is_some() {
            FRESH_SECS
        } else {
            MISSING_SECS
        };
        if self.is_fresh(path, lifetime) {
            Cached::Fresh(entry)
        } else {
            Cached::Stale(entry)
        }
    }

    fn write_entry(&self, path: &Path, entry: &Entry) {
        let result = serde_json::to_vec(entry)
            .map_err(|e| ArtError::Io(e.to_string()))
            .and_then(|bytes| write_dated(path, &bytes, (self.clock)()));
        if let Err(error) = result {
            tracing::warn!(%error, "illustrations : fiche non rangée dans le cache");
        }
    }

    fn read_image(&self, path: &Path) -> Cached<Vec<u8>> {
        match fs::read(path) {
            Ok(bytes) if looks_like_jpeg(&bytes) => {
                if self.is_fresh(path, FRESH_SECS) {
                    Cached::Fresh(bytes)
                } else {
                    Cached::Stale(bytes)
                }
            }
            _ => Cached::Missing,
        }
    }

    /// Vrai si le fichier a été rangé il y a moins de `lifetime` secondes : sa date
    /// de modification est celle de l'horloge au moment de l'écriture.
    fn is_fresh(&self, path: &Path, lifetime: i64) -> bool {
        let Some(written) = modified_secs(path) else {
            return false;
        };
        (self.clock)() - written < lifetime
    }
}

/// Erreur JSON du hub (`{ "error": "<code>", "message": "…" }`).
#[derive(Deserialize)]
struct ErrorJson {
    error: String,
    #[serde(default)]
    message: String,
}

enum ApiError {
    NotFound,
    Disabled,
    Other(String),
}

fn api_error(status: u16, text: &str) -> ApiError {
    match serde_json::from_str::<ErrorJson>(text) {
        Ok(error) if error.error == "art_not_found" => ApiError::NotFound,
        Ok(error) if error.error == "art_disabled" => ApiError::Disabled,
        Ok(error) => ApiError::Other(format!("{status} {} {}", error.error, error.message)),
        // Un hub trop ancien ne connaît pas la route : 404 sans corps JSON.
        Err(_) if status == 404 => ApiError::Disabled,
        Err(_) => ApiError::Other(format!("code {status}")),
    }
}

/// Nom de dossier tiré de l'adresse du hub : hôte, port et chemin, chaque caractère
/// autre qu'une lettre, un chiffre, `.` ou `-` devient `_`.
fn folder_name(url: &HubUrl) -> String {
    let text = url
        .as_str()
        .strip_prefix("https://")
        .or_else(|| url.as_str().strip_prefix("http://"))
        .unwrap_or(url.as_str());
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Date de modification d'un fichier, en secondes Unix.
fn modified_secs(path: &Path) -> Option<i64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let secs = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    i64::try_from(secs).ok()
}

/// Écrit `bytes` par un fichier temporaire renommé (jamais de fichier à moitié
/// écrit), daté de `at` (secondes Unix).
fn write_dated(path: &Path, bytes: &[u8], at: i64) -> Result<(), ArtError> {
    let io = |e: std::io::Error| ArtError::Io(e.to_string());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io)?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    {
        // Bloc : le fichier est fermé à la sortie, avant le renommage.
        let mut file = fs::File::create(&temp).map_err(io)?;
        file.write_all(bytes).map_err(io)?;
        let date = UNIX_EPOCH + Duration::from_secs(u64::try_from(at).unwrap_or(0));
        file.set_modified(date).map_err(io)?;
    }
    fs::rename(&temp, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nom_de_dossier_du_hub() {
        let url = HubUrl::parse("http://127.0.0.1:8787").unwrap();
        assert_eq!(folder_name(&url), "127.0.0.1_8787");
        let url = HubUrl::parse("https://Hub.Exemple.org/rafthub").unwrap();
        assert_eq!(folder_name(&url), "hub.exemple.org_rafthub");
    }

    #[test]
    fn fiche_nettoyee() {
        let art = GameArt {
            igdb_id: 1,
            cover: Some("../mal".to_string()),
            hero: Some("ar1".to_string()),
            attribution: "IGDB.com".to_string(),
            attribution_url: "javascript:alert(1)".to_string(),
        }
        .sanitized();
        assert_eq!(art.cover, None);
        assert_eq!(art.hero.as_deref(), Some("ar1"));
        assert_eq!(art.attribution_url, "");
    }

    #[test]
    fn ecriture_datee_relue() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b.json");
        assert!(write_dated(&path, br#"{"art":null}"#, 1_800_000_000).is_ok());
        assert_eq!(modified_secs(&path), Some(1_800_000_000));
        let entry: Entry = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(entry.art.is_none());
    }
}
