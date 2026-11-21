//! Distribution du launcher par le hub (ADR 0036, « Launcher relié au hub »).
//!
//! Le hub sert un zip qui contient l'exe **officiel** du launcher et un fichier
//! `rafthub.toml` qui pointe vers ce hub. L'exe n'est jamais modifié : il garde la
//! signature de l'ADR 0022.
//!
//! Déroulement d'un « rafraîchir » ([`Distribution::refresh`]) :
//! 1. lire le manifeste `update.json` de la dernière release GitHub (adresse de la
//!    liste blanche, 64 Kio au plus) ;
//! 2. télécharger l'exe qu'il annonce (liste blanche, taille bornée) ;
//! 3. contrôler son sha256, puis sa **signature minisign** avec la clé publique des
//!    mises à jour ;
//! 4. le ranger dans le dossier de cache, avec sa signature et un petit `cache.json`.
//!
//! Avant chaque service, la signature de l'exe du cache est **revérifiée** : un
//! fichier changé sur le disque n'est jamais servi.
//!
//! Le réseau passe par le trait [`ReleaseSource`] : les tests y mettent un faux qui
//! sert des fichiers en mémoire, signés avec une clé générée pour le test.

use std::fmt;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Clé publique minisign des mises à jour du launcher.
///
/// **Copie** de `launcher_core::update::UPDATE_PUBLIC_KEY` (ADR 0022) : le hub ne
/// dépend pas de `launcher-core`, qui tirerait le serveur, le générateur et le
/// trousseau. Un test (`tests/distribution.rs`) vérifie que les deux restent égales.
pub const UPDATE_PUBLIC_KEY: &str = "RWS+YnudIFIVAFDtalsvodpSlxBLmoOCG7qs5tB7VUHc6n+H3dot3FjI";

/// Adresse du manifeste : copie de `launcher_core::update::UPDATE_MANIFEST_URL`.
pub const UPDATE_MANIFEST_URL: &str =
    "https://github.com/Yonako25/raftup/releases/latest/download/update.json";

/// Hôtes d'où le hub accepte de télécharger (comme `launcher_core::profile::ALLOWED_HOSTS`
/// pour GitHub). Les redirections de GitHub restent en HTTPS ; la signature fait le reste.
pub const ALLOWED_HOSTS: &[&str] = &["github.com", "objects.githubusercontent.com"];

/// Taille maximale du manifeste (c'est un petit JSON).
pub const MANIFEST_MAX_BYTES: u64 = 64 * 1024;

/// Plafond de la taille de l'exe, quoi que dise le manifeste (comme l'ADR 0022).
pub const EXE_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Nom de l'exe dans le zip (nom du binaire du launcher).
pub const EXE_NAME: &str = "raftup.exe";

/// Nom du fichier qui relie le launcher au hub (`launcher_core::rafthub::BUNDLED_FILE`).
pub const BUNDLED_FILE: &str = "rafthub.toml";

/// Nom du zip servi.
pub const ZIP_NAME: &str = "raftup-windows.zip";

const CACHE_INFO: &str = "cache.json";
const CACHE_SIGNATURE: &str = "raftup.exe.minisig";

/// Ce qui empêche de rafraîchir le cache ou de servir le zip.
#[derive(Debug, Error)]
pub enum DistributionError {
    /// Adresse hors liste blanche, pas en HTTPS, avec identifiants ou port.
    #[error("adresse refusée : {0}")]
    BadUrl(String),
    /// Le réseau ou GitHub n'a pas répondu comme prévu.
    #[error("téléchargement de {url} impossible : {reason}")]
    Network { url: String, reason: String },
    /// Le fichier reçu dépasse la taille permise.
    #[error("fichier trop gros (plus de {0} octets)")]
    TooLarge(u64),
    /// Le manifeste n'est pas du bon format.
    #[error("manifeste illisible : {0}")]
    Manifest(String),
    /// Le manifeste propose une version plus ancienne que celle du cache.
    #[error("le manifeste propose la version {offered}, plus ancienne que {cached}")]
    Downgrade { offered: String, cached: String },
    /// Le sha256 de l'exe n'est pas celui du manifeste.
    #[error("empreinte sha256 inattendue")]
    HashMismatch,
    /// La clé publique compilée n'est pas une clé minisign.
    #[error("clé publique minisign invalide")]
    BadPublicKey,
    /// La signature est absente, illisible ou ne correspond pas à l'exe.
    #[error("signature minisign incorrecte : exe refusé")]
    BadSignature,
    /// Aucun exe en cache : il faut d'abord rafraîchir.
    #[error("aucun launcher en cache")]
    NotCached,
    /// Un rafraîchissement est déjà en cours.
    #[error("un rafraîchissement est déjà en cours")]
    Busy,
    /// Lecture ou écriture du cache impossible.
    #[error("accès à {path} : {reason}")]
    Io { path: PathBuf, reason: String },
    /// Fabrication du zip impossible.
    #[error("fabrication du zip : {0}")]
    Zip(String),
}

