//! État de la partie qui évolue : lieux visités et objets reçus.
//!
//! Aucun réseau ici : la tâche salle s'en sert, puis envoie les messages.
//! Les règles reprennent celles du serveur officiel (`MultiServer.py` 0.6.8) :
//! `register_location_checks`, `send_items_to`, `get_received_items`.

use std::collections::{BTreeMap, BTreeSet};

use ap_protocol::{
    ClientStatus, Hint, HintStatus, ItemFlags, ItemId, LocationId, NetworkItem, SlotId, SlotType,
    TeamId,
};
use multidata::{MultiData, PlacedItem};

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
    /// Nombre d'indices payés par (équipe, slot) (`hints_used` du serveur officiel).
    hints_used: BTreeMap<(TeamId, SlotId), u32>,
}

/// Ce que `notify_hints` demande d'envoyer, une fois l'état mis à jour.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct HintEffects {
    /// Slots dont la liste d'indices a reçu un nouvel indice : pour chacun, un
    /// `SetReply` de `_read_hints_<équipe>_<slot>` puis un `RoomUpdate` `hint_points`.
    pub new_hint_slots: BTreeSet<(TeamId, SlotId)>,
    /// Messages `PrintJSON` de type `Hint`, par slot concerné (ordre croissant).
    /// Les indices de ce slot (où il est celui qui trouve) viennent en premier.
    pub messages: Vec<(SlotId, Vec<Hint>)>,
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
}

// Moteur d'indices. Les méthodes encore inutilisées (pour `!hint`, la
// sauvegarde) portent chacune un `allow(dead_code)`.
impl GameState {
    /// Nombre d'indices déjà payés par un slot.
    pub fn hints_used(&self, team: TeamId, slot: SlotId) -> u32 {
        self.hints_used.get(&(team, slot)).copied().unwrap_or(0)
    }

    #[allow(dead_code)] // pour `!hint` / la sauvegarde, pas encore branché
    /// Fixe le nombre d'indices payés (chargement d'une sauvegarde).
    pub fn set_hints_used(&mut self, team: TeamId, slot: SlotId, used: u32) {
        self.hints_used.insert((team, slot), used);
    }

    #[allow(dead_code)] // pour `!hint` / la sauvegarde, pas encore branché
    /// Ajoute `count` indices payés ; renvoie le nouveau total.
    pub fn add_hints_used(&mut self, team: TeamId, slot: SlotId, count: u32) -> u32 {
        let used = self.hints_used.entry((team, slot)).or_insert(0);
        *used = used.saturating_add(count);
        *used
    }

    /// Points d'indice d'un slot (`get_slot_points`) : points des lieux visités
    /// moins le coût des indices déjà payés.
    pub fn slot_points(
        &self,
        data: &MultiData,
        team: TeamId,
        slot: SlotId,
        location_check_points: i64,
    ) -> i64 {
        let checked = self.checked_count(team, slot) as i64;
        let cost = i64::from(hint_cost(data, slot));
        let used = i64::from(self.hints_used(team, slot));
        location_check_points * checked - cost * used
    }

    /// L'indice déjà connu pour ce lieu de ce slot (`get_hint`).
    pub fn get_hint(&self, team: TeamId, finding: SlotId, location: LocationId) -> Option<&Hint> {
        self.hints(team, finding)
            .iter()
            .find(|h| h.location == location && h.finding_player == finding)
    }

    /// Statut d'un nouvel indice : « trouvé » si le lieu est visité, sinon le
    /// statut demandé, ou (`None`) un statut automatique : à éviter pour un
    /// piège, prioritaire sinon.
    fn new_hint_status(
        &self,
        team: TeamId,
        finding: SlotId,
        location: LocationId,
        flags: ItemFlags,
        status: Option<HintStatus>,
    ) -> (bool, HintStatus) {
        let found = self
            .location_checks
            .get(&(team, finding))
            .is_some_and(|c| c.contains(&location));
        let status = if found {
            HintStatus::FOUND
        } else {
            match status {
                Some(status) => status,
                None if flags.contains(ItemFlags::TRAP) => HintStatus::AVOID,
                None => HintStatus::PRIORITY,
            }
        };
        (found, status)
    }

