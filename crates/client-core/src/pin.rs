//! Épinglage d'un certificat par son empreinte SHA-256 (ADR 0043).
//!
//! Un serveur hébergé par le launcher a un certificat auto-signé, généré par Raftup.
//! Aucune autorité ne le signe : un client ordinaire le refuse. Le lien d'invitation
//! porte son empreinte (`fp`) ; ce module accepte le certificat **seulement** si son
//! empreinte est exactement celle-là. Le nom du serveur n'est pas vérifié : un
//! certificat auto-signé n'a pas de nom fiable, c'est l'empreinte qui l'identifie.
//!
//! La signature de la poignée de main est, elle, vérifiée normalement : le serveur
//! prouve qu'il a la clé privée du certificat épinglé.

use std::fmt;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

/// Empreinte SHA-256 d'un certificat (sur sa forme DER), 32 octets.
///
/// Texte : 64 chiffres hexadécimaux en minuscules, sans séparateur.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    /// Empreinte d'un certificat DER.
    pub fn of_der(der: &[u8]) -> Fingerprint {
        let digest = Sha256::digest(der);
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&digest);
        Fingerprint(bytes)
    }

    /// Lit 64 chiffres hexadécimaux (majuscules acceptées). `None` sinon : un texte
    /// plus court, plus long ou avec un autre caractère est refusé.
    pub fn parse(text: &str) -> Option<Fingerprint> {
        let text = text.as_bytes();
        if text.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        // `chunks(2)` : deux caractères hexadécimaux par octet.
        for (byte, pair) in bytes.iter_mut().zip(text.chunks(2)) {
            let high = hex_value(pair[0])?;
            let low = hex_value(pair[1])?;
            *byte = high * 16 + low;
        }
        Some(Fingerprint(bytes))
    }

    /// Forme texte : 64 chiffres hexadécimaux en minuscules.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// L'empreinte est publique (elle est dans le lien) : on peut l'afficher.
impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", self.to_hex())
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Vérificateur rustls : le certificat du serveur doit avoir l'empreinte épinglée.
///
/// `ServerCertVerifier` est un trait de rustls (une « interface ») : rustls appelle
/// ces méthodes pendant la poignée de main, à la place de sa vérification habituelle.
#[derive(Debug)]
struct PinnedVerifier {
    pin: Fingerprint,
    /// Algorithmes de signature du moteur `ring`, pour vérifier la poignée de main.
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if Fingerprint::of_der(end_entity.as_ref()) == self.pin {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "empreinte du certificat différente de celle du lien".to_string(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Configuration TLS client qui ne croit que le certificat d'empreinte `pin`.
pub(crate) fn pinned_config(pin: Fingerprint) -> rustls::ClientConfig {
    let provider = rustls::crypto::ring::default_provider();
    let verifier = PinnedVerifier {
        pin,
        algorithms: provider.signature_verification_algorithms,
    };
    rustls::ClientConfig::builder()
        // `dangerous()` : rustls oblige à le dire quand on remplace sa vérification.
        // Ici elle est remplacée par une vérification plus stricte (une seule empreinte).
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empreinte_texte_aller_retour() {
        let fp = Fingerprint::of_der(b"certificat");
        let text = fp.to_hex();
        assert_eq!(text.len(), 64);
        assert_eq!(Fingerprint::parse(&text), Some(fp));
        assert_eq!(Fingerprint::parse(&text.to_uppercase()), Some(fp));
        // Mauvaises longueurs, mauvais caractères.
        assert_eq!(Fingerprint::parse(&text[..63]), None);
        assert_eq!(Fingerprint::parse(&format!("{text}0")), None);
        assert_eq!(Fingerprint::parse(&format!("{}g", &text[..63])), None);
        assert_eq!(Fingerprint::parse(""), None);
        // Un caractère non ASCII ne fait pas paniquer le découpage.
        assert_eq!(Fingerprint::parse(&format!("{}é", &text[..62])), None);
    }
}