/// D'où viennent le manifeste et l'exe. Le vrai passe par HTTPS ; les tests en
/// donnent un faux, sans réseau.
///
/// `Send + Sync` : la source est partagée entre les threads qui servent les requêtes.
pub trait ReleaseSource: Send + Sync {
    /// Octets à `url`, `max_bytes` au plus (au-delà : [`DistributionError::TooLarge`]).
    fn fetch(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, DistributionError>;
}

/// Vraie source : client HTTPS `ureq`, déjà utilisé par le hub pour Discord.
pub struct HttpRelease {
    agent: ureq::Agent,
}

impl HttpRelease {
    pub fn new() -> HttpRelease {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            // Refuse tout ce qui n'est pas HTTPS, y compris les redirections.
            .https_only(true)
            // GitHub redirige vers son stockage : quelques sauts, pas plus.
            .max_redirects(5)
            .max_redirects_will_error(true)
            .http_status_as_error(true)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(600)))
            .build()
            .into();
        HttpRelease { agent }
    }
}

impl Default for HttpRelease {
    fn default() -> Self {
        HttpRelease::new()
    }
}

impl ReleaseSource for HttpRelease {
    fn fetch(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, DistributionError> {
        let network = |reason: String| DistributionError::Network {
            url: url.to_string(),
            reason,
        };
        let response = self
            .agent
            .get(url)
            .call()
            .map_err(|e| network(e.to_string()))?;
        let mut bytes = Vec::new();
        // `take` borne la lecture : un flux sans fin ne remplit pas la mémoire.
        response
            .into_body()
            .into_reader()
            .take(max_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| network(e.to_string()))?;
        if bytes.len() as u64 > max_bytes {
            return Err(DistributionError::TooLarge(max_bytes));
        }
        Ok(bytes)
    }
}

/// Réglages de la distribution. `Default` : vraie source, vraie clé, cache dans
/// `rafthub-cache/` à côté de la base par défaut.
#[derive(Clone)]
pub struct DistributionConfig {
    /// Dossier du cache (créé au premier rafraîchissement).
    pub cache_dir: PathBuf,
    /// Adresse du manifeste (doit être dans la liste blanche).
    pub manifest_url: String,
    /// Clé publique minisign (ligne base64 « RW… »).
    pub public_key: String,
    /// `Arc<dyn …>` : la source est partagée, et son vrai type choisi au lancement.
    pub source: Arc<dyn ReleaseSource>,
}

impl Default for DistributionConfig {
    fn default() -> Self {
        DistributionConfig {
            cache_dir: PathBuf::from("rafthub-cache"),
            manifest_url: UPDATE_MANIFEST_URL.to_string(),
            public_key: UPDATE_PUBLIC_KEY.to_string(),
            source: Arc::new(HttpRelease::new()),
        }
    }
}

// `Debug` écrit à la main : un `dyn ReleaseSource` n'en a pas.
impl fmt::Debug for DistributionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DistributionConfig")
            .field("cache_dir", &self.cache_dir)
            .field("manifest_url", &self.manifest_url)
            .finish_non_exhaustive()
    }
}

/// Ce que `cache.json` retient de l'exe en cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedLauncher {
    /// Version annoncée par le manifeste (semver).
    pub version: String,
    /// sha256 de l'exe, en hexadécimal.
    pub sha256: String,
    /// Taille en octets.
    pub size: u64,
    /// Date du téléchargement (secondes Unix).
    pub fetched_at: i64,
}

/// État du cache pour la page d'admin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheStatus {
    pub launcher: CachedLauncher,
    /// Vrai si l'exe du disque porte encore une signature valide.
    pub signature_ok: bool,
}

