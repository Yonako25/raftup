//! Explique en français pourquoi une connexion à la partie échoue (F05).
//!
//! Le joueur ne doit jamais voir « erreur WebSocket : IO error… » : on traduit chaque
//! échec en un [`ConnectionProblem`], et chaque problème a une phrase qui dit quoi faire.
//!
//! - [`explain`] : fonction pure, `ClientError` vers `ConnectionProblem` ;
//! - [`check_connection`] : essaie vraiment de se connecter (puis se déconnecte).

use std::error::Error;
use std::io::ErrorKind;
use std::time::Duration;

use client_core::{candidate_urls, Client, ClientError, ConnectOptions};

/// Ce qui empêche de se connecter, du point de vue du joueur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionProblem {
    /// L'adresse saisie n'a pas la bonne forme.
    BadAddress,
    /// Le serveur est introuvable (nom inconnu, réseau injoignable).
    Unreachable,
    /// La machine répond, mais rien n'écoute sur ce port.
    PortClosed,
    /// Le chiffrement `wss` échoue (certificat refusé, serveur sans chiffrement…).
    Tls,
    /// Pas de réponse dans le délai.
    Timeout,
    /// Le serveur ne connaît pas ce nom de slot.
    InvalidSlot,
    /// Le slot existe, mais pour un autre jeu.
    InvalidGame,
    /// Le serveur exige une autre version du client.
    IncompatibleVersion,
    /// Mot de passe de la salle manquant ou faux.
    InvalidPassword,
    /// Le réglage de réception des objets est refusé.
    InvalidItemsHandling,
    /// Tout le reste, avec le texte technique pour le journal.
    Other(String),
}

impl ConnectionProblem {
    /// Phrase courte, en français, qui dit quoi faire.
    pub fn message(&self) -> String {
        match self {
            ConnectionProblem::BadAddress => {
                "L'adresse n'est pas valide. Écris-la comme « hote:port », par exemple \
                 archipelago.gg:38281."
                    .to_string()
            }
            ConnectionProblem::Unreachable => {
                "Le serveur est introuvable. Vérifie l'adresse et ta connexion Internet."
                    .to_string()
            }
            ConnectionProblem::PortClosed => {
                "Le serveur ne répond pas sur ce port. Vérifie le numéro de port, et que la \
                 partie est bien lancée."
                    .to_string()
            }
            ConnectionProblem::Tls => {
                "La connexion chiffrée a échoué : le certificat du serveur est refusé. Demande \
                 à l'hôte un certificat valide, ou une adresse en ws:// s'il accepte le clair."
                    .to_string()
            }
            ConnectionProblem::Timeout => {
                "Le serveur met trop de temps à répondre. Vérifie l'adresse et le port, puis \
                 réessaie."
                    .to_string()
            }
            ConnectionProblem::InvalidSlot => {
                "Ce nom de slot n'existe pas dans la partie. Vérifie l'orthographe, majuscules \
                 comprises."
                    .to_string()
            }
            ConnectionProblem::InvalidGame => {
                "Ce slot est prévu pour un autre jeu. Choisis le slot qui correspond à ton jeu."
                    .to_string()
            }
            ConnectionProblem::IncompatibleVersion => {
                "La version du client ne convient pas à cette partie. Mets à jour le mod ou \
                 Raftup."
                    .to_string()
            }
            ConnectionProblem::InvalidPassword => {
                "Mot de passe refusé. Demande le bon mot de passe à l'hôte de la partie."
                    .to_string()
            }
            ConnectionProblem::InvalidItemsHandling => {
                "Le serveur refuse la façon dont ce client reçoit les objets. Mets à jour le \
                 mod du jeu."
                    .to_string()
            }
            ConnectionProblem::Other(detail) => {
                format!("La connexion a échoué ({detail}). Consulte le journal pour les détails.")
            }
        }
    }
}

/// Traduit une erreur du client en problème compréhensible. Fonction pure : pas de réseau.
pub fn explain(e: &ClientError) -> ConnectionProblem {
    match e {
        ClientError::InvalidAddress(_) => ConnectionProblem::BadAddress,
        ClientError::Timeout => ConnectionProblem::Timeout,
        ClientError::Refused(codes) => explain_refusal(codes),
        ClientError::WebSocket(inner) => explain_socket_error(inner.as_ref()),
        other => ConnectionProblem::Other(other.to_string()),
    }
}

/// Le serveur a répondu `ConnectionRefused` avec des codes d'erreur (`InvalidSlot`…).
fn explain_refusal(codes: &[String]) -> ConnectionProblem {
    // Un code peut en cacher un autre : on prend le premier qu'on reconnaît.
    for code in codes {
        match code.as_str() {
            "InvalidSlot" => return ConnectionProblem::InvalidSlot,
            "InvalidGame" => return ConnectionProblem::InvalidGame,
            "IncompatibleVersion" => return ConnectionProblem::IncompatibleVersion,
            "InvalidPassword" => return ConnectionProblem::InvalidPassword,
            "InvalidItemsHandling" => return ConnectionProblem::InvalidItemsHandling,
            _ => {}
        }
    }
    ConnectionProblem::Other(format!("refus du serveur : {}", codes.join(", ")))
}