    /// Indice pour un lieu du slot (`collect_hint_location_id`). Si l'indice
    /// existe déjà, c'est lui qui est renvoyé. Lieu inconnu : liste vide.
    /// Le serveur officiel prend `HINT_UNSPECIFIED` par défaut : à l'appelant
    /// de passer `Some(HintStatus::UNSPECIFIED)`.
    pub fn collect_hint_location_id(
        &self,
        data: &MultiData,
        team: TeamId,
        slot: SlotId,
        location: LocationId,
        status: Option<HintStatus>,
    ) -> Vec<Hint> {
        if let Some(previous) = self.get_hint(team, slot, location) {
            return vec![previous.clone()];
        }
        let Some(placed) = data.locations.get(&slot).and_then(|l| l.get(&location)) else {
            return Vec::new();
        };
        let (found, status) = self.new_hint_status(team, slot, location, placed.flags, status);
        vec![make_hint(data, slot, location, placed, found, status)]
    }

    #[allow(dead_code)] // pour `!hint` / la sauvegarde, pas encore branché
    /// Indices pour un objet (`collect_hints`) : tous les lieux, dans tous les
    /// mondes, qui contiennent cet objet pour le slot ou pour un groupe dont il
    /// fait partie. Ordre : monde croissant, puis lieu croissant (le serveur
    /// officiel suit l'ordre d'insertion du multidata : à vérifier).
    pub fn collect_hints(
        &self,
        data: &MultiData,
        team: TeamId,
        slot: SlotId,
        item: ItemId,
        status: Option<HintStatus>,
    ) -> Vec<Hint> {
        let mut receivers = BTreeSet::from([slot]);
        for (group_id, info) in &data.slots {
            if info.slot_type == SlotType::GROUP && info.group_members.contains(&slot) {
                receivers.insert(*group_id);
            }
        }
        let mut hints = Vec::new();
        for (finding, locations) in &data.locations {
            for (location, placed) in locations {
                if placed.item != item || !receivers.contains(&placed.player) {
                    continue;
                }
                if let Some(previous) = self.get_hint(team, *finding, *location) {
                    hints.push(previous.clone());
                    continue;
                }
                let (found, status) =
                    self.new_hint_status(team, *finding, *location, placed.flags, status);
                hints.push(make_hint(data, *finding, *location, placed, found, status));
            }
        }
        hints
    }

    /// Mémorise et diffuse des indices (`notify_hints`). Modifie l'état et
    /// renvoie ce que la tâche salle doit envoyer.
    ///
    /// - `only_new` : ignore les indices que le slot qui trouve connaît déjà ;
    /// - `persist_even_if_found` : mémorise aussi les indices déjà trouvés
    ///   (`LocationScouts`) ; sinon ils sont seulement affichés ;
    /// - `recipients` : si présent, seuls ces slots reçoivent le message.
    pub fn notify_hints(
        &mut self,
        data: &MultiData,
        team: TeamId,
        hints: &[Hint],
        only_new: bool,
        persist_even_if_found: bool,
        recipients: Option<&[SlotId]>,
    ) -> HintEffects {
        let mut effects = HintEffects::default();
        // On manipule des références (`&Hint`) : pas de copie, et elles restent
        // valables tant que `hints` (emprunté par l'appelant) existe.
        let mut hints: Vec<&Hint> = hints.iter().collect();
        if only_new {
            hints.retain(|h| !self.hints(team, h.finding_player).contains(h));
        }
        if hints.is_empty() {
            return effects;
        }
        // Tri stable, « trouvés » d'abord (comme `sorted(..., reverse=True)` de
        // Python, qui garde l'ordre d'origine des égalités).
        hints.sort_by_key(|h| std::cmp::Reverse(h.found));

        let mut concerns: BTreeMap<SlotId, Vec<&Hint>> = BTreeMap::new();
        for hint in hints {
            for player in slot_set(data, hint.receiving_player) {
                concerns.entry(player).or_default().push(hint);
            }
            let local = hint.receiving_player == hint.finding_player;
            let list = concerns.entry(hint.finding_player).or_default();
            if !local && !list.contains(&hint) {
                list.push(hint);
            }
            if (!hint.found || persist_even_if_found)
                && !self.hints(team, hint.finding_player).contains(hint)
            {
                self.add_hint(team, hint.finding_player, hint);
                effects.new_hint_slots.insert((team, hint.finding_player));
                for player in slot_set(data, hint.receiving_player) {
                    self.add_hint(team, player, hint);
                    effects.new_hint_slots.insert((team, player));
                }
            }
        }
        for (slot, list) in concerns {
            if recipients.is_some_and(|r| !r.contains(&slot)) {
                continue;
            }
            let mut list: Vec<Hint> = list.into_iter().cloned().collect();
            // Tri stable : les indices que ce slot a trouvés d'abord.
            list.sort_by_key(|h| h.finding_player != slot);
            effects.messages.push((slot, list));
        }
        effects
    }

