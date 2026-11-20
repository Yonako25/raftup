//! Sonde de santé : `rafthub healthcheck` (déploiement Docker, `deploy/rafthub/`).
//!
//! L'image Docker du hub ne contient ni `curl` ni `wget`. Le binaire sait donc
//! vérifier lui-même qu'un hub répond : il demande la feuille de style publique
//! (`/static/rafthub.css`) et attend un code 200. Cette page répond aussi bien
//! pendant l'assistant de premier lancement que hub installé : un hub qui attend son
//! installation est « en bonne santé », sinon Traefik ne le publierait pas et
//! personne ne pourrait ouvrir l'assistant. Pas `/admin/static/…` : avec
//! `--admin-listen`, `/admin` répond 404 sur l'adresse publique (ADR 0040).
//! Avec `--admin-listen`, l'assistant tourne sur l'adresse de la console : l'adresse
//! publique ne répond qu'une fois le hub installé.
//!
//! Rien que la bibliothèque standard : une requête HTTP/1.1 minimale suffit, et la
//! sonde ne touche jamais à la base de données.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::error::HubError;

/// Chemin demandé par la sonde.
pub const PROBE_PATH: &str = "/static/rafthub.css";

/// Adresse à joindre pour une adresse d'écoute donnée. Un hub qui écoute sur
/// `0.0.0.0` (toutes les interfaces, cas du conteneur) se joint sur `127.0.0.1` ;
/// de même `[::]` → `[::1]`.
pub fn probe_address(listen: &str) -> String {
    if let Some(port) = listen.strip_prefix("0.0.0.0:") {
        return format!("127.0.0.1:{port}");
    }
    if let Some(port) = listen.strip_prefix("[::]:") {
        return format!("[::1]:{port}");
    }
    listen.to_string()
}

/// Vérifie que le hub qui écoute sur `listen` répond 200 à [`PROBE_PATH`].
pub fn check(listen: &str, timeout: Duration) -> Result<(), HubError> {
    let target = probe_address(listen);
    let fail = |what: &str| HubError::Internal(format!("sonde {target} : {what}"));
    // `to_socket_addrs` résout l'adresse ; on garde la première.
    let addr: SocketAddr = target
        .to_socket_addrs()
        .map_err(|_| fail("adresse invalide"))?
        .next()
        .ok_or_else(|| fail("adresse invalide"))?;
    let mut stream =
        TcpStream::connect_timeout(&addr, timeout).map_err(|e| fail(&e.to_string()))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| fail(&e.to_string()))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| fail(&e.to_string()))?;
    let request =
        format!("GET {PROBE_PATH} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| fail(&e.to_string()))?;
    // Seule la première ligne compte (« HTTP/1.1 200 OK ») : on lit peu.
    let mut head = [0u8; 64];
    let mut read = 0;
    while read < head.len() {
        match stream.read(&mut head[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(e) => return Err(fail(&e.to_string())),
        }
        if head[..read].contains(&b'\n') {
            break;
        }
    }
    let status_line = String::from_utf8_lossy(&head[..read]);
    let status = status_line.split_whitespace().nth(1).unwrap_or("");
    if status == "200" {
        Ok(())
    } else {
        Err(fail(&format!("réponse {status:?} au lieu de 200")))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn adresse_de_toutes_les_interfaces_jointe_en_local() {
        assert_eq!(probe_address("0.0.0.0:8787"), "127.0.0.1:8787");
        assert_eq!(probe_address("[::]:8787"), "[::1]:8787");
        assert_eq!(probe_address("127.0.0.1:9000"), "127.0.0.1:9000");
    }

    /// Petit serveur qui répond une seule fois `response` et rend son adresse.
    fn one_shot_server(response: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 512];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(response.as_bytes());
        });
        addr
    }

    #[test]
    fn sonde_reussit_sur_200() {
        let addr = one_shot_server("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        check(&addr, Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn sonde_echoue_sur_503() {
        let addr = one_shot_server("HTTP/1.1 503 Service Unavailable\r\n\r\n");
        assert!(check(&addr, Duration::from_secs(5)).is_err());
    }

    #[test]
    fn sonde_echoue_sans_serveur() {
        // Port pris puis rendu : plus personne n'y écoute.
        let addr = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().to_string()
        };
        assert!(check(&addr, Duration::from_secs(2)).is_err());
    }
}
