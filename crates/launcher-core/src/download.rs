//! Téléchargement HTTPS, cache rangé par sha256 et taille maximale.
//!
//! Principe : un fichier téléchargé n'est jamais utilisé avant que son hash soit vérifié.
//! - il est écrit dans `<dir>/<sha256>.part`, puis renommé en `<dir>/<sha256>` seulement si
//!   le hash correspond ;
//! - le cache est relu et re-haché à chaque usage : un fichier corrompu est supprimé puis
//!   retéléchargé ;
//! - la taille est comptée pendant la lecture (on ne fait pas confiance à `Content-Length`) ;
//! - seul HTTPS est accepté, redirections comprises.
//!
//! Le code est synchrone : l'interface l'appelle depuis un thread à part (ADR 0015).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use py_bridge::CancelToken;

use crate::hash::Sha256;

/// Ce qui fait échouer un téléchargement.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// Écriture ou lecture impossible dans le cache.
    #[error("accès à {path} : {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// L'adresse n'est pas en HTTPS (ou une redirection y mène).
    #[error("adresse refusée, HTTPS obligatoire : {0}")]
    NotHttps(String),
    /// Le serveur a répondu par un code d'erreur.
    #[error("le serveur a répondu {status} pour {url}")]
    Http { url: String, status: u16 },
    /// Échec réseau (DNS, connexion, TLS, délai dépassé, coupure…).
    #[error("échec réseau pour {url} : {reason}")]
    Network { url: String, reason: String },
    /// Le fichier dépasse la taille maximale autorisée.
    #[error("{url} dépasse la taille maximale de {max} octets")]
    TooLarge { url: String, max: u64 },
    /// Le contenu reçu ne correspond pas au hash attendu.
    #[error("hash incorrect pour {url} : attendu {expected}, obtenu {actual}")]
    HashMismatch {
        url: String,
        expected: Sha256,
        actual: Sha256,
    },
    /// Le joueur a annulé pendant le téléchargement ; le `.part` est supprimé.
    #[error("téléchargement annulé")]
    Cancelled,
}

/// Ouvre une adresse et rend un flux d'octets à lire.
///
/// `Send + Sync` : le téléchargeur peut être partagé avec le thread de l'interface.
/// Un trait permet de remplacer le réseau par des fichiers locaux dans les tests.
pub trait Fetcher: Send + Sync {
    fn open(&self, url: &str) -> Result<Box<dyn Read + Send>, DownloadError>;
}

/// Vrai client HTTP (`ureq`), HTTPS seulement.
pub struct HttpFetcher {
    agent: ureq::Agent,
}

impl HttpFetcher {
    pub fn new() -> HttpFetcher {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            // Refuse tout ce qui n'est pas HTTPS, y compris les redirections.
            .https_only(true)
            // Redirections bornées : trop de sauts => erreur (GitHub en fait quelques-unes).
            .max_redirects(5)
            .max_redirects_will_error(true)
            // On veut les codes 4xx/5xx comme erreurs `StatusCode`.
            .http_status_as_error(true)
            .timeout_resolve(Some(Duration::from_secs(10)))
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            // Durée totale maximale de lecture du corps (le budget n'est pas relancé à
            // chaque lecture) : 30 minutes pour un gros mod sur une connexion lente.
            .timeout_recv_body(Some(Duration::from_secs(30 * 60)))
            .build()
            .into();
        HttpFetcher { agent }
    }
}

impl Default for HttpFetcher {
    fn default() -> Self {
        HttpFetcher::new()
    }
}

impl Fetcher for HttpFetcher {
    fn open(&self, url: &str) -> Result<Box<dyn Read + Send>, DownloadError> {
        if !is_https(url) {
            return Err(DownloadError::NotHttps(url.to_string()));
        }
        let response = self
            .agent
            .get(url)
            .call()
            .map_err(|e| map_ureq_error(url, e))?;
        // Le corps est rendu tel quel : la taille maximale est comptée par `Downloader`,
        // qui arrête de lire dès qu'elle est dépassée.
        let (_, body) = response.into_parts();
        Ok(Box::new(body.into_with_config().limit(u64::MAX).reader()))
    }
}

