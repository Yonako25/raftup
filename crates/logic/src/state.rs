//! Inventaire : combien d'exemplaires de chaque sorte d'objet on possède.

use crate::ids::ItemKindId;
use crate::world::MultiWorld;

/// Inventaire de tous les joueurs à la fois, indexé par [`ItemKindId`].
///
/// Une sorte d'objet appartient à un seul joueur, donc un compteur par sorte suffit à
/// distinguer « l'épée du joueur 1 » de « l'épée du joueur 2 ».
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct State {
    counts: Vec<u32>,
}

impl State {
    /// Inventaire vide, dimensionné pour le monde.
    pub fn new(world: &MultiWorld) -> State {
        State {
            counts: vec![0; world.item_kinds().len()],
        }
    }

    /// Nombre d'exemplaires possédés (0 pour une sorte inconnue).
    pub fn count(&self, item: ItemKindId) -> u32 {
        self.counts.get(item.index()).copied().unwrap_or(0)
    }

    /// Ramasse un objet.
    ///
    /// Comme dans Archipelago, seuls les objets de **progression** comptent : les autres
    /// ne débloquent rien, on les ignore. Renvoie `true` si le compteur a changé.
    pub fn collect(&mut self, world: &MultiWorld, item: ItemKindId) -> bool {
        let progression = world
            .item_kind(item)
            .is_some_and(|kind| kind.classification.progression);
        if !progression {
            return false;
        }
        self.add(item, 1);
        true
    }

    /// Repose un objet ramassé par [`State::collect`] (utile au placement « supposé »).
    /// Renvoie `true` si le compteur a changé.
    pub fn remove(&mut self, world: &MultiWorld, item: ItemKindId) -> bool {
        let progression = world
            .item_kind(item)
            .is_some_and(|kind| kind.classification.progression);
        match self.counts.get_mut(item.index()) {
            Some(count) if progression && *count > 0 => {
                *count -= 1;
                true
            }
            _ => false,
        }
    }

    /// Ajoute `n` exemplaires sans regarder la classification (tests, inventaire de départ
    /// déjà filtré).
    pub fn add(&mut self, item: ItemKindId, n: u32) {
        let index = item.index();
        // Le monde a pu recevoir de nouvelles sortes d'objets après la création de
        // l'inventaire : on agrandit le tableau au lieu de paniquer.
        if index >= self.counts.len() {
            self.counts.resize(index + 1, 0);
        }
        if let Some(count) = self.counts.get_mut(index) {
            *count = count.saturating_add(n);
        }
    }
}
