//! Tests des pièges connus du protocole (ADR 0004).

// Fichier de test : un unwrap() qui échoue fait simplement échouer le test.
#![allow(clippy::unwrap_used)]

use ap_protocol::server::Retrieved;
use ap_protocol::{
    decode_server_frame, encode_server_frame, NetworkItem, ProtocolError, ServerPacket,
};
use serde_json::{json, Map, Value};

/// Les entiers plus grands que u64 (bitfields du stockage) ne sont pas arrondis.
#[test]
fn grands_entiers_conserves() {
    let text = r#"[{"cmd":"Retrieved","keys":{"bits":123456789012345678901234567890}}]"#;
    let packets = decode_server_frame(text).unwrap();
    assert_eq!(encode_server_frame(&packets).unwrap(), text);
}

/// L'ordre des clés est gardé, comme dans un dictionnaire Python.
#[test]
fn ordre_des_cles_conserve() {
    let mut keys = Map::new();
    keys.insert("zebre".into(), json!(1));
    keys.insert("abeille".into(), json!(2));
    let packet = ServerPacket::Retrieved(Retrieved {
        keys,
        extra: Map::new(),
    });
    let text = encode_server_frame(&[packet]).unwrap();
    assert_eq!(
        text,
        r#"[{"cmd":"Retrieved","keys":{"zebre":1,"abeille":2}}]"#
    );
}

/// Un champ inconnu (protocole plus récent) ne fait pas échouer la lecture.
#[test]
fn champ_inconnu_tolere() {
    let text = r#"[{"cmd":"ConnectionRefused","errors":["InvalidSlot"],"nouveau":42}]"#;
    let packets = decode_server_frame(text).unwrap();
    assert_eq!(packets[0].cmd(), "ConnectionRefused");
}

/// Une commande inconnue est gardée telle quelle, et réécrite à l'identique.
#[test]
fn commande_inconnue_gardee() {
    let text = r#"[{"cmd":"Nouveaute","x":1.5}]"#;
    let packets = decode_server_frame(text).unwrap();
    assert!(matches!(packets[0], ServerPacket::Unknown(_)));
    assert_eq!(packets[0].cmd(), "Nouveaute");
    assert_eq!(encode_server_frame(&packets).unwrap(), text);
}

/// Un JSON très imbriqué est refusé proprement, sans faire planter le programme.
#[test]
fn imbrication_profonde_refusee() {
    let depth = 100_000;
    let text = format!(
        r#"[{{"cmd":"Bounced","data":{{"x":{}{}}}}}]"#,
        "[".repeat(depth),
        "]".repeat(depth)
    );
    assert!(matches!(
        decode_server_frame(&text),
        Err(ProtocolError::Json(_))
    ));
}

#[test]
fn paquet_sans_cmd_refuse() {
    assert!(matches!(
        decode_server_frame(r#"[{"errors":[]}]"#),
        Err(ProtocolError::MissingCmd)
    ));
    assert!(matches!(
        decode_server_frame(r#"[42]"#),
        Err(ProtocolError::NotAnObject)
    ));
}

#[test]
fn paquet_connu_mal_forme_refuse() {
    let err = decode_server_frame(r#"[{"cmd":"ReceivedItems","index":"zero"}]"#).unwrap_err();
    match err {
        ProtocolError::InvalidPacket { cmd, .. } => assert_eq!(cmd, "ReceivedItems"),
        other => panic!("erreur inattendue : {other}"),
    }
}

/// Le champ `"class"` est écrit : les clients Python en ont besoin.
#[test]
fn champ_class_ecrit() {
    let item = NetworkItem {
        item: 6,
        location: 1,
        player: 1,
        flags: ap_protocol::ItemFlags::PROGRESSION,
    };
    let value: Value = serde_json::to_value(item).unwrap();
    assert_eq!(value["class"], "NetworkItem");
}
