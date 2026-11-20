//! HTTPS intégré, en option (ADR 0039).
//!
//! Sans réglage TLS, le hub parle HTTP en clair et se place derrière un proxy
//! (ADR 0033) : ce module ne sert pas. Avec un réglage, il sert lui-même en HTTPS :
//!
//! - **fichiers PEM** (`--tls-cert`, `--tls-key`) : certificat et clé fournis, par
//!   exemple `fullchain.pem` et `privkey.pem` de certbot ;
//! - **Let's Encrypt** (`--tls-acme <domaine>`) : certificat obtenu et renouvelé
//!   tout seul par la crate `rustls-acme`, défi TLS-ALPN-01 (le hub doit être joint
//!   sur le port 443), certificats rangés dans `rafthub-data/acme/`.
//!
//! Les deux passent par [`TlsSetup`] puis [`serve`], qui sert un routeur axum en
//! HTTP ou en HTTPS selon le cas.

use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use rustls::pki_types::pem::{self, PemObject};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Semaphore};
use tokio_rustls::server::TlsStream;
use tokio_rustls::LazyConfigAcceptor;

/// Erreur de configuration TLS : chaque cas dit ce qui ne va pas, et dans quel fichier.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// Fichier absent ou illisible (droits, chemin…).
    #[error("impossible de lire {what} {path}")]
    Read {
        what: &'static str,
        path: String,
        source: std::io::Error,
    },
    /// Fichier trop gros pour être un PEM : sûrement pas le bon fichier.
    #[error("{what} {path} fait plus d'1 Mio : ce n'est sûrement pas un fichier PEM")]
    TooLarge { what: &'static str, path: String },
    /// Pas de bloc `BEGIN CERTIFICATE` dans le fichier.
    #[error("aucun certificat PEM dans {path}")]
    NoCertificate { path: String },
    /// Pas de bloc `BEGIN PRIVATE KEY` (ou RSA, EC) dans le fichier.
    #[error("aucune clé privée PEM dans {path}")]
    NoKey { path: String },
    /// PEM mal formé.
    #[error("PEM illisible dans {path} : {message}")]
    BadPem { path: String, message: String },
    /// rustls refuse la paire : clé qui ne va pas avec le certificat, type non géré…
    #[error("certificat ou clé refusés : {0}")]
    Rejected(#[from] rustls::Error),
    /// Dossier des certificats Let's Encrypt impossible à créer.
    #[error("impossible de créer le dossier {path}")]
    CacheDir {
        path: String,
        source: std::io::Error,
    },
}

/// Réglages de Let's Encrypt (`--tls-acme`).
#[derive(Clone, Debug)]
pub struct AcmeOptions {
    /// Nom de domaine du hub, par exemple `hub.exemple.org`.
    pub domain: String,
    /// Adresse e-mail de contact donnée à Let's Encrypt (facultative).
    pub contact_email: Option<String>,
    /// Dossier où ranger le compte et les certificats (`rafthub-data/acme/`).
    pub cache_dir: PathBuf,
    /// Vrai : serveur d'essai de Let's Encrypt (certificats non reconnus par les
    /// navigateurs, mais limites bien plus larges). Pour essayer son installation.
    pub staging: bool,
}

/// Configuration TLS prête à servir.
///
/// `Clone` est bon marché : les configurations rustls sont partagées par `Arc`
/// (un pointeur compté : plusieurs tâches lisent la même configuration).
#[derive(Clone)]
pub struct TlsSetup {
    /// Configuration des connexions normales (navigateur, launcher).
    default: Arc<ServerConfig>,
    /// Configuration des connexions de vérification de Let's Encrypt (défi
    /// TLS-ALPN-01) ; `None` avec des fichiers PEM.
    challenge: Option<Arc<ServerConfig>>,
}

/// Taille maximale d'un fichier PEM : un certificat avec sa chaîne fait quelques Kio.
const MAX_PEM_BYTES: u64 = 1024 * 1024;

/// Durée maximale de la poignée de main TLS : un client trop lent est coupé.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Poignées de main en cours au plus : au-delà, on attend qu'une finisse.
const MAX_HANDSHAKES: usize = 256;

/// Protocole annoncé aux clients (ALPN) : le hub ne parle que HTTP/1.1.
const HTTP_1_1: &[u8] = b"http/1.1";

/// Fournisseur de chiffrement : `ring`, comme le reste du dépôt (pas d'aws-lc).
fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

impl TlsSetup {
    /// Lit le certificat (avec sa chaîne) et la clé privée dans deux fichiers PEM.
    pub fn from_pem_files(cert: &Path, key: &Path) -> Result<TlsSetup, TlsError> {
        let cert_pem = read_pem("le certificat", cert)?;
        let key_pem = read_pem("la clé privée", key)?;
        Self::build(
            &cert_pem,
            &cert.display().to_string(),
            &key_pem,
            &key.display().to_string(),
        )
    }

    /// Comme [`TlsSetup::from_pem_files`], à partir du contenu des fichiers.
    pub fn from_pem(cert_pem: &[u8], key_pem: &[u8]) -> Result<TlsSetup, TlsError> {
        Self::build(cert_pem, "le certificat", key_pem, "la clé")
    }

    fn build(
        cert_pem: &[u8],
        cert_name: &str,
        key_pem: &[u8],
        key_name: &str,
    ) -> Result<TlsSetup, TlsError> {
        // Le fichier du certificat peut contenir toute la chaîne : on garde tout, dans l'ordre.
        let mut certs = Vec::new();
        for cert in CertificateDer::pem_slice_iter(cert_pem) {
            certs.push(cert.map_err(|e| bad_pem(cert_name, e))?);
        }
        if certs.is_empty() {
            return Err(TlsError::NoCertificate {
                path: cert_name.to_string(),
            });
        }
        let key = match PrivateKeyDer::from_pem_slice(key_pem) {
            Ok(key) => key,
            Err(pem::Error::NoItemsFound) => {
                return Err(TlsError::NoKey {
                    path: key_name.to_string(),
                })
            }
            Err(error) => return Err(bad_pem(key_name, error)),
        };
        // rustls vérifie que la clé va avec le certificat.
        let mut config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        config.alpn_protocols = vec![HTTP_1_1.to_vec()];
        Ok(TlsSetup {
            default: Arc::new(config),
            challenge: None,
        })
    }

    /// Certificat Let's Encrypt automatique. Lance une tâche de fond qui obtient le
    /// certificat, le renouvelle avant son expiration et écrit ce qu'elle fait au
    /// journal. À appeler depuis le runtime tokio (dans `main`).
    ///
    /// Tant que le premier certificat n'est pas là, les connexions HTTPS échouent ;
    /// il arrive en général en moins d'une minute.
    pub fn acme(options: &AcmeOptions) -> Result<TlsSetup, TlsError> {
        std::fs::create_dir_all(&options.cache_dir).map_err(|source| TlsError::CacheDir {
            path: options.cache_dir.display().to_string(),
            source,
        })?;
        let contacts: Vec<String> = options
            .contact_email
            .iter()
            .map(|email| format!("mailto:{email}"))
            .collect();
        // `directory_lets_encrypt(true)` = vrai serveur, `false` = serveur d'essai.
        let mut state = rustls_acme::AcmeConfig::new([options.domain.clone()])
            .contact(contacts)
            .cache(rustls_acme::caches::DirCache::new(
                options.cache_dir.clone(),
            ))
            .directory_lets_encrypt(!options.staging)
            .state();

        // Le « résolveur » donne à rustls le certificat du moment : il change tout
        // seul au renouvellement, sans redémarrer le hub.
        let mut default = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_cert_resolver(state.resolver());
        default.alpn_protocols = vec![HTTP_1_1.to_vec()];
        let challenge = state.challenge_rustls_config_with_provider(provider());

        // `state` est un flux (`Stream`) d'événements : il faut le faire avancer pour
        // que les commandes, renouvellements compris, aient lieu.
        let domain = options.domain.clone();
        tokio::spawn(async move {
            while let Some(event) = state.next().await {
                match event {
                    Ok(ok) => tracing::info!(domaine = %domain, "Let's Encrypt : {ok:?}"),
                    Err(err) => tracing::warn!(domaine = %domain, "Let's Encrypt : {err}"),
                }
            }
        });
        Ok(TlsSetup {
            default: Arc::new(default),
            challenge: Some(challenge),
        })
    }
}

/// Lit un fichier PEM, au plus [`MAX_PEM_BYTES`].
fn read_pem(what: &'static str, path: &Path) -> Result<Vec<u8>, TlsError> {
    let read_error = |source| TlsError::Read {
        what,
        path: path.display().to_string(),
        source,
    };
    let file = std::fs::File::open(path).map_err(read_error)?;
    let mut bytes = Vec::new();
    // `take` : on ne lit jamais plus que la limite (+ 1 octet pour savoir si elle est dépassée).
    file.take(MAX_PEM_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() as u64 > MAX_PEM_BYTES {
        return Err(TlsError::TooLarge {
            what,
            path: path.display().to_string(),
        });
    }
    Ok(bytes)
}

fn bad_pem(name: &str, error: pem::Error) -> TlsError {
    TlsError::BadPem {
        path: name.to_string(),
        message: error.to_string(),
    }
}

/// Sert `app` sur `listener`, en HTTPS si `tls` est donné, en HTTP sinon, jusqu'à
/// ce que `shutdown` se termine.
pub async fn serve<F>(
    listener: TcpListener,
    tls: Option<TlsSetup>,
    app: axum::Router,
    shutdown: F,
) -> std::io::Result<()>
where
    // `Future + Send + 'static` : axum attend le signal d'arrêt dans une autre tâche.
    F: std::future::Future<Output = ()> + Send + 'static,
{
    match tls {
        None => {
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
        }
        Some(tls) => {
            axum::serve(TlsListener::new(listener, tls)?, app)
                .with_graceful_shutdown(shutdown)
                .await
        }
    }
}

/// Écoute TCP qui rend des connexions déjà chiffrées, pour `axum::serve`.
///
/// Les poignées de main TLS se font dans des tâches à part : un client lent ne
/// bloque pas les autres. Une tâche de fond accepte les connexions TCP et envoie
/// les connexions TLS prêtes dans un canal, que `accept` lit.
struct TlsListener {
    ready: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local_addr: SocketAddr,
}

impl TlsListener {
    fn new(tcp: TcpListener, tls: TlsSetup) -> std::io::Result<TlsListener> {
        let local_addr = tcp.local_addr()?;
        let (sender, ready) = mpsc::channel(64);
        tokio::spawn(accept_loop(tcp, tls, sender));
        Ok(TlsListener { ready, local_addr })
    }
}

// Le trait `Listener` d'axum : ce qu'il faut à `axum::serve` pour recevoir des connexions.
impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.ready.recv().await {
            Some(connection) => connection,
            // La tâche de fond ne s'arrête que si ce `TlsListener` disparaît :
            // ce cas n'arrive pas, on attend sans fin plutôt que de planter.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.local_addr)
    }
}

/// Accepte les connexions TCP et lance une poignée de main TLS pour chacune.
/// S'arrête (et libère le port) dès que le serveur ne lit plus le canal.
async fn accept_loop(
    tcp: TcpListener,
    tls: TlsSetup,
    sender: mpsc::Sender<(TlsStream<TcpStream>, SocketAddr)>,
) {
    let slots = Arc::new(Semaphore::new(MAX_HANDSHAKES));
    loop {
        // `select!` attend la première des deux : arrêt du serveur ou nouvelle connexion.
        let accepted = tokio::select! {
            _ = sender.closed() => return,
            accepted = tcp.accept() => accepted,
        };
        let (stream, addr) = match accepted {
            Ok(pair) => pair,
            Err(error) => {
                // Trop de fichiers ouverts, connexion coupée… : on réessaie un peu après.
                tracing::debug!(%error, "connexion TCP refusée");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        // Une place parmi les poignées de main en cours ; `Err` = sémaphore fermé (jamais).
        let Ok(slot) = Arc::clone(&slots).acquire_owned().await else {
            return;
        };
        let tls = tls.clone();
        let sender = sender.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(stream, &tls)).await;
            drop(slot);
            match result {
                Ok(Ok(Some(connection))) => {
                    // Le serveur s'est arrêté entre-temps : la connexion est abandonnée.
                    let _ = sender.send((connection, addr)).await;
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => tracing::debug!(%error, "poignée de main TLS ratée"),
                Err(_) => tracing::debug!("poignée de main TLS trop lente"),
            }
        });
    }
}

/// Poignée de main TLS. `Ok(None)` : connexion de vérification de Let's Encrypt,
/// terminée ici (elle ne porte pas de requête HTTP).
async fn handshake(
    stream: TcpStream,
    tls: &TlsSetup,
) -> std::io::Result<Option<TlsStream<TcpStream>>> {
    // On lit d'abord le « bonjour » du client pour choisir la configuration.
    let start = LazyConfigAcceptor::new(rustls::server::Acceptor::default(), stream).await?;
    if let Some(challenge) = &tls.challenge {
        if rustls_acme::is_tls_alpn_challenge(&start.client_hello()) {
            tracing::info!("vérification TLS-ALPN-01 de Let's Encrypt");
            let mut connection = start.into_stream(Arc::clone(challenge)).await?;
            connection.shutdown().await?;
            return Ok(None);
        }
    }
    let connection = start.into_stream(Arc::clone(&tls.default)).await?;
    Ok(Some(connection))
}