/// Traduit une erreur `ureq` en erreur du launcher.
fn map_ureq_error(url: &str, error: ureq::Error) -> DownloadError {
    match error {
        ureq::Error::StatusCode(status) => DownloadError::Http {
            url: url.to_string(),
            status,
        },
        ureq::Error::RequireHttpsOnly(_) => DownloadError::NotHttps(url.to_string()),
        other => DownloadError::Network {
            url: url.to_string(),
            reason: other.to_string(),
        },
    }
}

/// Faux téléchargeur pour les tests : une URL correspond à un fichier local. Aucun réseau.
pub struct DirFetcher {
    files: BTreeMap<String, PathBuf>,
    opens: Arc<AtomicUsize>,
}

impl DirFetcher {
    pub fn new() -> DirFetcher {
        DirFetcher {
            files: BTreeMap::new(),
            opens: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Associe une URL à un fichier local.
    pub fn with_file(mut self, url: &str, path: impl Into<PathBuf>) -> DirFetcher {
        self.files.insert(url.to_string(), path.into());
        self
    }

    /// Compteur partagé du nombre d'ouvertures (pour vérifier qu'on ne retélécharge pas).
    /// `Arc` : le compteur reste lisible par le test même après avoir donné le fetcher
    /// au `Downloader`.
    pub fn open_counter(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.opens)
    }
}

impl Default for DirFetcher {
    fn default() -> Self {
        DirFetcher::new()
    }
}

impl Fetcher for DirFetcher {
    fn open(&self, url: &str) -> Result<Box<dyn Read + Send>, DownloadError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        // Même règle que le vrai client : HTTPS seulement.
        if !is_https(url) {
            return Err(DownloadError::NotHttps(url.to_string()));
        }
        let path = self.files.get(url).ok_or_else(|| DownloadError::Http {
            url: url.to_string(),
            status: 404,
        })?;
        let file = File::open(path).map_err(|source| DownloadError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(Box::new(file))
    }
}

/// Télécharge dans un cache rangé par sha256.
pub struct Downloader {
    /// Dossier du cache : un fichier `<sha256>` par téléchargement vérifié.
    pub dir: PathBuf,
    /// `Box<dyn Fetcher>` : on choisit le vrai réseau ou le faux à l'exécution.
    pub fetcher: Box<dyn Fetcher>,
}

impl Downloader {
    pub fn new(dir: PathBuf, fetcher: Box<dyn Fetcher>) -> Downloader {
        Downloader { dir, fetcher }
    }

    /// Rend le chemin d'un fichier vérifié (`<dir>/<sha256>`), en le téléchargeant au besoin.
    ///
    /// `progress` reçoit le nombre d'octets reçus jusqu'ici.
    /// Un mauvais hash ne laisse rien derrière lui (ni `.part`, ni fichier final).
    pub fn get(
        &self,
        url: &str,
        sha256: &Sha256,
        max_bytes: u64,
        progress: &mut dyn FnMut(u64),
    ) -> Result<PathBuf, DownloadError> {
        // Un jeton neuf n'est jamais annulé : `get` se comporte comme avant.
        self.get_cancellable(url, sha256, max_bytes, progress, &CancelToken::new())
    }

