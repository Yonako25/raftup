//! Tests sur des paquets client écrits par l'encodeur officiel (Archipelago 0.6.8)
//! et acceptés par le serveur officiel.
//!
//! Voir `tests/captures/README.md` pour la provenance des fichiers.

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use ap_protocol::client::{Bounce, Connect, DataStorageOperation, Set, UpdateHint};
use ap_protocol::{
    decode_client_frame, encode_client_frame, ClientPacket, ClientStatus, HintStatus,
    ItemsHandling, NetworkVersion,
};
use serde_json::{json, Map, Value};

/// Dossier des captures. `CARGO_MANIFEST_DIR` = dossier de la crate.
fn captures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/captures/client-0.6.8")
}

fn read_capture(name: &str) -> String {
    fs::read_to_string(captures_dir().join(name)).unwrap()
}

/// Lit une capture qui ne contient qu'un paquet.
fn single_packet(name: &str) -> ClientPacket {
    let mut packets = decode_client_frame(&read_capture(name)).unwrap();
    assert_eq!(packets.len(), 1, "{name}");
    packets.remove(0)
}

/// Vérifie que `packet`, une fois écrit, donne le même JSON que la capture.
fn assert_same_json(packet: ClientPacket, name: &str) {
    let ours: Value = serde_json::from_str(&encode_client_frame(&[packet]).unwrap()).unwrap();
    let official: Value = serde_json::from_str(&read_capture(name)).unwrap();
    assert_eq!(ours, official, "{name}");
}

/// Pour chaque capture : lecture, réécriture, et comparaison avec l'original.
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
            decode_client_frame(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for packet in &packets {
            assert!(
                !matches!(packet, ClientPacket::Unknown(_)),
                "{}: paquet {} non reconnu",
                path.display(),
                packet.cmd()
            );
        }
        let rewritten = encode_client_frame(&packets).unwrap();
        let original: Value = serde_json::from_str(&text).unwrap();
        let again: Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(original, again, "{}", path.display());
        count += 1;
    }
    assert!(count >= 14, "seulement {count} captures trouvées");
}

/// Un `Connect` construit en Rust donne exactement le JSON du client officiel,
/// `"class": "Version"` compris.
#[test]
fn connect_comme_le_client_officiel() {
    let connect = Connect {
        password: None,
        game: "Slime Rancher".into(),
        name: "Rancher1".into(),
        uuid: "raftup-test-uuid".into(),
        version: NetworkVersion {
            major: 0,
            minor: 6,
            build: 8,
        },
        items_handling: ItemsHandling::ALL,
        tags: vec!["AP".into()],
        slot_data: Some(true),
    };
    assert_eq!(
        single_packet("Connect.json"),
        ClientPacket::Connect(connect.clone())
    );
    assert_same_json(ClientPacket::Connect(connect), "Connect.json");
}

#[test]
fn sync_sans_champ() {
    assert_eq!(single_packet("Sync.json"), ClientPacket::Sync);
    assert_same_json(ClientPacket::Sync, "Sync.json");
}

#[test]
fn objectif_atteint() {
    let ClientPacket::StatusUpdate(update) = single_packet("StatusUpdate.json") else {
        panic!("StatusUpdate attendu");
    };
    assert_eq!(update.status, ClientStatus::GOAL);
}

#[test]
fn priorite_d_indice() {
    let expected = UpdateHint {
        player: 1,
        location: 6,
        status: Some(HintStatus::AVOID),
    };
    assert_eq!(
        single_packet("UpdateHint.json"),
        ClientPacket::UpdateHint(expected)
    );
}

#[test]
fn set_avec_operations() {
    let set = Set {
        key: "raftup_test".into(),
        default: Some(json!(0)),
        want_reply: Some(true),
        operations: vec![
            DataStorageOperation {
                operation: "add".into(),
                value: json!(5),
            },
            DataStorageOperation {
                operation: "max".into(),
                value: json!(3),
            },
        ],
        extra: Map::new(),
    };
    assert_same_json(ClientPacket::Set(set), "Set.json");
}

#[test]
fn bounce_sans_data() {
    let ClientPacket::Bounce(bounce) = single_packet("Bounce_Ping.json") else {
        panic!("Bounce attendu");
    };
    assert_eq!(
        bounce,
        Bounce {
            slots: Some(vec![1]),
            ..Bounce::default()
        }
    );
}

#[test]
fn deathlink_garde_l_heure_exacte() {
    let text = read_capture("Bounce_DeathLink.json");
    let packets = decode_client_frame(&text).unwrap();
    let rewritten = encode_client_frame(&packets).unwrap();
    assert!(rewritten.contains("1731670000.123456"), "{rewritten}");
    assert!(rewritten.contains("Rancher1 a glissé"), "{rewritten}");
}

#[test]
fn plusieurs_paquets_dans_une_trame() {
    let packets = decode_client_frame(&read_capture("Get_SetNotify.json")).unwrap();
    let cmds: Vec<&str> = packets.iter().map(ClientPacket::cmd).collect();
    assert_eq!(cmds, ["Get", "SetNotify"]);
}