/// Manifeste de l'ADR 0022 (les champs inconnus, comme `notes`, sont ignorés).
#[derive(Deserialize)]
struct Manifest {
    format_version: u32,
    version: String,
    url: String,
    sha256: String,
    max_bytes: u64,
    signature: String,
}

/// Le cache de l'exe officiel et le zip déjà fabriqué.
pub struct Distribution {
    config: DistributionConfig,
    /// Un seul rafraîchissement à la fois.
    refresh_lock: Mutex<()>,
    /// Dernier zip fabriqué, avec ce qui l'identifie (sha256 de l'exe + adresse du hub) :
    /// on ne le refait pas à chaque téléchargement.
    zip_memo: Mutex<Option<(String, Arc<Vec<u8>>)>>,
}

impl Distribution {
    pub fn new(config: DistributionConfig) -> Distribution {
        Distribution {
            config,
            refresh_lock: Mutex::new(()),
            zip_memo: Mutex::new(None),
        }
    }

    /// État du cache, `None` s'il est vide. Relit l'exe pour revérifier sa signature.
    /// Bloquant (lecture disque) : à appeler depuis `spawn_blocking`.
    pub fn status(&self) -> Result<Option<CacheStatus>, DistributionError> {
        let Some(launcher) = self.read_info()? else {
            return Ok(None);
        };
        let signature_ok = self.load_verified(&launcher).is_ok();
        Ok(Some(CacheStatus {
            launcher,
            signature_ok,
        }))
    }

    /// Télécharge le dernier exe signé et remplace le cache. Bloquant (réseau).
    pub fn refresh(&self, now: i64) -> Result<CachedLauncher, DistributionError> {
        // `try_lock` : si un autre rafraîchissement tient le verrou, on refuse au lieu
        // d'attendre (et de lancer deux téléchargements de suite).
        let _guard = match self.refresh_lock.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return Err(DistributionError::Busy),
            // Verrou « empoisonné » (un thread a paniqué en le tenant) : il ne protège
            // aucune donnée, on peut le reprendre.
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        let key = self.public_key()?;

        // 1. Le manifeste.
        check_url(&self.config.manifest_url)?;
        let bytes = self
            .config
            .source
            .fetch(&self.config.manifest_url, MANIFEST_MAX_BYTES)?;
        let manifest = parse_manifest(&bytes)?;
        if let Some(cached) = self.read_info()? {
            refuse_downgrade(&manifest.version, &cached.version)?;
        }
        let signature =
            Signature::decode(&manifest.signature).map_err(|_| DistributionError::BadSignature)?;

        // 2. L'exe, puis 3. son sha256 et sa signature.
        let exe = self
            .config
            .source
            .fetch(&manifest.url, manifest.max_bytes)?;
        if sha256_hex(&exe) != manifest.sha256 {
            return Err(DistributionError::HashMismatch);
        }
        // `false` : les signatures « legacy » (non pré-hachées) sont refusées.
        key.verify(&exe, &signature, false)
            .map_err(|_| DistributionError::BadSignature)?;

        // 4. Rangement : `cache.json` en dernier, il « valide » les deux autres.
        let launcher = CachedLauncher {
            version: manifest.version,
            sha256: manifest.sha256,
            size: exe.len() as u64,
            fetched_at: now,
        };
        let dir = &self.config.cache_dir;
        fs::create_dir_all(dir).map_err(io_error(dir))?;
        write_atomic(&dir.join(EXE_NAME), &exe)?;
        write_atomic(&dir.join(CACHE_SIGNATURE), manifest.signature.as_bytes())?;
        let info = serde_json::to_vec_pretty(&launcher)
            .map_err(|e| DistributionError::Manifest(e.to_string()))?;
        write_atomic(&dir.join(CACHE_INFO), &info)?;
        tracing::info!(version = %launcher.version, "launcher officiel mis en cache");
        Ok(launcher)
    }

