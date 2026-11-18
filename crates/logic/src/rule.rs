//! Règles d'accès : un arbre fermé, évalué contre un inventaire.
//!
//! Le vocabulaire reprend celui du module `rule_builder` d'Archipelago (ADR 0027, § 2).
//! Les `OptionFilter` n'existent pas ici : ils sont résolus quand on construit le monde,
//! puisqu'on connaît alors les options du joueur.
//!
//! Il n'y a pas de « non » : une règle ne peut que devenir vraie quand l'inventaire grandit.
//! C'est ce qui permet au calcul d'atteignabilité d'avancer sans jamais revenir en arrière.

use crate::ids::{EntranceId, ItemGroupId, ItemKindId, LocationId, RegionId};
use crate::state::State;
use crate::sweep::Reachability;
use crate::world::MultiWorld;

/// Une règle d'accès (d'une entrée, d'un lieu ou de l'objectif d'un joueur).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// Toujours vraie.
    True,
    /// Toujours fausse.
    False,
    /// Au moins `count` exemplaires de l'objet.
    Has { item: ItemKindId, count: u32 },
    /// Au moins un exemplaire de chacun (vraie si la liste est vide).
    HasAll(Vec<ItemKindId>),
    /// Au moins un exemplaire de l'un d'eux (fausse si la liste est vide).
    HasAny(Vec<ItemKindId>),
    /// Au moins le nombre demandé de chacun (vraie si la liste est vide).
    HasAllCounts(Vec<(ItemKindId, u32)>),
    /// Au moins `count` objets du groupe, tous membres confondus.
    HasGroup { group: ItemGroupId, count: u32 },
    /// La région est atteignable.
    CanReachRegion(RegionId),
    /// Le lieu est atteignable.
    CanReachLocation(LocationId),
    /// L'entrée est franchissable.
    CanReachEntrance(EntranceId),
    /// Toutes les sous-règles (vraie si la liste est vide, comme `all` en Python).
    And(Vec<Rule>),
    /// Au moins une sous-règle (fausse si la liste est vide, comme `any` en Python).
    Or(Vec<Rule>),
}

impl Rule {
    /// Raccourci : au moins un exemplaire de `item`.
    pub fn has(item: ItemKindId) -> Rule {
        Rule::Has { item, count: 1 }
    }

    /// Raccourci : au moins `count` exemplaires de `item`.
    pub fn has_count(item: ItemKindId, count: u32) -> Rule {
        Rule::Has { item, count }
    }

    /// Évalue la règle.
    ///
    /// `reach` sert aux règles `CanReach…` : c'est ce qu'on sait déjà atteignable.
    /// Un identifiant inconnu (monde mal construit) rend la règle fausse au lieu de
    /// paniquer ; [`MultiWorld::validate`] sert à repérer ces erreurs.
    ///
    /// Aucune allocation : on lit seulement des compteurs et des drapeaux.
    pub fn eval(&self, world: &MultiWorld, state: &State, reach: &Reachability) -> bool {
        match self {
            Rule::True => true,
            Rule::False => false,
            Rule::Has { item, count } => state.count(*item) >= *count,
            Rule::HasAll(items) => items.iter().all(|item| state.count(*item) >= 1),
            Rule::HasAny(items) => items.iter().any(|item| state.count(*item) >= 1),
            Rule::HasAllCounts(pairs) => pairs
                .iter()
                .all(|(item, count)| state.count(*item) >= *count),
            Rule::HasGroup { group, count } => match world.item_group(*group) {
                Some(group) => {
                    // Somme des compteurs des membres ; `saturating_add` évite tout
                    // débordement (on plafonne au lieu de paniquer).
                    let total = group
                        .members
                        .iter()
                        .fold(0u32, |sum, item| sum.saturating_add(state.count(*item)));
                    total >= *count
                }
                None => false,
            },
            Rule::CanReachRegion(region) => reach.region(*region),
            Rule::CanReachLocation(location) => reach.location(*location),
            Rule::CanReachEntrance(entrance) => reach.entrance(*entrance),
            Rule::And(rules) => rules.iter().all(|rule| rule.eval(world, state, reach)),
            Rule::Or(rules) => rules.iter().any(|rule| rule.eval(world, state, reach)),
        }
    }

    /// Parcourt l'arbre et appelle `visit` sur chaque nœud (la règle elle-même comprise),
    /// en profondeur, dans l'ordre. Sert à la validation.
    ///
    /// `&mut impl FnMut` : on prête la même fonction aux appels récursifs au lieu de la
    /// déplacer, et `FnMut` lui permet de modifier ce qu'elle capture (ici, une erreur).
    pub(crate) fn walk(&self, visit: &mut impl FnMut(&Rule)) {
        visit(self);
        if let Rule::And(rules) | Rule::Or(rules) = self {
            for rule in rules {
                rule.walk(visit);
            }
        }
    }
}
