//! Statut « prêt ? » d'un slot, testé par une vraie connexion (F08, ADR 0024).
//!
//! L'hôte clique sur « Tester » : on ouvre une connexion comme le ferait le jeu du joueur,
//! avec l'adresse publique de la partie et le slot, puis on la referme. Le résultat est un
//! [`Readiness`] avec une phrase en français. On réutilise [`crate::diagnose`] (F05) : aucune
//! logique de connexion n'est dupliquée ici.
//!
//! Limite : cela prouve que le serveur accepte ce slot depuis cette machine, pas que le
//! joueur a installé son mod. Le test apparaît dans le journal du serveur comme une
//! connexion brève de ce slot.

use std::time::Duration;

use crate::diagnose::{check_connection_with_timeout, ConnectionProblem};

/// Délai accordé à un test de slot.
const READY_TIMEOUT: Duration = Duration::from_secs(8);

/// Résultat du test d'un slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// Le serveur a accepté la connexion sur ce slot.
    Ready,
    /// Le serveur refuse (mot de passe, réglage de réception des objets).
    Refused(ConnectionProblem),
    /// Le chiffrement `wss` échoue.
    Tls,
    /// Pas de réponse dans le délai.
    TimedOut,
    /// Le serveur exige une autre version du client.
    WrongVersion,
    /// Le slot n'existe pas, ou est prévu pour un autre jeu.
    WrongSlot(ConnectionProblem),
    /// Serveur injoignable ou autre échec : le texte vient de `diagnose`.
    Unreachable(ConnectionProblem),
}

impl Readiness {
    /// Range un problème de connexion dans la bonne catégorie.
    pub fn from_problem(problem: ConnectionProblem) -> Readiness {
        match problem {
            ConnectionProblem::Tls => Readiness::Tls,
            ConnectionProblem::Timeout => Readiness::TimedOut,
            ConnectionProblem::IncompatibleVersion => Readiness::WrongVersion,
            ConnectionProblem::InvalidSlot | ConnectionProblem::InvalidGame => {
                Readiness::WrongSlot(problem)
            }
            ConnectionProblem::InvalidPassword | ConnectionProblem::InvalidItemsHandling => {
                Readiness::Refused(problem)
            }
            other => Readiness::Unreachable(other),
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Readiness::Ready)
    }

    /// Étiquette courte pour une pastille.
    pub fn label(&self) -> &'static str {
        match self {
            Readiness::Ready => "Prêt",
            Readiness::Refused(_) => "Refusé",
            Readiness::Tls => "Erreur TLS",
            Readiness::TimedOut => "Délai dépassé",
            Readiness::WrongVersion => "Mauvaise version",
            Readiness::WrongSlot(_) => "Mauvais slot",
            Readiness::Unreachable(_) => "Injoignable",
        }
    }

    /// Phrase en français qui dit quoi faire.
    pub fn message(&self) -> String {
        match self {
            Readiness::Ready => "Le serveur accepte ce slot : le joueur peut se connecter.".into(),
            Readiness::Tls => ConnectionProblem::Tls.message(),
            Readiness::TimedOut => ConnectionProblem::Timeout.message(),
            Readiness::WrongVersion => ConnectionProblem::IncompatibleVersion.message(),
            Readiness::Refused(p) | Readiness::WrongSlot(p) | Readiness::Unreachable(p) => {
                p.message()
            }
        }
    }
}

/// Teste un slot par une vraie connexion, avec le délai par défaut.
pub async fn check_slot(
    address: &str,
    slot: &str,
    game: &str,
    password: Option<&str>,
) -> Readiness {
    check_slot_with_timeout(address, slot, game, password, READY_TIMEOUT).await
}

/// Comme [`check_slot`], avec un délai au choix (utile aux tests).
pub async fn check_slot_with_timeout(
    address: &str,
    slot: &str,
    game: &str,
    password: Option<&str>,
    timeout: Duration,
) -> Readiness {
    match check_connection_with_timeout(address, slot, password, game, timeout).await {
        Ok(()) => Readiness::Ready,
        Err(problem) => Readiness::from_problem(problem),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classement_des_problemes() {
        let r = Readiness::from_problem;
        assert_eq!(r(ConnectionProblem::Tls), Readiness::Tls);
        assert_eq!(r(ConnectionProblem::Timeout), Readiness::TimedOut);
        assert_eq!(
            r(ConnectionProblem::IncompatibleVersion),
            Readiness::WrongVersion
        );
        assert!(matches!(
            r(ConnectionProblem::InvalidSlot),
            Readiness::WrongSlot(_)
        ));
        assert!(matches!(
            r(ConnectionProblem::InvalidPassword),
            Readiness::Refused(_)
        ));
        assert!(matches!(
            r(ConnectionProblem::PortClosed),
            Readiness::Unreachable(_)
        ));
    }
}
