//! État de la partie qui évolue : lieux visités et objets reçus.
//!
//! Aucun réseau ici : la tâche salle s'en sert, puis envoie les messages.
//! Les règles reprennent celles du serveur officiel (`MultiServer.py` 0.6.8) :
//! `register_location_checks`, `send_items_to`, `get_received_items`.

use std::collections::{BTreeMap, BTreeSet};

use ap_protocol::{
    ClientStatus, Hint, HintStatus, LocationId, NetworkItem, SlotId, SlotType, TeamId,
};
use multidata::MultiData;

/// Un objet trouvé dans un lieu, et le slot qui le reçoit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sent {
    /// L'objet ; `player` est le slot qui l'a trouvé.
    pub item: NetworkItem,
    /// Le slot qui le reçoit (peut être un groupe).
    pub receiver: SlotId,
}

/// Ce qui change pendant la partie.
#[derive(Debug, Default)]
pub(crate) struct GameState {
    /// Lieux visités, par (équipe, slot).
    location_checks: BTreeMap<(TeamId, SlotId), BTreeSet<LocationId>>,
    /// Objets reçus, par (équipe, slot, `remote_items`).
    ///
    /// Comme sur le serveur officiel, il y a deux listes par slot :
    /// - `true` : tous les objets, y compris ceux trouvés dans son propre monde ;
    /// - `false` : seulement ceux trouvés par les autres.
    received_items: BTreeMap<(TeamId, SlotId, bool), Vec<NetworkItem>>,
    /// Statut de chaque (équipe, slot) déjà vu : connecté, prêt, objectif atteint…
    /// Absent = jamais connecté (le serveur officiel distingue ce cas).
    client_status: BTreeMap<(TeamId, SlotId), ClientStatus>,
    /// Pour chaque groupe, les membres qui ont déjà fait `collect`.
    group_collected: BTreeMap<SlotId, BTreeSet<SlotId>>,
    /// Indices connus de chaque (équipe, slot). Un ensemble en Python : pas de
    /// doublon, mais l'ordre ne compte pas.
    hints: BTreeMap<(TeamId, SlotId), Vec<Hint>>,
}

impl GameState {
    /// État de départ : les slots qui ne sont pas des joueurs (groupes,
    /// spectateurs) ont atteint leur objectif d'office.
    pub fn new(data: &MultiData) -> GameState {
        let mut game = GameState::default();
        for (slot, info) in &data.slots {
            if info.slot_type != SlotType::PLAYER {
                game.client_status.insert((0, *slot), ClientStatus::GOAL);
            }
        }
        for (slot, hints) in &data.precollected_hints {
            let list = game.hints.entry((0, *slot)).or_default();
            for hint in hints {
                if !list.contains(hint) {
                    list.push(hint.clone());
                }
            }
        }
        game
    }

    /// Statut d'un slot ; `None` s'il ne s'est jamais connecté.
    pub fn status(&self, team: TeamId, slot: SlotId) -> Option<ClientStatus> {
        self.client_status.get(&(team, slot)).copied()
    }

    pub fn set_status(&mut self, team: TeamId, slot: SlotId, status: ClientStatus) {
        self.client_status.insert((team, slot), status);
    }

    /// Note qu'un membre de groupe a fait `collect` ; vrai si tous l'ont fait.
    pub fn mark_group_collected(
        &mut self,
        data: &MultiData,
        group: SlotId,
        member: SlotId,
    ) -> bool {
        let collected = self.group_collected.entry(group).or_default();
        collected.insert(member);
        let members: BTreeSet<SlotId> = slot_set(data, group).into_iter().collect();
        *collected == members
    }

    /// Indices d'un slot, tels quels (sans les relire).
    pub fn hints(&self, team: TeamId, slot: SlotId) -> &[Hint] {
        match self.hints.get(&(team, slot)) {
            Some(hints) => hints,
            None => &[],
        }
    }

    /// Relit les indices d'un slot : ceux dont le lieu a été visité passent à
    /// « trouvé » (`recheck_hints` du serveur officiel). Chaque indice changé
    /// est aussi remplacé chez les autres slots concernés.
    ///
    /// Renvoie les (équipe, slot) dont un indice a changé.
    pub fn recheck_hints(
        &mut self,
        data: &MultiData,
        team: TeamId,
        slot: SlotId,
    ) -> BTreeSet<(TeamId, SlotId)> {
        let mut changed = BTreeSet::new();
        let Some(hints) = self.hints.get(&(team, slot)).cloned() else {
            return changed;
        };
        let mut new_hints: Vec<Hint> = Vec::new();
        for hint in hints {
            let new_hint = self.re_check(team, &hint);
            if !new_hints.contains(&new_hint) {
                new_hints.push(new_hint.clone());
            }
            if new_hint == hint {
                continue;
            }
            let mut concerned = slot_set(data, hint.receiving_player);
            concerned.push(hint.finding_player);
            for player in concerned {
                changed.insert((team, player));
                if player != slot {
                    self.replace_hint(team, player, &hint, &new_hint);
                }
            }
        }
        self.hints.insert((team, slot), new_hints);
        changed
    }