    /// Ajoute un indice à un slot, sans doublon (un ensemble en Python).
    fn add_hint(&mut self, team: TeamId, slot: SlotId, hint: &Hint) {
        let list = self.hints.entry((team, slot)).or_default();
        if !list.contains(hint) {
            list.push(hint.clone());
        }
    }

    /// Change le statut d'un indice (`UpdateHint`) chez tous les slots concernés.
    /// Les contrôles du paquet (droits, statut valide) restent à l'appelant.
    ///
    /// Renvoie les (équipe, slot) dont la clé `_read_hints` change ; vide si
    /// l'indice est inconnu ou inchangé.
    pub fn update_hint(
        &mut self,
        data: &MultiData,
        team: TeamId,
        finding: SlotId,
        location: LocationId,
        status: HintStatus,
    ) -> BTreeSet<(TeamId, SlotId)> {
        let mut changed = BTreeSet::new();
        let Some(hint) = self.get_hint(team, finding, location).cloned() else {
            return changed;
        };
        let new_hint = re_prioritize(&hint, status);
        if new_hint == hint {
            return changed;
        }
        let mut concerned = slot_set(data, hint.receiving_player);
        concerned.push(hint.finding_player);
        for player in concerned {
            self.replace_hint(team, player, &hint, &new_hint);
            changed.insert((team, player));
        }
        changed
    }

    #[allow(dead_code)] // pour `!getitem`, pas encore branché
    /// Objet donné par `!getitem` : ajouté aux deux listes du slot.
    pub fn add_cheat_item(&mut self, team: TeamId, slot: SlotId, item: NetworkItem) {
        for remote_items in [false, true] {
            self.received_items
                .entry((team, slot, remote_items))
                .or_default()
                .push(item);
        }
    }
}

impl GameState {
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

/// Coût d'un indice pour un slot (`get_hint_cost`) : un pourcentage du nombre
/// de ses lieux, au moins 1 ; 0 si les indices sont gratuits.
pub(crate) fn hint_cost(data: &MultiData, slot: SlotId) -> u32 {
    let percent = data.server_options.hint_cost;
    if percent == 0 {
        return 0;
    }
    let count = data.locations.get(&slot).map_or(0, |l| l.len());
    // Même ordre de calcul que Python (flottants), puis troncature.
    let cost = (percent as f64 * 0.01 * count as f64) as i64;
    u32::try_from(cost.max(1)).unwrap_or(u32::MAX)
}

/// `Hint.re_prioritize` : un indice trouvé reste « trouvé ».
pub(crate) fn re_prioritize(hint: &Hint, status: HintStatus) -> Hint {
    let status = if hint.found && status != HintStatus::FOUND {
        HintStatus::FOUND
    } else {
        status
    };
    let mut new_hint = hint.clone();
    new_hint.status = status;
    new_hint
}

/// Construit un indice pour un lieu du slot `finding`.
fn make_hint(
    data: &MultiData,
    finding: SlotId,
    location: LocationId,
    placed: &PlacedItem,
    found: bool,
    status: HintStatus,
) -> Hint {
    let entrance = data
        .er_hint_data
        .get(&finding)
        .and_then(|e| e.get(&location))
        .cloned()
        .unwrap_or_default();
    Hint {
        receiving_player: placed.player,
        finding_player: finding,
        location,
        item: placed.item,
        found,
        entrance,
        item_flags: placed.flags,
        status,
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
    use ap_protocol::NetworkSlot;

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

    fn hint_for(receiving: SlotId, finding: SlotId, location: LocationId, found: bool) -> Hint {
        Hint {
            receiving_player: receiving,
            finding_player: finding,
            location,
            item: 1,
            found,
            entrance: String::new(),
            item_flags: ItemFlags(0),
            status: if found {
                HintStatus::FOUND
            } else {
                HintStatus::UNSPECIFIED
            },
        }
    }

    #[test]
    fn cout_des_indices() {
        let mut data = tiny(); // Alice a 3 lieux, Bob 1.
        data.server_options.hint_cost = 0;
        assert_eq!(hint_cost(&data, 1), 0);
        // 10 % de 3 lieux = 0,3 : minimum 1.
        data.server_options.hint_cost = 10;
        assert_eq!(hint_cost(&data, 1), 1);
        // 50 % de 3 lieux = 1,5 : tronqué à 1 ; 100 % : 3.
        data.server_options.hint_cost = 50;
        assert_eq!(hint_cost(&data, 1), 1);
        data.server_options.hint_cost = 100;
        assert_eq!(hint_cost(&data, 1), 3);
        // 67 % de 3 = 2,01 : 2.
        data.server_options.hint_cost = 67;
        assert_eq!(hint_cost(&data, 1), 2);
    }

    #[test]
    fn points_d_indice() {
        let mut data = tiny();
        data.server_options.hint_cost = 100; // 3 points par indice pour Alice
        let mut game = GameState::default();
        game.register_checks(&data, 0, 1, &[10, 11]);
        assert_eq!(game.slot_points(&data, 0, 1, 5), 10);
        assert_eq!(game.add_hints_used(0, 1, 2), 2);
        assert_eq!(game.hints_used(0, 1), 2);
        assert_eq!(game.slot_points(&data, 0, 1, 5), 4);
        // Peut devenir négatif, comme en Python.
        game.set_hints_used(0, 1, 10);
        assert_eq!(game.slot_points(&data, 0, 1, 5), -20);
    }

    #[test]
    fn creation_d_indices() {
        let mut data = tiny();
        // Lieu 10 d'Alice : un piège pour Bob.
        data.locations.get_mut(&1).unwrap().insert(
            10,
            PlacedItem {
                item: 100,
                player: 2,
                flags: ItemFlags::TRAP,
            },
        );
        let mut game = GameState::default();
        // Statut par défaut du code : UNSPECIFIED ; automatique : AVOID (piège).
        let h = game.collect_hint_location_id(&data, 0, 1, 10, Some(HintStatus::UNSPECIFIED));
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].status, HintStatus::UNSPECIFIED);
        assert!(!h[0].found);
        let h = game.collect_hint_location_id(&data, 0, 1, 10, None);
        assert_eq!(h[0].status, HintStatus::AVOID);
        // Pas un piège : PRIORITY.
        let h = game.collect_hint_location_id(&data, 0, 1, 11, None);
        assert_eq!(h[0].status, HintStatus::PRIORITY);
        // Lieu inconnu.
        assert!(game
            .collect_hint_location_id(&data, 0, 1, 99, None)
            .is_empty());
        // Lieu déjà visité : trouvé, quel que soit le statut demandé.
        game.register_checks(&data, 0, 1, &[11]);
        let h = game.collect_hint_location_id(&data, 0, 1, 11, Some(HintStatus::PRIORITY));
        assert!(h[0].found);
        assert_eq!(h[0].status, HintStatus::FOUND);
        // Un indice déjà connu est renvoyé tel quel.
        let known = hint_for(2, 1, 10, false);
        game.add_hint(0, 1, &known);
        assert_eq!(
            game.collect_hint_location_id(&data, 0, 1, 10, None),
            [known]
        );
    }

