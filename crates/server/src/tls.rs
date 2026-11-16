//! Chiffrement `wss` : certificat et clé privée lus dans des fichiers PEM (ADR 0009).
//!
//! Mêmes fichiers que pour le serveur officiel (`--cert`, `--cert_key`) : par
//! exemple `fullchain.pem` et `privkey.pem` de Let's Encrypt.

use std::path::Path;
use std::sync::Arc;

use rustls::pki_types::pem::{self, PemObject};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

use crate::error::TlsError;

/// Configuration TLS prête à accepter des clients.
///
/// `Clone` est bon marché : `TlsAcceptor` partage la configuration par un `Arc`.
#[derive(Clone)]
pub struct TlsConfig {
    acceptor: TlsAcceptor,
}

impl TlsConfig {
    /// Lit le certificat (avec sa chaîne) et la clé privée dans deux fichiers PEM.
    pub fn from_pem_files(cert: &Path, key: &Path) -> Result<TlsConfig, TlsError> {
        let cert_pem = read(cert)?;
        let key_pem = read(key)?;
        Self::build(
            &cert_pem,
            &cert.display().to_string(),
            &key_pem,
            &key.display().to_string(),
        )
    }

    /// Comme [`TlsConfig::from_pem_files`], à partir du contenu des fichiers.
    pub fn from_pem(cert_pem: &[u8], key_pem: &[u8]) -> Result<TlsConfig, TlsError> {
        Self::build(cert_pem, "le certificat", key_pem, "la clé")
    }

    fn build(
        cert_pem: &[u8],
        cert_name: &str,
        key_pem: &[u8],
        key_name: &str,
    ) -> Result<TlsConfig, TlsError> {
        // Le fichier du certificat peut contenir toute la chaîne : on garde
        // tous les certificats, dans l'ordre.
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
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        Ok(TlsConfig {
            acceptor: TlsAcceptor::from(Arc::new(config)),
        })
    }

    pub(crate) fn acceptor(&self) -> TlsAcceptor {
        self.acceptor.clone()
    }
}

fn read(path: &Path) -> Result<Vec<u8>, TlsError> {
    std::fs::read(path).map_err(|source| TlsError::Read {
        path: path.display().to_string(),
        source,
    })
}

fn bad_pem(name: &str, error: pem::Error) -> TlsError {
    TlsError::BadPem {
        path: name.to_string(),
        message: error.to_string(),
    }
}