    /// Un indice dont le lieu a été visité devient « trouvé » (`Hint.re_check`).
    fn re_check(&self, team: TeamId, hint: &Hint) -> Hint {
        if hint.found && hint.status == HintStatus::FOUND {
            return hint.clone();
        }
        let checks = self.location_checks.get(&(team, hint.finding_player));
        if checks.is_some_and(|c| c.contains(&hint.location)) {
            let mut found = hint.clone();
            found.found = true;
            found.status = HintStatus::FOUND;
            return found;
        }
        hint.clone()
    }

    /// Remplace un indice, seulement s'il est présent chez ce slot.
    fn replace_hint(&mut self, team: TeamId, slot: SlotId, old: &Hint, new: &Hint) {
        if let Some(hints) = self.hints.get_mut(&(team, slot)) {
            if let Some(position) = hints.iter().position(|h| h == old) {
                hints.remove(position);
                if !hints.contains(new) {
                    hints.push(new.clone());
                }
            }
        }
    }

    /// Lieux visités par un slot, dans l'ordre croissant.
    pub fn checked(&self, team: TeamId, slot: SlotId) -> Vec<LocationId> {
        match self.location_checks.get(&(team, slot)) {
            Some(checks) => checks.iter().copied().collect(),
            None => Vec::new(),
        }
    }

    /// Nombre de lieux visités par un slot.
    pub fn checked_count(&self, team: TeamId, slot: SlotId) -> usize {
        self.location_checks
            .get(&(team, slot))
            .map_or(0, |c| c.len())
    }

    /// Lieux pas encore visités, dans l'ordre croissant.
    /// `None` si le slot n'a pas de lieux du tout (un groupe).
    pub fn missing(&self, data: &MultiData, team: TeamId, slot: SlotId) -> Option<Vec<LocationId>> {
        let locations = data.locations.get(&slot)?;
        let checks = self.location_checks.get(&(team, slot));
        let is_checked = |l: &LocationId| checks.is_some_and(|c| c.contains(l));
        Some(
            locations
                .keys()
                .copied()
                .filter(|l| !is_checked(l))
                .collect(),
        )
    }

    /// Objets reçus par un slot (sans l'inventaire de départ).
    pub fn received(&self, team: TeamId, slot: SlotId, remote_items: bool) -> &[NetworkItem] {
        match self.received_items.get(&(team, slot, remote_items)) {
            Some(items) => items,
            None => &[],
        }
    }

    /// Enregistre des lieux visités et distribue leurs objets.
    ///
    /// Renvoie les nouveaux lieux (triés) et les objets envoyés, dans l'ordre du
    /// serveur officiel : triés par destinataire, puis objet, puis lieu.
    /// Les lieux inconnus ou déjà visités sont ignorés.
    pub fn register_checks(
        &mut self,
        data: &MultiData,
        team: TeamId,
        slot: SlotId,
        locations: &[LocationId],
    ) -> (Vec<LocationId>, Vec<Sent>) {
        let Some(slot_locations) = data.locations.get(&slot) else {
            return (Vec::new(), Vec::new());
        };
        let checks = self.location_checks.entry((team, slot)).or_default();
        let new_locations: BTreeSet<LocationId> = locations
            .iter()
            .copied()
            .filter(|l| slot_locations.contains_key(l) && !checks.contains(l))
            .collect();
        checks.extend(new_locations.iter().copied());

        // (destinataire, objet, lieu, drapeaux) : l'ordre de tri du serveur officiel.
        let mut sortable: Vec<_> = new_locations
            .iter()
            .filter_map(|l| {
                slot_locations
                    .get(l)
                    .map(|p| (p.player, p.item, *l, p.flags))
            })
            .collect();
        sortable
            .sort_by_key(|&(receiver, item, location, flags)| (receiver, item, location, flags.0));

        let mut sent = Vec::new();
        for (receiver, item, location, flags) in sortable {
            let item = NetworkItem {
                item,
                location,
                player: slot,
                flags,
            };
            self.send_items_to(data, team, receiver, item);
            sent.push(Sent { item, receiver });
        }
        (new_locations.into_iter().collect(), sent)
    }

