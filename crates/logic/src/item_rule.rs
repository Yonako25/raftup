//! Règles d'objet d'un lieu et accessibilité d'un joueur (ADR 0029).
//!
//! Une **règle d'objet** dit quels objets un lieu a le droit de recevoir, comme
//! `item_rule` et `always_allow` dans Archipelago (c'est aussi ainsi qu'Archipelago
//! applique `local_items` et `non_local_items`). Ici, la règle n'est pas une fonction mais
//! une **liste** de sortes d'objets, calculée par Archipelago lui-même sur chaque objet du
//! pool : elle ne dépend donc jamais de l'inventaire.
//!
//! Beaucoup de lieux ont la même règle (« pas les objets locaux des autres joueurs ») : les
//! règles sont rangées une seule fois dans une arène du [`MultiWorld`](crate::MultiWorld)
//! et chaque lieu garde seulement l'index de la sienne ([`ItemRuleId`](crate::ItemRuleId)).

use crate::ids::ItemKindId;

/// Quels objets un lieu accepte.
///
/// Les listes sont triées et sans doublon (voir [`ItemRule::forbid`]) : on y cherche par
/// dichotomie (`binary_search`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ItemRule {
    /// Liste des objets cités par la règle.
    list: Vec<ItemKindId>,
    /// `true` : seuls les objets de `list` sont acceptés ; `false` : ils sont refusés.
    allow_only: bool,
    /// Objets toujours acceptés, même si `list` les refuse et même dans un lieu exclu
    /// (`always_allow` d'Archipelago).
    always: Vec<ItemKindId>,
}

/// Trie et retire les doublons.
fn sorted(mut items: Vec<ItemKindId>) -> Vec<ItemKindId> {
    items.sort();
    items.dedup();
    items
}

impl ItemRule {
    /// Règle qui **refuse** les objets de `forbidden` ; ceux de `always` passent toujours.
    pub fn forbid(forbidden: Vec<ItemKindId>, always: Vec<ItemKindId>) -> ItemRule {
        ItemRule {
            list: sorted(forbidden),
            allow_only: false,
            always: sorted(always),
        }
    }

    /// Règle qui **n'accepte que** les objets de `allowed` ; ceux de `always` passent
    /// toujours.
    pub fn allow_only(allowed: Vec<ItemKindId>, always: Vec<ItemKindId>) -> ItemRule {
        ItemRule {
            list: sorted(allowed),
            allow_only: true,
            always: sorted(always),
        }
    }

    /// L'objet passe par `always_allow`.
    pub fn always_allows(&self, item: ItemKindId) -> bool {
        self.always.binary_search(&item).is_ok()
    }

    /// L'objet passe la règle ordinaire (`item_rule`).
    pub fn allows(&self, item: ItemKindId) -> bool {
        let listed = self.list.binary_search(&item).is_ok();
        listed == self.allow_only
    }

    /// Objets cités par la règle (liste puis `always`), pour vérifier qu'ils existent.
    pub fn items(&self) -> impl Iterator<Item = ItemKindId> + '_ {
        self.list.iter().chain(self.always.iter()).copied()
    }
}

/// Ce que le placement doit garantir à un joueur (`accessibility` d'Archipelago).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Accessibility {
    /// Chaque lieu du joueur est atteignable (le défaut).
    #[default]
    Full,
    /// Chaque objet de progression du joueur est atteignable ; certains de ses lieux
    /// peuvent rester fermés (`items`).
    Items,
    /// Seule la victoire du joueur est garantie : ses objets peuvent rester hors
    /// d'atteinte, et ses lieux aussi s'ils ne gardent pas la progression d'un autre joueur
    /// (`minimal`).
    Minimal,
}