    #[test]
    fn indices_par_objet_et_groupes() {
        let data = tiny();
        let game = GameState::default();
        // Objet 102 : dans le lieu 12 d'Alice, pour le groupe 3 = {1, 2}.
        // Bob (membre du groupe) le cherche : trouvé via son groupe.
        let hints = game.collect_hints(&data, 0, 2, 102, None);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].receiving_player, 3);
        assert_eq!(hints[0].finding_player, 1);
        assert_eq!(hints[0].status, HintStatus::PRIORITY);
        // Objet 200 (lieu 20 de Bob, pour Alice) : Bob ne le reçoit pas.
        assert!(game.collect_hints(&data, 0, 2, 200, None).is_empty());
        assert_eq!(game.collect_hints(&data, 0, 1, 200, None).len(), 1);
    }

    #[test]
    fn diffusion_d_un_indice_non_local() {
        let data = tiny();
        let mut game = GameState::default();
        // Alice (1) trouve, Bob (2) recevra.
        let hint = hint_for(2, 1, 10, false);
        let effects = game.notify_hints(&data, 0, std::slice::from_ref(&hint), false, false, None);
        assert_eq!(effects.new_hint_slots, BTreeSet::from([(0, 1), (0, 2)]));
        assert_eq!(effects.messages.len(), 2);
        assert_eq!(effects.messages[0], (1, vec![hint.clone()]));
        assert_eq!(effects.messages[1], (2, vec![hint.clone()]));
        assert_eq!(game.hints(0, 1), std::slice::from_ref(&hint));
        assert_eq!(game.hints(0, 2), std::slice::from_ref(&hint));

        // only_new : déjà connu, rien à faire du tout.
        let effects = game.notify_hints(&data, 0, std::slice::from_ref(&hint), true, false, None);
        assert_eq!(effects, HintEffects::default());
        // Sans only_new : réaffiché, mais pas de nouveau slot.
        let effects = game.notify_hints(&data, 0, std::slice::from_ref(&hint), false, false, None);
        assert!(effects.new_hint_slots.is_empty());
        assert_eq!(effects.messages.len(), 2);
        // Destinataires restreints.
        let effects = game.notify_hints(&data, 0, &[hint], false, false, Some(&[2]));
        assert_eq!(effects.messages.len(), 1);
        assert_eq!(effects.messages[0].0, 2);
    }

    #[test]
    fn diffusion_d_un_indice_local_ou_de_groupe() {
        let data = tiny();
        let mut game = GameState::default();
        // Local : Alice trouve et reçoit, un seul message, une seule copie.
        let local = hint_for(1, 1, 11, false);
        let effects = game.notify_hints(&data, 0, std::slice::from_ref(&local), false, false, None);
        assert_eq!(effects.messages, [(1, vec![local.clone()])]);
        assert_eq!(effects.new_hint_slots, BTreeSet::from([(0, 1)]));
        // Groupe 3 = {1, 2} reçoit, Alice trouve : le groupe lui-même n'a pas de
        // client, mais ses membres 1 et 2 sont prévenus ; Alice une seule fois.
        let grouped = hint_for(3, 1, 12, false);
        let effects =
            game.notify_hints(&data, 0, std::slice::from_ref(&grouped), false, false, None);
        let slots: Vec<_> = effects.messages.iter().map(|(s, _)| *s).collect();
        assert_eq!(slots, [1, 2]);
        assert_eq!(effects.messages[0].1.len(), 1);
        assert_eq!(game.hints(0, 1).len(), 2);
        assert_eq!(game.hints(0, 2), [grouped]);
    }

    #[test]
    fn indice_trouve_et_persistance() {
        let data = tiny();
        let mut game = GameState::default();
        let found = hint_for(2, 1, 10, true);
        // Trouvé : affiché mais pas mémorisé.
        let effects = game.notify_hints(&data, 0, std::slice::from_ref(&found), false, false, None);
        assert!(effects.new_hint_slots.is_empty());
        assert_eq!(effects.messages.len(), 2);
        assert!(game.hints(0, 1).is_empty());
        // Avec persist (LocationScouts) : mémorisé.
        let effects = game.notify_hints(&data, 0, std::slice::from_ref(&found), false, true, None);
        assert_eq!(effects.new_hint_slots, BTreeSet::from([(0, 1), (0, 2)]));
        assert_eq!(game.hints(0, 1), [found]);
    }

    #[test]
    fn tri_des_messages() {
        let data = tiny();
        let mut game = GameState::default();
        // Pour Bob : un indice trouvé par Alice (non trouvé), un trouvé par lui.
        let by_alice = hint_for(2, 1, 10, false);
        let by_bob = hint_for(2, 2, 20, false);
        let found = hint_for(2, 1, 11, true);
        let effects = game.notify_hints(
            &data,
            0,
            &[by_alice.clone(), by_bob.clone(), found.clone()],
            false,
            false,
            None,
        );
        // Bob : d'abord ce qu'il a trouvé lui-même, puis le reste, avec les
        // « trouvés » avant les autres (tri stable en deux passes).
        let for_bob = &effects.messages.iter().find(|(s, _)| *s == 2).unwrap().1;
        assert_eq!(for_bob, &[by_bob, found, by_alice]);
    }

    #[test]
    fn changement_de_statut() {
        let data = tiny();
        let mut game = GameState::default();
        let hint = hint_for(3, 1, 12, false);
        game.notify_hints(&data, 0, std::slice::from_ref(&hint), false, false, None);
        let changed = game.update_hint(&data, 0, 1, 12, HintStatus::AVOID);
        assert_eq!(changed, BTreeSet::from([(0, 1), (0, 2)]));
        assert_eq!(game.hints(0, 1)[0].status, HintStatus::AVOID);
        assert_eq!(game.hints(0, 2)[0].status, HintStatus::AVOID);
        // Même statut ou indice inconnu : rien ne change.
        assert!(game
            .update_hint(&data, 0, 1, 12, HintStatus::AVOID)
            .is_empty());
        assert!(game
            .update_hint(&data, 0, 1, 99, HintStatus::AVOID)
            .is_empty());
        // Un indice trouvé reste « trouvé ».
        let found = hint_for(2, 1, 10, true);
        game.notify_hints(&data, 0, std::slice::from_ref(&found), false, true, None);
        assert!(game
            .update_hint(&data, 0, 1, 10, HintStatus::PRIORITY)
            .is_empty());
    }

    #[test]
    fn objet_de_triche() {
        let mut game = GameState::default();
        let item = NetworkItem {
            item: 5,
            location: -1,
            player: 0,
            flags: ItemFlags(0),
        };
        game.add_cheat_item(0, 1, item);
        assert_eq!(game.received(0, 1, true), [item]);
        assert_eq!(game.received(0, 1, false), [item]);
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