    /// Comme [`Downloader::get`], mais s'arrête avec `DownloadError::Cancelled` dès que
    /// `cancel` est annulé (vérifié entre deux paquets de 64 Kio, donc très vite).
    pub fn get_cancellable(
        &self,
        url: &str,
        sha256: &Sha256,
        max_bytes: u64,
        progress: &mut dyn FnMut(u64),
        cancel: &CancelToken,
    ) -> Result<PathBuf, DownloadError> {
        if cancel.is_cancelled() {
            return Err(DownloadError::Cancelled);
        }
        // Contrôle fait avant tout accès au disque ou au réseau.
        if !is_https(url) {
            return Err(DownloadError::NotHttps(url.to_string()));
        }
        fs::create_dir_all(&self.dir).map_err(|source| DownloadError::Io {
            path: self.dir.clone(),
            source,
        })?;

        let final_path = self.dir.join(sha256.to_string());
        let part_path = self.dir.join(format!("{sha256}.part"));

        // 1. Le cache : on re-hache à chaque usage, car le fichier a pu être abîmé.
        if final_path.exists() {
            let actual = hash_file(&final_path)?;
            if actual == *sha256 {
                let size = fs::metadata(&final_path)
                    .map_err(|source| io_error(&final_path, source))?
                    .len();
                progress(size);
                return Ok(final_path);
            }
            tracing::warn!("cache corrompu, suppression de {}", final_path.display());
            fs::remove_file(&final_path).map_err(|source| io_error(&final_path, source))?;
        }

        // 2. Téléchargement. Quoi qu'il arrive, le `.part` est supprimé en cas d'échec.
        let result = self.download_to_part(url, sha256, max_bytes, &part_path, progress, cancel);
        if let Err(error) = result {
            // Le résultat de la suppression est ignoré : l'erreur d'origine est plus utile.
            let _ = fs::remove_file(&part_path);
            return Err(error);
        }

        // 3. Hash bon : le fichier prend sa place définitive.
        fs::rename(&part_path, &final_path).map_err(|source| {
            let _ = fs::remove_file(&part_path);
            io_error(&final_path, source)
        })?;
        Ok(final_path)
    }

    /// Écrit le flux dans `part_path` puis vérifie taille et hash.
    fn download_to_part(
        &self,
        url: &str,
        expected: &Sha256,
        max_bytes: u64,
        part_path: &Path,
        progress: &mut dyn FnMut(u64),
        cancel: &CancelToken,
    ) -> Result<(), DownloadError> {
        let mut reader = self.fetcher.open(url)?;
        let mut file = File::create(part_path).map_err(|source| io_error(part_path, source))?;

        let mut buffer = [0u8; 64 * 1024];
        let mut total: u64 = 0;
        loop {
            if cancel.is_cancelled() {
                return Err(DownloadError::Cancelled);
            }
            let read = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    return Err(DownloadError::Network {
                        url: url.to_string(),
                        reason: e.to_string(),
                    })
                }
            };
            // `usize` -> `u64` : sans perte sur les plateformes visées.
            total += read as u64;
            // La taille est comptée ici, pendant la lecture : on s'arrête dès le dépassement.
            if total > max_bytes {
                return Err(DownloadError::TooLarge {
                    url: url.to_string(),
                    max: max_bytes,
                });
            }
            file.write_all(&buffer[..read])
                .map_err(|source| io_error(part_path, source))?;
            progress(total);
        }
        file.flush().map_err(|source| io_error(part_path, source))?;
        // On ferme le fichier avant de le relire et de le renommer (obligatoire sous Windows).
        drop(file);

        let actual = hash_file(part_path)?;
        if actual != *expected {
            return Err(DownloadError::HashMismatch {
                url: url.to_string(),
                expected: *expected,
                actual,
            });
        }
        Ok(())
    }
}

fn io_error(path: &Path, source: io::Error) -> DownloadError {
    DownloadError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn hash_file(path: &Path) -> Result<Sha256, DownloadError> {
    let mut file = File::open(path).map_err(|source| io_error(path, source))?;
    Sha256::of_reader(&mut file).map_err(|source| io_error(path, source))
}

/// Vrai si l'adresse commence par `https://` (casse ignorée).
fn is_https(url: &str) -> bool {
    url.get(..8)
        .is_some_and(|start| start.eq_ignore_ascii_case("https://"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnait_https() {
        assert!(is_https("https://a.b/c"));
        assert!(is_https("HTTPS://a.b/c"));
        assert!(!is_https("http://a.b/c"));
        assert!(!is_https("ftp://a.b"));
        assert!(!is_https("é"));
    }
}