    /// Zip `raftup-windows.zip` relié au hub `hub_url`. L'exe du cache est revérifié
    /// avant d'entrer dans le zip. Bloquant : à appeler depuis `spawn_blocking`.
    pub fn zip_for(&self, hub_url: &str) -> Result<Arc<Vec<u8>>, DistributionError> {
        let launcher = self.read_info()?.ok_or(DistributionError::NotCached)?;
        let memo_key = format!("{}|{hub_url}", launcher.sha256);
        if let Some((key, zip)) = self.memo().as_ref() {
            // Le zip mémorisé a été fait à partir d'un exe vérifié de même sha256 ; on
            // vérifie quand même que le fichier du disque n'a pas changé depuis.
            if *key == memo_key && self.load_verified(&launcher).is_ok() {
                return Ok(Arc::clone(zip));
            }
        }
        let exe = self.load_verified(&launcher)?;
        let zip = Arc::new(build_zip(&exe, hub_url)?);
        *self.memo() = Some((memo_key, Arc::clone(&zip)));
        Ok(zip)
    }

    /// Le verrou du zip mémorisé (repris même « empoisonné » : ce n'est qu'un cache).
    fn memo(&self) -> std::sync::MutexGuard<'_, Option<(String, Arc<Vec<u8>>)>> {
        self.zip_memo
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn public_key(&self) -> Result<PublicKey, DistributionError> {
        PublicKey::from_base64(self.config.public_key.trim())
            .map_err(|_| DistributionError::BadPublicKey)
    }

    /// Lit `cache.json` ; `None` si le cache est vide.
    fn read_info(&self) -> Result<Option<CachedLauncher>, DistributionError> {
        let path = self.config.cache_dir.join(CACHE_INFO);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(&path)(error)),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| DistributionError::Io {
                path,
                reason: e.to_string(),
            })
    }

    /// Relit l'exe du cache et rend ses octets si sha256 **et** signature sont bons.
    fn load_verified(&self, launcher: &CachedLauncher) -> Result<Vec<u8>, DistributionError> {
        let key = self.public_key()?;
        let dir = &self.config.cache_dir;
        let exe_path = dir.join(EXE_NAME);
        let exe = fs::read(&exe_path).map_err(io_error(&exe_path))?;
        if sha256_hex(&exe) != launcher.sha256 {
            return Err(DistributionError::HashMismatch);
        }
        let sig_path = dir.join(CACHE_SIGNATURE);
        let text = fs::read_to_string(&sig_path).map_err(io_error(&sig_path))?;
        let signature = Signature::decode(&text).map_err(|_| DistributionError::BadSignature)?;
        key.verify(&exe, &signature, false)
            .map_err(|_| DistributionError::BadSignature)?;
        Ok(exe)
    }
}

/// Lit et contrôle le manifeste (format, version, adresse, taille, empreinte).
fn parse_manifest(bytes: &[u8]) -> Result<Manifest, DistributionError> {
    let bad = |why: String| DistributionError::Manifest(why);
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|e| bad(e.to_string()))?;
    if manifest.format_version != 1 {
        return Err(bad(format!(
            "format {} inconnu (attendu : 1)",
            manifest.format_version
        )));
    }
    Version::parse(&manifest.version)
        .map_err(|e| bad(format!("version « {} » : {e}", manifest.version)))?;
    check_url(&manifest.url)?;
    if manifest.max_bytes == 0 || manifest.max_bytes > EXE_MAX_BYTES {
        return Err(bad(format!(
            "taille maximale {} refusée",
            manifest.max_bytes
        )));
    }
    let hash_ok = manifest.sha256.len() == 64
        && manifest
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !hash_ok {
        return Err(bad("sha256 : 64 caractères hexadécimaux minuscules".into()));
    }
    Ok(manifest)
}

/// Refuse une version plus ancienne que celle du cache (on ne revient pas en arrière,
/// même avec un vieux manifeste bien signé).
fn refuse_downgrade(offered: &str, cached: &str) -> Result<(), DistributionError> {
    let (Ok(new), Ok(old)) = (Version::parse(offered), Version::parse(cached)) else {
        // Cache illisible : on le remplace.
        return Ok(());
    };
    if new < old {
        return Err(DistributionError::Downgrade {
            offered: offered.to_string(),
            cached: cached.to_string(),
        });
    }
    Ok(())
}

/// HTTPS, hôte de la liste blanche, ni identifiants ni port (comme l'ADR 0022).
pub fn check_url(url: &str) -> Result<(), DistributionError> {
    let bad = |why: &str| DistributionError::BadUrl(format!("« {url} » : {why}"));
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(bad("HTTPS obligatoire"));
    };
    if url.contains([' ', '\0', '\n', '\r', '\\']) {
        return Err(bad("caractère interdit"));
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.contains(['@', ':']) {
        return Err(bad("hôte invalide"));
    }
    if !ALLOWED_HOSTS.contains(&host.to_ascii_lowercase().as_str()) {
        return Err(bad("domaine hors de la liste blanche"));
    }
    Ok(())
}

