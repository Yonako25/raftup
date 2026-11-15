//! Tests sur de vrais paquets capturés auprès du serveur officiel (Archipelago 0.6.8).
//!
//! Voir `tests/captures/README.md` pour la provenance des fichiers.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use ap_protocol::server::JsonMessagePart;
use ap_protocol::{decode_server_frame, encode_server_frame, ItemFlags, ServerPacket};
use serde_json::Value;

/// Dossier des captures. `CARGO_MANIFEST_DIR` = dossier de la crate.
fn captures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/captures/serveur-0.6.8")
}

fn read_capture(name: &str) -> String {
    fs::read_to_string(captures_dir().join(name)).unwrap()
}

/// Lit une capture qui ne contient qu'un paquet.
fn single_packet(name: &str) -> ServerPacket {
    let mut packets = decode_server_frame(&read_capture(name)).unwrap();
    assert_eq!(packets.len(), 1, "{name}");
    packets.remove(0)
}

/// Pour chaque capture : lecture, réécriture, et comparaison avec l'original.
/// Le JSON réécrit doit être identique à l'original, champ par champ.
#[test]
fn toutes_les_captures_se_relisent_a_l_identique() {
    let mut count = 0;
    for entry in fs::read_dir(captures_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let packets =
            decode_server_frame(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for packet in &packets {
            assert!(
                !matches!(packet, ServerPacket::Unknown(_)),
                "{}: paquet {} non reconnu",
                path.display(),
                packet.cmd()
            );
        }
        let rewritten = encode_server_frame(&packets).unwrap();
        let original: Value = serde_json::from_str(&text).unwrap();
        let again: Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(original, again, "{}", path.display());
        count += 1;
    }
    // Garde-fou : le test ne doit pas passer « à vide » si le dossier change.
    assert!(count >= 20, "seulement {count} captures trouvées");
}

#[test]
fn room_info() {
    let ServerPacket::RoomInfo(info) = single_packet("RoomInfo.json") else {
        panic!("RoomInfo attendu");
    };
    assert_eq!(
        (info.version.major, info.version.minor, info.version.build),
        (0, 6, 8)
    );
    assert!(!info.password);
    assert_eq!(info.games, ["Archipelago", "Slime Rancher"]);
    assert_eq!(info.hint_cost, 10);
    assert_eq!(info.seed_name, "13515657874892102023");
    // Les clés gardent l'ordre du serveur (preserve_order).
    let keys: Vec<&String> = info.permissions.keys().collect();
    assert_eq!(keys, ["release", "remaining", "collect"]);
    // Le nombre à virgule n'est pas abîmé.
    assert_eq!(info.time, 1794728556.4879146);
}

#[test]
fn connexion_refusee() {
    let ServerPacket::ConnectionRefused(refused) = single_packet("ConnectionRefused.json") else {
        panic!("ConnectionRefused attendu");
    };
    assert_eq!(refused.errors, ["InvalidSlot"]);
}

#[test]
fn connexion_acceptee() {
    let ServerPacket::Connected(connected) = single_packet("Connected.json") else {
        panic!("Connected attendu");
    };
    assert_eq!((connected.team, connected.slot), (0, 1));
    assert_eq!(connected.players[0].name, "Rancher1");
    assert!(connected.checked_locations.is_empty());
    assert!(!connected.missing_locations.is_empty());
    // Les clés `"1"` du JSON deviennent des nombres.
    assert_eq!(connected.slot_info[&1].game, "Slime Rancher");
    assert!(connected.slot_data.is_some());
}

#[test]
fn objets_de_depart() {
    let ServerPacket::ReceivedItems(received) = single_packet("ReceivedItems.json") else {
        panic!("ReceivedItems attendu");
    };
    assert_eq!(received.index, 0);
    // Inventaire de départ : lieu -2, donné par le serveur (joueur 0).
    assert!(received
        .items
        .iter()
        .all(|i| i.location == -2 && i.player == 0));
}

#[test]
fn objet_envoye() {
    let ServerPacket::PrintJson(print) = single_packet("PrintJSON_ItemSend.json") else {
        panic!("PrintJSON attendu");
    };
    assert_eq!(print.kind.as_deref(), Some("ItemSend"));
    assert_eq!(print.receiving, Some(1));
    let item = print.item.unwrap();
    assert!(item.flags.contains(ItemFlags::PROGRESSION));
    let kinds: Vec<Option<&str>> = print.data.iter().map(|p| p.kind.as_deref()).collect();
    assert_eq!(
        kinds,
        [
            Some("player_id"),
            None,
            Some("item_id"),
            None,
            Some("location_id"),
            None
        ]
    );
    assert_eq!(
        print.data[2],
        JsonMessagePart {
            text: Some("6".into()),
            kind: Some("item_id".into()),
            color: None,
            flags: Some(ItemFlags::PROGRESSION),
            player: Some(1),
            hint_status: None,
        }
    );
}

#[test]
fn message_sans_type() {
    let ServerPacket::PrintJson(print) = single_packet("PrintJSON.json") else {
        panic!("PrintJSON attendu");
    };
    assert_eq!(print.kind, None);
}

#[test]
fn deathlink() {
    let ServerPacket::Bounced(bounced) = single_packet("Bounced_DeathLink.json") else {
        panic!("Bounced attendu");
    };
    assert_eq!(
        bounced.tags.as_deref(),
        Some(&["DeathLink".to_string()][..])
    );
    assert_eq!(bounced.data.as_ref().unwrap()["cause"], "test");
    // `1700000000.0` doit être réécrit avec sa virgule, comme en Python.
    let rewritten = encode_server_frame(&[ServerPacket::Bounced(bounced)]).unwrap();
    assert!(rewritten.contains("1700000000.0"), "{rewritten}");
}

#[test]
fn paquet_invalide() {
    let ServerPacket::InvalidPacket(invalid) = single_packet("InvalidPacket.json") else {
        panic!("InvalidPacket attendu");
    };
    assert_eq!(invalid.kind, "arguments");
    assert_eq!(invalid.original_cmd.as_deref(), Some("LocationScouts"));
}

#[test]
fn reponse_au_set() {
    let ServerPacket::SetReply(reply) = single_packet("SetReply.json") else {
        panic!("SetReply attendu");
    };
    assert_eq!(reply.key, "raftup_test");
    assert_eq!(reply.value, 5);
    assert_eq!(reply.original_value, 0);
    assert_eq!(reply.slot, Some(1));
    // Les champs du `Set` d'origine sont gardés.
    assert_eq!(reply.extra["want_reply"], true);
    assert!(reply.extra.contains_key("operations"));
    assert!(!reply.extra.contains_key("cmd"));
}

#[test]
fn trame_de_plusieurs_paquets() {
    let packets = decode_server_frame(&read_capture("trame_multiple.json")).unwrap();
    assert!(packets.len() > 1);
    assert_eq!(packets[0].cmd(), "Connected");
}
