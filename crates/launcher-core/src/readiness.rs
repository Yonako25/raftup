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

use crate::diagnose::{check_connection_pinned, check_connection_with_timeout, ConnectionProblem};

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

/// Pause entre deux re-tests automatiques d'un même slot, en secondes.
pub const AUTO_REFRESH_INTERVAL: f64 = 60.0;

/// Ce que la cadence a besoin de savoir d'un slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotTiming<'a> {
    pub name: &'a str,
    /// Un test est déjà en cours pour ce slot.
    pub checking: bool,
    /// Heure (secondes depuis 1970) de la fin du dernier test, `None` si jamais testé.
    pub last_tested: Option<f64>,
}

/// Cadence des re-tests automatiques : logique pure, l'heure `now` est passée en paramètre
/// (aucune horloge lue ici), ce qui permet de la tester sans attendre.
///
/// Un slot est à re-tester si `enabled`, s'il n'est pas déjà en test, s'il a déjà été testé
/// une fois (le premier test reste un geste de l'hôte : il peut avoir à saisir le mot de
/// passe) et si `interval` secondes se sont écoulées depuis la fin de ce test. Une horloge
/// qui recule ne déclenche rien.
pub fn slots_due<'a>(
    enabled: bool,
    now: f64,
    interval: f64,
    slots: &[SlotTiming<'a>],
) -> Vec<&'a str> {
    if !enabled {
        return Vec::new();
    }
    slots
        .iter()
        .filter(|s| !s.checking)
        .filter(|s| matches!(s.last_tested, Some(last) if now - last >= interval))
        .map(|s| s.name)
        .collect()
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

/// Comme [`check_slot`], avec l'empreinte du certificat généré par Raftup (ADR 0043) :
/// le test « prêt ? » d'une partie chiffrée par Raftup passe en `wss` épinglé.
pub async fn check_slot_pinned(
    address: &str,
    slot: &str,
    game: &str,
    password: Option<&str>,
    pinned: Option<client_core::Fingerprint>,
) -> Readiness {
    match check_connection_pinned(address, slot, password, game, pinned, READY_TIMEOUT).await {
        Ok(()) => Readiness::Ready,
        Err(problem) => Readiness::from_problem(problem),
    }
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

    fn timing(name: &str, checking: bool, last: Option<f64>) -> SlotTiming<'_> {
        SlotTiming {
            name,
            checking,
            last_tested: last,
        }
    }

    #[test]
    fn cadence_attend_l_intervalle() {
        let slots = [timing("A", false, Some(100.0))];
        assert!(slots_due(true, 159.9, 60.0, &slots).is_empty());
        assert_eq!(slots_due(true, 160.0, 60.0, &slots), vec!["A"]);
    }

    #[test]
    fn cadence_jamais_deux_tests_en_meme_temps() {
        let slots = [timing("A", true, Some(0.0)), timing("B", false, Some(0.0))];
        assert_eq!(slots_due(true, 500.0, 60.0, &slots), vec!["B"]);
    }

    #[test]
    fn cadence_inactive_ou_jamais_testee() {
        let slots = [timing("A", false, Some(0.0)), timing("B", false, None)];
        assert!(slots_due(false, 500.0, 60.0, &slots).is_empty());
        // B n'a jamais été testé : le premier test reste manuel.
        assert_eq!(slots_due(true, 500.0, 60.0, &slots), vec!["A"]);
    }

    #[test]
    fn cadence_horloge_qui_recule() {
        let slots = [timing("A", false, Some(1000.0))];
        assert!(slots_due(true, 10.0, 60.0, &slots).is_empty());
    }

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
