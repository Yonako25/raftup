//! HTTPS intégré (ADR 0039) : le hub sert en HTTPS avec un certificat auto-signé
//! créé pendant le test (`rcgen`), et un client qui fait confiance à ce certificat.
//! Aucun appel vers Let's Encrypt : la partie ACME n'est pas testée ici.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::net::SocketAddr;
use std::sync::Arc;

use hub::tls::{TlsError, TlsSetup};
use hub::HubConfig;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Certificat auto-signé pour `localhost` : (PEM du certificat, PEM de la clé, DER).
fn self_signed() -> (String, String, Vec<u8>) {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    (
        certified.cert.pem(),
        certified.key_pair.serialize_pem(),
        certified.cert.der().to_vec(),
    )
}

/// Lance le hub en HTTPS sur un port libre ; rend son adresse, l'émetteur qui
/// l'arrête et la tâche du serveur.
async fn start_https(
    setup: TlsSetup,
) -> (
    SocketAddr,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let app = common::app_with(HubConfig::for_tests()).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(hub::tls::serve(listener, Some(setup), app, async {
        let _ = stopped.await;
    }));
    (addr, stop, server)
}

/// Client qui fait confiance à `cert_der` seulement ; les codes 4xx ne sont pas des erreurs.
fn trusting_agent(cert_der: Vec<u8>) -> ureq::Agent {
    let root = ureq::tls::Certificate::from_der(&cert_der).to_owned();
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::Specific(Arc::new(vec![root])))
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .http_status_as_error(false)
        .build()
        .into()
}

/// GET bloquant (ureq), lancé hors du runtime tokio : rend le code HTTP ou l'erreur.
async fn get(agent: ureq::Agent, url: String) -> Result<u16, String> {
    tokio::task::spawn_blocking(move || {
        agent
            .get(&url)
            .call()
            .map(|response| response.status().as_u16())
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn le_hub_repond_en_https() {
    let (cert, key, der) = self_signed();
    let setup = TlsSetup::from_pem(cert.as_bytes(), key.as_bytes()).unwrap();
    let (addr, stop, server) = start_https(setup).await;

    // Sans jeton, `/v1/me` répond 401 : c'est bien l'API du hub, en HTTPS.
    let url = format!("https://localhost:{}/v1/me", addr.port());
    assert_eq!(get(trusting_agent(der), url).await, Ok(401));

    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn client_sans_confiance_et_http_en_clair_refuses() {
    let (cert, key, _) = self_signed();
    let setup = TlsSetup::from_pem(cert.as_bytes(), key.as_bytes()).unwrap();
    let (addr, stop, server) = start_https(setup).await;

    // Client normal (racines publiques) : le certificat auto-signé est refusé.
    let normal: ureq::Agent = ureq::Agent::config_builder().build().into();
    let url = format!("https://localhost:{}/v1/me", addr.port());
    assert!(get(normal.clone(), url).await.is_err());
    // HTTP en clair sur le port HTTPS : pas de réponse HTTP.
    let url = format!("http://localhost:{}/v1/me", addr.port());
    assert!(get(normal, url).await.is_err());

    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn le_port_est_libere_a_l_arret() {
    // L'assistant d'installation s'arrête puis le vrai hub réécoute sur le même port.
    let (cert, key, der) = self_signed();
    let setup = TlsSetup::from_pem(cert.as_bytes(), key.as_bytes()).unwrap();
    let (addr, stop, server) = start_https(setup.clone()).await;
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();

    // La tâche d'écoute rend le port peu après l'arrêt : quelques essais au plus.
    let mut listener = None;
    for _ in 0..50 {
        if let Ok(bound) = TcpListener::bind(addr).await {
            listener = Some(bound);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let listener = listener.expect("port toujours occupé après l'arrêt");
    let app = common::app_with(HubConfig::for_tests()).await;
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(hub::tls::serve(listener, Some(setup), app, async {
        let _ = stopped.await;
    }));
    let url = format!("https://localhost:{}/v1/me", addr.port());
    assert_eq!(get(trusting_agent(der), url).await, Ok(401));
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}

#[test]
fn fichier_absent_message_clair() {
    let dir = tempfile::tempdir().unwrap();
    let cert = dir.path().join("fullchain.pem");
    let key = dir.path().join("privkey.pem");
    let error = TlsSetup::from_pem_files(&cert, &key).err().unwrap();
    assert!(matches!(error, TlsError::Read { .. }), "{error}");
    let message = error.to_string();
    assert!(message.contains("le certificat"), "{message}");
    assert!(message.contains("fullchain.pem"), "{message}");

    // Certificat présent, clé absente : le message parle de la clé.
    let (cert_pem, _, _) = self_signed();
    std::fs::write(&cert, cert_pem).unwrap();
    let message = TlsSetup::from_pem_files(&cert, &key)
        .err()
        .unwrap()
        .to_string();
    assert!(message.contains("la clé privée"), "{message}");
    assert!(message.contains("privkey.pem"), "{message}");
}

#[test]
fn fichiers_pem_lus_depuis_le_disque() {
    let dir = tempfile::tempdir().unwrap();
    let (cert_pem, key_pem, _) = self_signed();
    let cert = dir.path().join("fullchain.pem");
    let key = dir.path().join("privkey.pem");
    std::fs::write(&cert, cert_pem).unwrap();
    std::fs::write(&key, key_pem).unwrap();
    assert!(TlsSetup::from_pem_files(&cert, &key).is_ok());
}

#[test]
fn pem_vides_ou_melanges_refuses() {
    let (cert, key, _) = self_signed();
    let (_, other_key, _) = self_signed();
    // Pas de certificat dans le fichier (on a donné la clé à la place).
    assert!(matches!(
        TlsSetup::from_pem(key.as_bytes(), key.as_bytes()),
        Err(TlsError::NoCertificate { .. })
    ));
    // Pas de clé dans le fichier (on a donné le certificat à la place).
    assert!(matches!(
        TlsSetup::from_pem(cert.as_bytes(), cert.as_bytes()),
        Err(TlsError::NoKey { .. })
    ));
    // Clé d'un autre certificat : rustls refuse la paire.
    assert!(matches!(
        TlsSetup::from_pem(cert.as_bytes(), other_key.as_bytes()),
        Err(TlsError::Rejected(_))
    ));
}

#[test]
fn fichier_trop_gros_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let cert = dir.path().join("enorme.pem");
    std::fs::write(&cert, vec![b'A'; 1024 * 1024 + 1]).unwrap();
    let error = TlsSetup::from_pem_files(&cert, &cert).err().unwrap();
    assert!(matches!(error, TlsError::TooLarge { .. }), "{error}");
}