    /// Ajoute un objet aux listes du destinataire (de chaque membre, pour un groupe).
    fn send_items_to(
        &mut self,
        data: &MultiData,
        team: TeamId,
        receiver: SlotId,
        item: NetworkItem,
    ) {
        for target in slot_set(data, receiver) {
            if item.player != receiver {
                self.received_items
                    .entry((team, target, false))
                    .or_default()
                    .push(item);
            }
            self.received_items
                .entry((team, target, true))
                .or_default()
                .push(item);
        }
    }
}

/// Les slots concernés par un slot : les membres pour un groupe, lui-même sinon.
pub(crate) fn slot_set(data: &MultiData, slot: SlotId) -> Vec<SlotId> {
    match data.slots.get(&slot) {
        Some(info) if info.slot_type == SlotType::GROUP => info.group_members.clone(),
        _ => vec![slot],
    }
}

/// Lieux qui contiennent un objet pour ce slot, groupés par monde d'origine,
/// dans l'ordre croissant (`get_for_player` du serveur officiel).
pub(crate) fn locations_for_player(
    data: &MultiData,
    slot: SlotId,
) -> BTreeMap<SlotId, Vec<LocationId>> {
    let mut found: BTreeMap<SlotId, Vec<LocationId>> = BTreeMap::new();
    for (source, locations) in &data.locations {
        for (location, placed) in locations {
            if placed.player == slot {
                found.entry(*source).or_default().push(*location);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use ap_protocol::{ItemFlags, NetworkSlot};
    use multidata::PlacedItem;

    /// Partie minuscule : Alice (1) et Bob (2), groupe 3 = {1, 2}.
    fn tiny() -> MultiData {
        let mut data = test_data();
        let slot = |name: &str, kind, members: Vec<SlotId>| NetworkSlot {
            name: name.to_string(),
            game: "Jeu".to_string(),
            slot_type: kind,
            group_members: members,
        };
        data.slots
            .insert(1, slot("Alice", SlotType::PLAYER, vec![]));
        data.slots.insert(2, slot("Bob", SlotType::PLAYER, vec![]));
        data.slots
            .insert(3, slot("Groupe", SlotType::GROUP, vec![1, 2]));
        let placed = |item, player| PlacedItem {
            item,
            player,
            flags: ItemFlags(0),
        };
        // Lieux d'Alice : 10 → objet pour Bob, 11 → pour elle, 12 → pour le groupe.
        let alice = data.locations.entry(1).or_default();
        alice.insert(10, placed(100, 2));
        alice.insert(11, placed(101, 1));
        alice.insert(12, placed(102, 3));
        data.locations
            .entry(2)
            .or_default()
            .insert(20, placed(200, 1));
        data
    }

    fn test_data() -> MultiData {
        MultiData {
            seed_name: String::new(),
            generator_version: crate::room::SERVER_VERSION,
            minimum_server_version: crate::room::SERVER_VERSION,
            minimum_client_versions: BTreeMap::new(),
            tags: Vec::new(),
            race_mode: false,
            slots: BTreeMap::new(),
            connect_names: BTreeMap::new(),
            slot_data: BTreeMap::new(),
            locations: BTreeMap::new(),
            precollected_items: BTreeMap::new(),
            precollected_hints: BTreeMap::new(),
            er_hint_data: BTreeMap::new(),
            spheres: Vec::new(),
            server_options: Default::default(),
            datapackage: Default::default(),
        }
    }

    #[test]
    fn lieux_et_objets() {
        let data = tiny();
        let mut game = GameState::default();
        // 99 n'existe pas, 10 est en double : ignorés.
        let (new, sent) = game.register_checks(&data, 0, 1, &[12, 10, 99, 10, 11]);
        assert_eq!(new, [10, 11, 12]);
        // Trié par destinataire : Alice (1), Bob (2), groupe (3).
        let receivers: Vec<_> = sent.iter().map(|s| s.receiver).collect();
        assert_eq!(receivers, [1, 2, 3]);

        assert_eq!(game.checked(0, 1), [10, 11, 12]);
        assert_eq!(game.missing(&data, 0, 1), Some(vec![]));
        assert_eq!(game.missing(&data, 0, 2), Some(vec![20]));
        assert_eq!(game.missing(&data, 0, 3), None);

        // Alice : son propre objet (101) seulement dans la liste « tout » ;
        // l'objet du groupe (102) dans les deux, car trouvé par elle pour le groupe.
        let items = |slot, remote| -> Vec<_> {
            game.received(0, slot, remote)
                .iter()
                .map(|i| i.item)
                .collect()
        };
        assert_eq!(items(1, true), [101, 102]);
        assert_eq!(items(1, false), [102]);
        assert_eq!(items(2, true), [100, 102]);
        assert_eq!(items(2, false), [100, 102]);

        // Deuxième passage : rien de nouveau.
        let (new, sent) = game.register_checks(&data, 0, 1, &[10]);
        assert!(new.is_empty() && sent.is_empty());
    }
}
