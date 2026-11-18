//! Du multiworld rempli au [`MultiData`], comme `write_multidata` dans `Main.py`
//! d'Archipelago 0.6.8.

use std::collections::BTreeMap;

use ap_protocol::{ItemFlags, NetworkSlot, NetworkVersion, SlotType};
use indexmap::IndexMap;
use logic::{Classification, ItemKindId, MultiWorld, Reachability, State};
use multidata::{MultiData, PlacedItem, ServerOptions};
use serde_json::Value;

use crate::datapackage::archipelago_package;
use crate::{GenerateError, GenerateRequest};

/// Version d'Archipelago dont on reproduit les mondes (ADR 0027, § 3).
pub(crate) const GENERATOR_VERSION: NetworkVersion = version(0, 6, 8);
/// `World.required_server_version` en 0.6.8.
pub(crate) const MINIMUM_SERVER_VERSION: NetworkVersion = version(0, 5, 0);
/// `World.required_client_version` par défaut en 0.6.8 (APQuest ne le change pas).
pub(crate) const MINIMUM_CLIENT_VERSION: NetworkVersion = version(0, 1, 6);

const fn version(major: u32, minor: u32, build: u32) -> NetworkVersion {
    NetworkVersion {
        major,
        minor,
        build,
    }
}

/// Drapeaux réseau d'un objet (`ItemClassification` d'Archipelago).
fn flags(class: Classification) -> ItemFlags {
    let mut bits = 0u8;
    if class.progression {
        bits |= 0b0001;
    }
    if class.useful {
        bits |= 0b0010;
    }
    if class.trap {
        bits |= 0b0100;
    }
    if class.skip_balancing {
        bits |= 0b1000;
    }
    ItemFlags(bits)
}

/// Numéro de slot Archipelago (à partir de 1) d'un joueur du multiworld.
fn slot_number(slot: logic::SlotId) -> u32 {
    slot.raw() + 1
}

pub(crate) fn build(
    world: &MultiWorld,
    request: &GenerateRequest,
    seed_name: &str,
    start: &State,
    start_items: &[Vec<ItemKindId>],
    slot_data: Vec<Value>,
) -> Result<MultiData, GenerateError> {
    let mut slots = BTreeMap::new();
    let mut connect_names = BTreeMap::new();
    let mut minimum_client_versions = BTreeMap::new();
    let mut locations = BTreeMap::new();
    let mut precollected_items = BTreeMap::new();
    let mut precollected_hints = BTreeMap::new();
    let mut slot_data_map = BTreeMap::new();
    let mut datapackage = IndexMap::new();

    for ((slot_id, slot), (player, data)) in world
        .slot_ids()
        .zip(world.slots())
        .zip(request.players.iter().zip(slot_data))
    {
        let number = slot_number(slot_id);
        slots.insert(
            number,
            NetworkSlot {
                name: slot.name.clone(),
                game: slot.game.clone(),
                slot_type: SlotType::PLAYER,
                group_members: Vec::new(),
            },
        );
        connect_names.insert(slot.name.clone(), (0, number));
        minimum_client_versions.insert(number, MINIMUM_CLIENT_VERSION);
        locations.insert(number, BTreeMap::new());
        precollected_hints.insert(number, Vec::new());
        slot_data_map.insert(number, data);
        // Comme en Python, un jeu n'apparaît qu'une fois, à la place de son premier joueur.
        datapackage
            .entry(player.options.game().to_string())
            .or_insert_with(|| player.options.package());
    }
    datapackage.insert("Archipelago".to_string(), archipelago_package());

    // Inventaire de départ : seuls les objets qui ont un numéro réseau.
    for (slot_id, items) in world.slot_ids().zip(start_items) {
        let codes: Vec<i64> = items
            .iter()
            .filter_map(|item| world.item_kind(*item).and_then(|k| k.code))
            .collect();
        precollected_items.insert(slot_number(slot_id), codes);
    }

    // Contenu des lieux envoyables (les événements n'ont pas de numéro).
    for location in world.locations() {
        let (Some(address), Some(item)) = (location.address, location.item) else {
            continue;
        };
        let Some(kind) = world.item_kind(item) else {
            continue;
        };
        let Some(code) = kind.code else {
            continue;
        };
        if let Some(table) = locations.get_mut(&slot_number(location.slot)) {
            table.insert(
                address,
                PlacedItem {
                    item: code,
                    player: slot_number(kind.slot),
                    flags: flags(kind.classification),
                },
            );
        }
    }

    Ok(MultiData {
        seed_name: seed_name.to_string(),
        generator_version: GENERATOR_VERSION,
        minimum_server_version: MINIMUM_SERVER_VERSION,
        minimum_client_versions,
        tags: vec!["AP".to_string()],
        race_mode: false,
        slots,
        connect_names,
        slot_data: slot_data_map,
        locations,
        precollected_items,
        precollected_hints,
        er_hint_data: BTreeMap::new(),
        spheres: sendable_spheres(world, start.clone()),
        server_options: ServerOptions::default(),
        datapackage,
    })
}

/// Sphères des lieux envoyables, comme `MultiWorld.get_sendable_spheres` : avant chaque
/// sphère, les événements atteignables sont ramassés d'office (ils ne forment pas de
/// sphère) ; une sphère contient les lieux envoyables devenus atteignables.
fn sendable_spheres(world: &MultiWorld, start: State) -> Vec<BTreeMap<u32, Vec<i64>>> {
    let mut state = start;
    let mut reach = Reachability::compute(world, &state);
    let mut collected = vec![false; world.locations().len()];
    let mut spheres = Vec::new();

    // Un lieu est envoyable s'il a un numéro et que son objet en a un aussi.
    let sendable = |id: logic::LocationId| {
        world.location(id).is_some_and(|l| {
            l.address.is_some()
                && l.item
                    .and_then(|item| world.item_kind(item))
                    .is_some_and(|k| k.code.is_some())
        })
    };

    loop {
        // Événements : on les ramasse jusqu'à ce qu'il n'y en ait plus de nouveau.
        loop {
            let mut found = false;
            for id in world.location_ids() {
                let index = id.index();
                if collected[index] || sendable(id) || !reach.location(id) {
                    continue;
                }
                collected[index] = true;
                if let Some(item) = world.location(id).and_then(|l| l.item) {
                    state.collect(world, item);
                }
                found = true;
            }
            if !found {
                break;
            }
            reach.update(world, &state);
        }

        let sphere: Vec<logic::LocationId> = world
            .location_ids()
            .filter(|id| !collected[id.index()] && sendable(*id) && reach.location(*id))
            .collect();
        if sphere.is_empty() {
            break;
        }
        let mut by_slot: BTreeMap<u32, Vec<i64>> = BTreeMap::new();
        for id in &sphere {
            collected[id.index()] = true;
            if let Some(location) = world.location(*id) {
                if let Some(item) = location.item {
                    state.collect(world, item);
                }
                if let Some(address) = location.address {
                    by_slot
                        .entry(slot_number(location.slot))
                        .or_default()
                        .push(address);
                }
            }
        }
        for addresses in by_slot.values_mut() {
            addresses.sort_unstable();
        }
        spheres.push(by_slot);
        reach.update(world, &state);
    }
    spheres
}
