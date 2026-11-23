//! Certificat des parties hébergées par le hub (ADR 0052, point 4 ; ADR 0043).
//!
//! Chaque partie sert `wss` **et** `ws` sur son port (port mixte de l'ADR 0043) :
//! le launcher épingle le certificat par son empreinte (`fp` du lien d'invitation),
//! un mod non modifié retombe sur `ws`. Le certificat est auto-signé, créé une fois
//! par le hub dans `<parties>/tls/`, puis réutilisé (10 ans) : les liens d'invitation
//! restent valides d'un redémarrage à l'autre.
//!
//! Plus tard (étape 5 de l'ADR 0052), le certificat du domaine du hub pourra le
//! remplacer ; l'empreinte reste alors utile au launcher.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::CertificateDer;
use sha2::{Digest, Sha256};

use crate::error::HubError;

const CERT_FILE: &str = "cert.pem";
const KEY_FILE: &str = "key.pem";
/// Durée de validité, en années.
const VALID_YEARS: i32 = 10;

/// Le certificat des parties. Pas de `Debug` : il afficherait la clé.
pub struct GameCertificate {
    pub cert_pem: String,
    /// Clé privée (PEM) : jamais au journal.
    pub key_pem: String,
    /// Empreinte SHA-256 du certificat (DER), 64 chiffres hexadécimaux en minuscules,
    /// comme `client_core::Fingerprint::to_hex`.
    pub fingerprint: String,
}

/// Le certificat de `dir`, créé s'il manque.
pub fn ensure(dir: &Path) -> Result<GameCertificate, HubError> {
    let cert_path = dir.join(CERT_FILE);
    let key_path = dir.join(KEY_FILE);
    if cert_path.exists() && key_path.exists() {
        let read = |path: &Path| {
            std::fs::read_to_string(path)
                .map_err(|e| HubError::Internal(format!("certificat des parties illisible : {e}")))
        };
        let cert_pem = read(&cert_path)?;
        let key_pem = read(&key_path)?;
        let fingerprint = fingerprint_of(&cert_pem)?;
        return Ok(GameCertificate {
            cert_pem,
            key_pem,
            fingerprint,
        });
    }
    let generated = generate()?;
    std::fs::create_dir_all(dir)
        .map_err(|e| HubError::Internal(format!("dossier du certificat : {e}")))?;
    // La clé d'abord : un certificat sans sa clé serait inutilisable.
    write(&key_path, &generated.key_pem)?;
    write(&cert_path, &generated.cert_pem)?;
    tracing::info!(fingerprint = %generated.fingerprint, "certificat des parties créé");
    Ok(generated)
}

fn generate() -> Result<GameCertificate, HubError> {
    let error = |e: rcgen::Error| HubError::Internal(format!("génération du certificat : {e}"));
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).map_err(error)?;
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        "Rafthub (certificat des parties)",
    );
    let year = current_year();
    params.not_before = rcgen::date_time_ymd(year, 1, 1);
    params.not_after = rcgen::date_time_ymd(year + VALID_YEARS, 1, 1);
    let key = rcgen::KeyPair::generate().map_err(error)?;
    let cert = params.self_signed(&key).map_err(error)?;
    Ok(GameCertificate {
        fingerprint: hex(&Sha256::digest(cert.der())),
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

fn fingerprint_of(cert_pem: &str) -> Result<String, HubError> {
    let der = CertificateDer::from_pem_slice(cert_pem.as_bytes())
        .map_err(|_| HubError::Internal("certificat des parties mal formé".to_string()))?;
    Ok(hex(&Sha256::digest(der.as_ref())))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn current_year() -> i32 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // 31 556 952 s : la durée moyenne d'une année grégorienne.
    1970 + i32::try_from(seconds / 31_556_952).unwrap_or(0)
}

/// Écrit un fichier ; la clé privée n'est lisible que par le compte du hub sous Unix.
fn write(path: &Path, text: &str) -> Result<(), HubError> {
    let error = |e: std::io::Error| HubError::Internal(format!("écriture du certificat : {e}"));
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(error)?;
        file.write_all(text.as_bytes()).map_err(error)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, text).map_err(error)
    }
}
