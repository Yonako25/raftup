//! Suivi d'une partie depuis l'extérieur du serveur (launcher, ADR 0010).
//!
//! Deux choses :
//! - une **photo de l'état** ([`ServerStatus`]), que la tâche salle fabrique à
//!   la demande : elle reste seule à lire l'état de la partie (ADR 0006) ;
//! - un **fil d'événements** ([`GameEvent`]) : quelqu'un rejoint, part, envoie
//!   un objet, écrit dans le chat, atteint son objectif.
//!
//! Les événements portent des données (noms des slots, des objets, des lieux),
//! pas des phrases : c'est à l'interface de les écrire, dans sa langue.

use std::collections::BTreeMap;

use ap_protocol::{ItemId, LocationId, SlotId, TeamId};
use multidata::MultiData;
use serde::{Deserialize, Serialize};

/// Taille du fil d'événements : un abonné trop lent perd les plus anciens.
pub(crate) const EVENT_CAPACITY: usize = 1024;

/// Photo de la partie à un instant donné.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServerStatus {
    /// Un élément par slot de la seed, dans l'ordre des numéros de slot.
    pub slots: Vec<SlotStatus>,
    /// Connexions ouvertes, connectées à un slot ou non.
    pub connections: usize,
}

/// Où en est un slot.
///
/// `Deserialize` : le launcher relit cette forme quand Rafthub la renvoie pour une
/// partie hébergée (`GET /v1/games/{id}`, ADR 0052).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotStatus {
    pub team: TeamId,
    pub slot: SlotId,
    pub name: String,
    pub game: String,
    /// Vrai pour un groupe (« item link ») : il ne joue pas lui-même.
    pub is_group: bool,
    /// Connexions ouvertes sur ce slot (un joueur et ses outils de suivi).
    pub connections: usize,
    /// Tags de la première connexion de jeu (`DeathLink`…), vide sinon.
    pub tags: Vec<String>,
    /// Lieux déjà visités.
    pub checked: usize,
    /// Lieux du slot.
    pub total: usize,
    /// Statut du protocole : 0 inconnu, 5 connecté, 10 prêt, 20 en jeu, 30 objectif.
    pub status: u32,
    /// Heure (secondes depuis 1970) du dernier paquet reçu de ce slot.
    pub last_activity: Option<f64>,
    /// Message d'absence (`!away`, ADR 0031) ; `None` si le joueur n'est pas absent.
    /// Un absent sans message a `Some("")`.
    pub away: Option<String>,
    /// Votes en cours pour relâcher le monde de ce slot (`!vote_release`).
    pub release_votes: u32,
}

/// Ce qui vient de se passer dans la partie.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameEvent {
    /// Un joueur (ou un outil) s'est connecté à son slot.
    Joined {
        time: f64,
        slot: SlotId,
        name: String,
        game: String,
        tags: Vec<String>,
    },
    /// Il est parti.
    Left {
        time: f64,
        slot: SlotId,
        name: String,
    },
    /// Un objet trouvé dans un lieu part vers un slot (lui-même compris).
    ItemSent {
        time: f64,
        finder: String,
        receiver: String,
        item: String,
        location: String,
        /// Drapeaux de l'objet : 1 progression, 2 utile, 4 piège.
        flags: u32,
    },
    /// Message de chat (déjà accepté et diffusé).
    Chat {
        time: f64,
        slot: SlotId,
        name: String,
        text: String,
    },
    /// Message de l'hôte à tous les joueurs.
    ServerChat { time: f64, text: String },
    /// Un slot a atteint son objectif.
    Goal {
        time: f64,
        slot: SlotId,
        name: String,
    },
    /// Un joueur s'est déclaré absent (`!away`, ADR 0031).
    Away {
        time: f64,
        slot: SlotId,
        name: String,
        message: String,
    },
    /// Il est revenu (`!back`, ou un nouveau lieu visité).
    Back {
        time: f64,
        slot: SlotId,
        name: String,
    },
    /// Un vote pour relâcher le monde de `target` (`!vote_release`).
    /// `passed` : la majorité est atteinte, le monde est relâché.
    ReleaseVote {
        time: f64,
        target: SlotId,
        target_name: String,
        voter_name: String,
        votes: u32,
        needed: u32,
        passed: bool,
    },
}

/// Noms des objets et des lieux, pour écrire des événements lisibles.
#[derive(Debug, Default)]
pub(crate) struct Names {
    items: BTreeMap<(String, ItemId), String>,
    locations: BTreeMap<(String, LocationId), String>,
}

impl Names {
    /// Construit les dictionnaires inverses (numéro → nom) de chaque jeu de la seed.
    pub fn new(data: &MultiData) -> Names {
        let mut names = Names::default();
        for (game, package) in &data.datapackage {
            for (name, id) in &package.item_name_to_id {
                names.items.insert((game.clone(), *id), name.clone());
            }
            for (name, id) in &package.location_name_to_id {
                names.locations.insert((game.clone(), *id), name.clone());
            }
        }
        names
    }

    /// Nom d'un objet du jeu `game`, ou son numéro s'il est inconnu.
    pub fn item(&self, game: &str, id: ItemId) -> String {
        self.items
            .get(&(game.to_string(), id))
            .cloned()
            .unwrap_or_else(|| format!("objet {id}"))
    }

    /// Nom d'un lieu du jeu `game`, ou son numéro s'il est inconnu.
    pub fn location(&self, game: &str, id: LocationId) -> String {
        self.locations
            .get(&(game.to_string(), id))
            .cloned()
            .unwrap_or_else(|| format!("lieu {id}"))
    }

    /// Nom d'un objet comme `ctx.item_names[game][id]` du serveur officiel :
    /// les noms du jeu « Archipelago » sont ajoutés à chaque jeu et passent
    /// devant (`update`), et un numéro inconnu donne `Unknown item (ID:x)`
    /// (`KeyedDefaultDict`).
    pub fn python_item(&self, game: &str, id: ItemId) -> String {
        let find = |g: &str| self.items.get(&(g.to_string(), id));
        find("Archipelago")
            .or_else(|| find(game))
            .cloned()
            .unwrap_or_else(|| format!("Unknown item (ID:{id})"))
    }

    /// Comme [`Names::python_item`], pour un lieu : `Unknown location (ID:x)`.
    pub fn python_location(&self, game: &str, id: LocationId) -> String {
        let find = |g: &str| self.locations.get(&(g.to_string(), id));
        find("Archipelago")
            .or_else(|| find(game))
            .cloned()
            .unwrap_or_else(|| format!("Unknown location (ID:{id})"))
    }
}