/// Contenu de `rafthub.toml`. L'adresse vient du réglage `public_url`, déjà validé
/// (pas de guillemet ni de contrôle) ; on échappe quand même `\` et `"`.
pub fn bundled_toml(hub_url: &str) -> String {
    let escaped = hub_url.replace('\\', "\\\\").replace('"', "\\\"");
    format!("# Relie ce launcher à un Rafthub (ADR 0036).\nformat_version = 1\nhub_url = \"{escaped}\"\n")
}

/// Fabrique le zip : l'exe tel quel et `rafthub.toml`.
fn build_zip(exe: &[u8], hub_url: &str) -> Result<Vec<u8>, DistributionError> {
    use zip::write::SimpleFileOptions;
    let zip_error = |e: zip::result::ZipError| DistributionError::Zip(e.to_string());
    let io = |e: std::io::Error| DistributionError::Zip(e.to_string());

    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    writer.start_file(EXE_NAME, options).map_err(zip_error)?;
    writer.write_all(exe).map_err(io)?;
    writer
        .start_file(BUNDLED_FILE, options)
        .map_err(zip_error)?;
    writer
        .write_all(bundled_toml(hub_url).as_bytes())
        .map_err(io)?;
    Ok(writer.finish().map_err(zip_error)?.into_inner())
}

/// sha256 en hexadécimal minuscule.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Écrit dans un fichier temporaire puis le renomme : un lecteur ne voit jamais un
/// fichier à moitié écrit.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), DistributionError> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    fs::write(&temp, bytes).map_err(io_error(&temp))?;
    fs::rename(&temp, path).map_err(io_error(path))
}

/// Fabrique la conversion d'une erreur d'entrée-sortie pour `path`.
///
/// Elle rend une **fermeture** (`impl Fn`) : on l'écrit `.map_err(io_error(&path))`.
fn io_error(path: &Path) -> impl Fn(std::io::Error) -> DistributionError + '_ {
    move |error| DistributionError::Io {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

/// Hôte (avec port éventuel) d'une adresse `http(s)://hôte[:port][/…]`, pour le lien
/// `raftup://hub/<hôte>`.
pub fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then_some(host)
}

/// Vrai si le launcher acceptera cette adresse (règles de `HubUrl::parse`, ADR 0035) :
/// HTTPS, ou HTTP vers ce PC.
pub fn launcher_accepts(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|prefix| {
            url.strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liste_blanche() {
        assert!(check_url(UPDATE_MANIFEST_URL).is_ok());
        assert!(check_url("https://objects.githubusercontent.com/x").is_ok());
        for url in [
            "http://github.com/a",
            "https://github.com@evil.example.com/a",
            "https://github.com:8443/a",
            "https://github.com.evil.example.com/a",
            "https://evil.example.com/a",
        ] {
            assert!(check_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn hote_et_adresses_acceptees() {
        assert_eq!(host_of("https://hub.exemple.org"), Some("hub.exemple.org"));
        assert_eq!(
            host_of("https://hub.exemple.org:8443/"),
            Some("hub.exemple.org:8443")
        );
        assert_eq!(host_of("ftp://x"), None);
        assert!(launcher_accepts("https://hub.exemple.org"));
        assert!(launcher_accepts("http://127.0.0.1:8787"));
        assert!(!launcher_accepts("http://hub.exemple.org"));
        assert!(!launcher_accepts("http://127.0.0.1.exemple.org"));
    }

    #[test]
    fn retour_en_arriere_refuse() {
        assert!(refuse_downgrade("0.2.0", "0.3.0").is_err());
        assert!(refuse_downgrade("0.3.0", "0.3.0").is_ok());
        assert!(refuse_downgrade("0.4.0", "0.3.0").is_ok());
    }

    #[test]
    fn toml_echappe() {
        let text = bundled_toml("https://hub.exemple.org");
        assert!(text.contains("format_version = 1\n"));
        assert!(text.contains("hub_url = \"https://hub.exemple.org\"\n"));
        assert!(bundled_toml("a\"b").contains("hub_url = \"a\\\"b\""));
    }
}
