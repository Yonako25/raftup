//! Ce que fait le joueur en ce moment, pour sa présence sur Rafthub (ADR 0035).
//!
//! - [`RunningGames`] : les jeux lancés depuis le launcher qui tournent encore ;
//! - [`presence_for`] : la présence à envoyer, « en partie · <jeu> » ou « en ligne ».
//!
//! La présence ne porte **que le nom du jeu** : jamais l'adresse du serveur, le nom du
//! slot ni le mot de passe. C'est pourquoi [`presence_for`] ne reçoit que des noms de
//! jeux, et rien d'autre de la partie.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::hub_client::Presence;
use crate::seed::SeedSummary;

/// Jeux lancés depuis le launcher qui tournent encore.
///
/// `Clone` est bon marché : toutes les copies partagent la même liste (`Arc`). Le
/// thread qui attend la fin du jeu en garde une, l'interface une autre.
#[derive(Debug, Clone, Default)]
pub struct RunningGames {
    // `Arc<Mutex<…>>` : la liste est partagée entre plusieurs threads ; le `Mutex`
    // garantit qu'un seul y touche à la fois.
    names: Arc<Mutex<Vec<String>>>,
}

impl RunningGames {
    pub fn new() -> RunningGames {
        RunningGames::default()
    }

    /// Note que `game` tourne, jusqu'à ce que le [`PlayingGuard`] rendu soit lâché
    /// (fin du processus du jeu). Deux lancements du même jeu comptent deux fois.
    pub fn start(&self, game: impl Into<String>) -> PlayingGuard {
        let game = game.into();
        lock(&self.names).push(game.clone());
        PlayingGuard {
            names: Arc::clone(&self.names),
            game,
        }
    }

    /// Les jeux en cours, du plus ancien lancement au plus récent.
    pub fn names(&self) -> Vec<String> {
        lock(&self.names).clone()
    }
}

/// Tant qu'il existe, le jeu compte comme lancé. Le lâcher (fin de portée, `drop`)
/// le retire de la liste, même si le thread qui le tient s'arrête sur une erreur.
#[derive(Debug)]
pub struct PlayingGuard {
    names: Arc<Mutex<Vec<String>>>,
    game: String,
}

impl Drop for PlayingGuard {
    fn drop(&mut self) {
        let mut names = lock(&self.names);
        // On retire une seule occurrence : un deuxième lancement du jeu reste compté.
        if let Some(index) = names.iter().position(|name| name == &self.game) {
            names.remove(index);
        }
    }
}

/// Verrouille la liste. Un thread qui a paniqué en la tenant ne la rend pas
/// inutilisable : `into_inner` reprend la liste telle qu'elle est (pas d'`unwrap`).
fn lock(names: &Mutex<Vec<String>>) -> MutexGuard<'_, Vec<String>> {
    names
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// La présence à envoyer au hub.
///
/// - un jeu lancé depuis le launcher : « en partie » avec ce jeu (le plus récent) ;
/// - sinon, une partie hébergée par ce PC : « en partie » avec les jeux de la seed
///   (groupes exclus, sans doublon, par ordre alphabétique : « APQuest, Slime Rancher ») ;
/// - sinon : « en ligne ».
///
/// Le nom est nettoyé et coupé à 64 caractères par [`Presence::playing`].
pub fn presence_for(launched: &[String], hosted: Option<&SeedSummary>) -> Presence {
    if let Some(game) = launched.last() {
        return Presence::playing(game);
    }
    let Some(seed) = hosted else {
        return Presence::online();
    };
    // `BTreeSet` : sans doublon et trié, donc le même texte à chaque fois.
    let games: BTreeSet<&str> = seed
        .slots
        .iter()
        .filter(|slot| !slot.is_group)
        .map(|slot| slot.game.as_str())
        .collect();
    let text = games.into_iter().collect::<Vec<_>>().join(", ");
    Presence::playing(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub_client::Status;
    use crate::seed::SeedSlot;

    fn seed(slots: &[(&str, &str, bool)]) -> SeedSummary {
        SeedSummary {
            seed_name: "s".into(),
            generator_version: "0.6.8".into(),
            slots: slots
                .iter()
                .enumerate()
                .map(|(i, (name, game, is_group))| SeedSlot {
                    slot: i as u32 + 1,
                    name: name.to_string(),
                    game: game.to_string(),
                    is_group: *is_group,
                    members: Vec::new(),
                    locations: 0,
                })
                .collect(),
            players: slots.len(),
            has_password: true,
            release_mode: "auto".into(),
            collect_mode: "auto".into(),
            hint_cost: 10,
        }
    }

    #[test]
    fn rien_en_cours_en_ligne() {
        assert_eq!(presence_for(&[], None), Presence::online());
    }

    #[test]
    fn jeu_lance_d_abord() {
        let hosted = seed(&[("Mimi", "APQuest", false)]);
        let launched = vec!["Slime Rancher".to_string()];
        assert_eq!(
            presence_for(&launched, Some(&hosted)),
            Presence::in_game("Slime Rancher")
        );
    }

    #[test]
    fn partie_hebergee_sans_slot_ni_adresse() {
        let hosted = seed(&[
            ("Mimi", "Slime Rancher", false),
            ("Bob", "APQuest", false),
            ("Zoé", "APQuest", false),
            ("Groupe", "Groupe secret", true),
        ]);
        let presence = presence_for(&[], Some(&hosted));
        assert_eq!(presence.status, Status::InGame);
        assert_eq!(presence.game.as_deref(), Some("APQuest, Slime Rancher"));
        // Ni le nom d'un slot, ni celui d'un groupe.
        let json = serde_json::to_string(&presence).unwrap();
        for secret in ["Mimi", "Bob", "Zoé", "Groupe"] {
            assert!(!json.contains(secret), "{json}");
        }
    }

    #[test]
    fn beaucoup_de_jeux_coupes_a_64() {
        let names: Vec<String> = (0..20).map(|i| format!("Jeu numéro {i:02}")).collect();
        let slots: Vec<(&str, &str, bool)> =
            names.iter().map(|n| ("x", n.as_str(), false)).collect();
        let presence = presence_for(&[], Some(&seed(&slots)));
        let game = presence.game.unwrap();
        assert!(game.chars().count() <= crate::hub_client::MAX_GAME_CHARS);
        assert!(game.ends_with('…'));
    }

    #[test]
    fn jeu_retire_a_la_fin_du_processus() {
        let games = RunningGames::new();
        let first = games.start("APQuest");
        let second = games.start("APQuest");
        let other = games.clone().start("Slime Rancher");
        assert_eq!(games.names(), ["APQuest", "APQuest", "Slime Rancher"]);
        drop(other);
        drop(first);
        // Le deuxième lancement d'APQuest tourne encore.
        assert_eq!(games.names(), ["APQuest"]);
        drop(second);
        assert!(games.names().is_empty());
    }

    #[test]
    fn jeu_retire_meme_si_le_thread_panique() {
        let games = RunningGames::new();
        let inner = games.clone();
        let result = std::thread::spawn(move || {
            let _guard = inner.start("APQuest");
            panic!("le thread s'arrête");
        })
        .join();
        assert!(result.is_err());
        assert!(games.names().is_empty());
    }
}
