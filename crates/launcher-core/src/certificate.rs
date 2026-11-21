//! Certificat généré par Raftup pour héberger en `wss` sans fichier PEM (ADR 0043).
//!
//! Au premier hébergement en mode « chiffré par Raftup », un certificat auto-signé et
//! sa clé sont créés (`rcgen`), rangés dans `<données>/tls/`, puis réutilisés : leur
//! empreinte SHA-256 va dans les liens d'invitation (`fp`), qui restent valides tant
//! que le certificat ne change pas. Durée : 10 ans ; une nouvelle paire n'est faite
//! qu'à la demande ([`regenerate`]) ou si les fichiers sont illisibles.
//!
//! La clé privée n'est jamais écrite au journal ni dans un lien. Sous Unix, son
//! fichier n'est lisible que par l'utilisateur (droits 600) ; sous Windows, le
//! dossier des données (`%LOCALAPPDATA%`) est déjà propre au compte.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use client_core::Fingerprint;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use thiserror::Error;

/// Nom du certificat dans le dossier `tls/`.
const CERT_FILE: &str = "raftup-cert.pem";
/// Nom de la clé privée dans le dossier `tls/`.
const KEY_FILE: &str = "raftup-key.pem";
/// Durée de validité du certificat, en années (ADR 0043).
const VALID_YEARS: i32 = 10;
/// Plus gros fichier PEM relu (un certificat fait moins de 2 Kio).
const MAX_PEM_BYTES: u64 = 64 * 1024;

/// Erreurs du certificat généré.
#[derive(Debug, Error)]
pub enum CertificateError {
    #[error("génération du certificat : {0}")]
    Generate(#[from] rcgen::Error),
    #[error("écriture de {path} : {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
}

/// Le certificat généré, prêt à servir.
// Pas de `Debug` dérivé : il afficherait la clé privée.
#[derive(Clone)]
pub struct GeneratedCertificate {
    pub cert_pem: String,
    /// Clé privée (PEM). Secrète : jamais au journal.
    pub key_pem: String,
    /// Empreinte SHA-256 du certificat, celle des liens d'invitation.
    pub fingerprint: Fingerprint,
}

impl std::fmt::Debug for GeneratedCertificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratedCertificate")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

/// Le certificat de `dir`, créé s'il n'existe pas encore (ou s'il est illisible).
pub fn ensure(dir: &Path) -> Result<GeneratedCertificate, CertificateError> {
    match load(dir) {
        Some(existing) => Ok(existing),
        None => regenerate(dir),
    }
}

/// Crée une nouvelle paire et remplace l'ancienne : les anciens liens d'invitation
/// ne passent plus l'épinglage.
pub fn regenerate(dir: &Path) -> Result<GeneratedCertificate, CertificateError> {
    let generated = generate(current_year())?;
    std::fs::create_dir_all(dir).map_err(|source| write_error(dir, source))?;
    // La clé d'abord, avec ses droits restreints, puis le certificat : un certificat
    // sans clé serait relu comme « illisible » et refait au lancement suivant.
    write_private(&dir.join(KEY_FILE), generated.key_pem.as_bytes())?;
    write_file(&dir.join(CERT_FILE), generated.cert_pem.as_bytes())?;
    tracing::info!(fingerprint = %generated.fingerprint, "certificat Raftup généré");
    Ok(generated)
}

/// Relit la paire de `dir`. `None` si un fichier manque ou ne se lit pas (la cause va
/// au journal, sans le contenu de la clé).
fn load(dir: &Path) -> Option<GeneratedCertificate> {
    let cert_path = dir.join(CERT_FILE);
    let key_path = dir.join(KEY_FILE);
    if !cert_path.exists() && !key_path.exists() {
        return None;
    }
    let read = |path: &PathBuf| match multidata::read_file_bounded(path, MAX_PEM_BYTES) {
        Ok(bytes) => String::from_utf8(bytes).ok(),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "certificat Raftup illisible");
            None
        }
    };
    let cert_pem = read(&cert_path)?;
    let key_pem = read(&key_path)?;
    let Ok(der) = CertificateDer::from_pem_slice(cert_pem.as_bytes()) else {
        tracing::warn!("certificat Raftup mal formé : une nouvelle paire sera créée");
        return None;
    };
    if PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).is_err() {
        tracing::warn!("clé du certificat Raftup mal formée : une nouvelle paire sera créée");
        return None;
    }
    Some(GeneratedCertificate {
        fingerprint: Fingerprint::of_der(der.as_ref()),
        cert_pem,
        key_pem,
    })
}

/// Fabrique un certificat auto-signé valable de l'an `year` à l'an `year + 10`.
/// Le nom n'a pas d'importance : les clients Raftup l'épinglent par son empreinte.
fn generate(year: i32) -> Result<GeneratedCertificate, CertificateError> {
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()])?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Raftup (certificat de partie)");
    params.not_before = rcgen::date_time_ymd(year, 1, 1);
    params.not_after = rcgen::date_time_ymd(year + VALID_YEARS, 1, 1);
    let key = rcgen::KeyPair::generate()?;
    let cert = params.self_signed(&key)?;
    Ok(GeneratedCertificate {
        fingerprint: Fingerprint::of_der(cert.der()),
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

/// L'année en cours (assez précis ici : le certificat commence le 1er janvier).
fn current_year() -> i32 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // 31 556 952 s : la durée moyenne d'une année grégorienne.
    1970 + i32::try_from(seconds / 31_556_952).unwrap_or(0)
}

fn write_error(path: &Path, source: std::io::Error) -> CertificateError {
    CertificateError::Write {
        path: path.display().to_string(),
        source,
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), CertificateError> {
    std::fs::write(path, bytes).map_err(|source| write_error(path, source))
}

/// Écrit la clé privée, lisible par le seul utilisateur sous Unix.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), CertificateError> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        // `mode(0o600)` : droits posés à la création, avant d'écrire la clé.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|source| write_error(path, source))?;
        file.write_all(bytes)
            .map_err(|source| write_error(path, source))
    }
    // Ailleurs (Windows) : le dossier des données est déjà propre au compte.
    #[cfg(not(unix))]
    {
        write_file(path, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cree_puis_reutilise() {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let dir = tmp.path().join("tls");
        let first = ensure(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert!(dir.join(CERT_FILE).exists() && dir.join(KEY_FILE).exists());
        // Deuxième appel : la même paire, donc la même empreinte (liens toujours bons).
        let second = ensure(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(first.key_pem, second.key_pem);
        // À la demande : une nouvelle empreinte.
        let third = regenerate(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert_ne!(first.fingerprint, third.fingerprint);
        // Le Debug ne montre jamais la clé.
        assert!(!format!("{third:?}").contains("PRIVATE KEY"));
    }

    #[test]
    fn fichier_abime_refait() {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let dir = tmp.path().to_path_buf();
        let first = ensure(&dir).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.join(CERT_FILE), "pas un certificat").unwrap_or_else(|e| panic!("{e}"));
        let again = ensure(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert_ne!(first.fingerprint, again.fingerprint);
    }

    #[cfg(unix)]
    #[test]
    fn cle_lisible_par_l_utilisateur_seul() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        ensure(tmp.path()).unwrap_or_else(|e| panic!("{e}"));
        let mode = std::fs::metadata(tmp.path().join(KEY_FILE))
            .unwrap_or_else(|e| panic!("{e}"))
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn valable_dix_ans() {
        let generated = generate(2026).unwrap_or_else(|e| panic!("{e}"));
        assert!(generated.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(current_year() >= 2026);
    }
}
