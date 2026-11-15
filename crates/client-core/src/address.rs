//! Transforme l'adresse saisie par le joueur en URL WebSocket à essayer.
//!
//! Règles (ADR 0005) :
//! - `wss://…` ou `ws://…` : utilisée telle quelle ;
//! - sans schéma : `wss://` d'abord, puis `ws://` en secours ;
//! - sans port : le port par défaut d'Archipelago, 38281.

use crate::error::ClientError;

/// Port par défaut d'un serveur Archipelago.
pub const DEFAULT_PORT: u16 = 38281;

/// Liste des URL à essayer, dans l'ordre.
pub fn candidate_urls(address: &str) -> Result<Vec<String>, ClientError> {
    let address = address.trim();
    if address.is_empty() {
        return Err(ClientError::InvalidAddress("adresse vide".into()));
    }
    // `split_once` coupe au premier « :// » : (schéma, reste).
    if let Some((scheme, rest)) = address.split_once("://") {
        return match scheme {
            "ws" | "wss" => Ok(vec![format!("{scheme}://{}", with_port(rest))]),
            other => Err(ClientError::InvalidAddress(format!(
                "schéma « {other} » non pris en charge (wss ou ws)"
            ))),
        };
    }
    let host = with_port(address);
    Ok(vec![format!("wss://{host}"), format!("ws://{host}")])
}

/// Ajoute `:38281` si l'adresse n'a pas de port.
fn with_port(address: &str) -> String {
    // Partie « hôte:port », avant un éventuel chemin.
    let (host_port, path) = match address.find('/') {
        Some(i) => address.split_at(i),
        None => (address, ""),
    };
    // Une adresse IPv6 s'écrit entre crochets : « [::1]:38281 ».
    let host_end = host_port.rfind(']').map_or(0, |i| i + 1);
    let has_port = host_port[host_end..].contains(':');
    if has_port {
        address.to_string()
    } else {
        format!("{host_port}:{DEFAULT_PORT}{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sans_schema_wss_puis_ws() {
        assert_eq!(
            candidate_urls("archipelago.gg:38281").unwrap(),
            ["wss://archipelago.gg:38281", "ws://archipelago.gg:38281"]
        );
    }

    #[test]
    fn schema_explicite_garde_tel_quel() {
        assert_eq!(
            candidate_urls("ws://localhost:1234").unwrap(),
            ["ws://localhost:1234"]
        );
        assert_eq!(
            candidate_urls(" wss://archipelago.gg:50000 ").unwrap(),
            ["wss://archipelago.gg:50000"]
        );
    }

    #[test]
    fn port_par_defaut() {
        assert_eq!(
            candidate_urls("wss://archipelago.gg").unwrap(),
            ["wss://archipelago.gg:38281"]
        );
        assert_eq!(
            candidate_urls("[::1]").unwrap(),
            ["wss://[::1]:38281", "ws://[::1]:38281"]
        );
        assert_eq!(
            candidate_urls("ws://[::1]:5000").unwrap(),
            ["ws://[::1]:5000"]
        );
    }

    #[test]
    fn adresses_refusees() {
        assert!(candidate_urls("").is_err());
        assert!(candidate_urls("http://archipelago.gg").is_err());
    }
}