/// Erreur réseau ou WebSocket : on regarde l'erreur d'entrée/sortie cachée dedans.
///
/// Rust : `dyn Error` + `source()` permettent de descendre la chaîne des causes, et
/// `downcast_ref::<io::Error>()` retrouve un type précis dans cette chaîne. On n'a ainsi
/// pas besoin de nommer les types de la crate WebSocket.
fn explain_socket_error(error: &(dyn Error + 'static)) -> ConnectionProblem {
    // Texte de toute la chaîne, en minuscules, pour reconnaître une erreur TLS :
    // tokio-rustls la fait passer dans une `io::Error`, sans type dédié accessible ici.
    let mut chain_text = String::new();
    let mut io_kind = None;
    let mut current: Option<&(dyn Error + 'static)> = Some(error);
    while let Some(e) = current {
        chain_text.push_str(&e.to_string().to_lowercase());
        chain_text.push(' ');
        if io_kind.is_none() {
            if let Some(io) = e.downcast_ref::<std::io::Error>() {
                io_kind = Some(io.kind());
            }
        }
        current = e.source();
    }

    if ["certificate", "tls", "handshake", "rustls", "peer sent no"]
        .iter()
        .any(|word| chain_text.contains(word))
        && io_kind != Some(ErrorKind::ConnectionRefused)
    {
        return ConnectionProblem::Tls;
    }

    match io_kind {
        Some(ErrorKind::ConnectionRefused) => ConnectionProblem::PortClosed,
        Some(ErrorKind::TimedOut) => ConnectionProblem::Timeout,
        // Connexion coupée en cours de route : pas un problème d'adresse.
        Some(
            ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::UnexpectedEof,
        ) => ConnectionProblem::Other(chain_text.trim().to_string()),
        // Nom inconnu, réseau ou hôte injoignable : le serveur n'est pas atteint.
        Some(_) => ConnectionProblem::Unreachable,
        None => ConnectionProblem::Other(chain_text.trim().to_string()),
    }
}

/// Délai total accordé à `check_connection`.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Essaie de se connecter au slot, puis se déconnecte tout de suite.
///
/// Même chose que [`check_connection_with_timeout`] avec un délai de 10 secondes.
pub async fn check_connection(
    address: &str,
    slot: &str,
    password: Option<&str>,
    game: &str,
) -> Result<(), ConnectionProblem> {
    check_connection_with_timeout(address, slot, password, game, CHECK_TIMEOUT).await
}

/// Comme [`check_connection`], avec un délai total au choix (utile aux tests).
///
/// Sans schéma dans l'adresse, `client-core` essaie `wss://` puis `ws://` et ne garde que
/// l'erreur de la **dernière** tentative : un échec TLS serait caché par l'échec du `ws://`.
/// On fait donc les tentatives nous-mêmes, une URL à la fois, pour garder la bonne erreur.
pub async fn check_connection_with_timeout(
    address: &str,
    slot: &str,
    password: Option<&str>,
    game: &str,
    timeout: Duration,
) -> Result<(), ConnectionProblem> {
    check_connection_pinned(address, slot, password, game, None, timeout).await
}

/// Comme [`check_connection_with_timeout`], avec l'empreinte du certificat du serveur
/// prise dans le lien d'invitation (ADR 0043) : en `wss`, ce seul certificat est cru.
pub async fn check_connection_pinned(
    address: &str,
    slot: &str,
    password: Option<&str>,
    game: &str,
    pinned: Option<client_core::Fingerprint>,
    timeout: Duration,
) -> Result<(), ConnectionProblem> {
    // Un bloc `async` est une valeur : `tokio::time::timeout` peut ainsi le surveiller en entier.
    let attempt = async {
        let urls = candidate_urls(address).map_err(|e| explain(&e))?;
        let mut problems: Vec<ConnectionProblem> = Vec::new();
        for url in urls {
            let mut options = ConnectOptions::new(&url, slot, game);
            options.password = password.map(str::to_string);
            options.pinned = pinned;
            match Client::connect(&options).await {
                Ok((client, _session)) => {
                    // Fermeture propre ; si elle échoue, la connexion a quand même réussi.
                    let _ = client.close().await;
                    return Ok(());
                }
                Err(error) => {
                    let problem = explain(&error);
                    // Le serveur a répondu et refuse : inutile d'essayer une autre URL.
                    if matches!(error, ClientError::Refused(_)) {
                        return Err(problem);
                    }
                    problems.push(problem);
                }
            }
        }
        // Toutes les tentatives ont échoué : une erreur TLS est la plus utile à montrer,
        // sinon la première (celle du `wss://`, l'essai principal).
        let best = problems
            .iter()
            .find(|p| **p == ConnectionProblem::Tls)
            .or(problems.first())
            .cloned();
        Err(best.unwrap_or(ConnectionProblem::Timeout))
    };

    match tokio::time::timeout(timeout, attempt).await {
        Ok(result) => result,
        Err(_elapsed) => Err(ConnectionProblem::Timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refus_du_serveur() {
        let refused = |code: &str| ClientError::Refused(vec![code.to_string()]);
        assert_eq!(
            explain(&refused("InvalidSlot")),
            ConnectionProblem::InvalidSlot
        );
        assert_eq!(
            explain(&refused("InvalidGame")),
            ConnectionProblem::InvalidGame
        );
        assert_eq!(
            explain(&refused("InvalidPassword")),
            ConnectionProblem::InvalidPassword
        );
        assert!(matches!(
            explain(&refused("Bizarre")),
            ConnectionProblem::Other(_)
        ));
    }

    #[test]
    fn adresse_et_delai() {
        assert_eq!(
            explain(&ClientError::InvalidAddress("x".into())),
            ConnectionProblem::BadAddress
        );
        assert_eq!(explain(&ClientError::Timeout), ConnectionProblem::Timeout);
    }
}
