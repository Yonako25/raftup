//! Messages d'indices : port de `Hint.as_network_message` et des textes de
//! statut de `NetUtils.py` (0.6.8).

use ap_protocol::server::{JsonMessagePart, PrintJson};
use ap_protocol::{Hint, HintStatus, NetworkItem};

/// Texte affiché pour un statut (`status_names`), `(unknown)` sinon.
pub(crate) fn hint_status_text(status: HintStatus) -> &'static str {
    match status {
        HintStatus::FOUND => "(found)",
        HintStatus::UNSPECIFIED => "(unspecified)",
        HintStatus::NO_PRIORITY => "(no priority)",
        HintStatus::AVOID => "(avoid)",
        HintStatus::PRIORITY => "(priority)",
        _ => "(unknown)",
    }
}

/// Morceau de texte sans type (`add_json_text` sans `type`).
fn text(value: impl ToString) -> JsonMessagePart {
    JsonMessagePart {
        text: Some(value.to_string()),
        kind: None,
        color: None,
        flags: None,
        player: None,
        hint_status: None,
    }
}

/// Morceau de texte avec un type (`player_id`, `entrance_name`…).
fn typed(value: impl ToString, kind: &str) -> JsonMessagePart {
    JsonMessagePart {
        kind: Some(kind.to_string()),
        ..text(value)
    }
}

/// Le message `PrintJSON` de type `Hint` d'un indice (`as_network_message`).
pub(crate) fn hint_message(hint: &Hint) -> PrintJson {
    let mut data = vec![
        text("[Hint]: "),
        typed(hint.receiving_player, "player_id"),
        text("'s "),
        JsonMessagePart {
            player: Some(hint.receiving_player),
            flags: Some(hint.item_flags),
            ..typed(hint.item, "item_id")
        },
        text(" is at "),
        JsonMessagePart {
            player: Some(hint.finding_player),
            ..typed(hint.location, "location_id")
        },
        text(" in "),
        typed(hint.finding_player, "player_id"),
    ];
    if hint.entrance.is_empty() {
        data.push(text("'s World"));
    } else {
        data.push(text("'s World at "));
        data.push(typed(&hint.entrance, "entrance_name"));
    }
    data.push(text(". "));
    data.push(JsonMessagePart {
        hint_status: Some(hint.status),
        ..typed(hint_status_text(hint.status), "hint_status")
    });
    PrintJson {
        data,
        kind: Some("Hint".to_string()),
        receiving: Some(hint.receiving_player),
        item: Some(NetworkItem {
            item: hint.item,
            location: hint.location,
            player: hint.finding_player,
            flags: hint.item_flags,
        }),
        found: Some(hint.found),
        team: None,
        slot: None,
        message: None,
        tags: None,
        countdown: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ap_protocol::ItemFlags;
    use serde_json::json;

    fn hint(entrance: &str, status: HintStatus) -> Hint {
        Hint {
            receiving_player: 2,
            finding_player: 1,
            location: 10,
            item: 100,
            found: false,
            entrance: entrance.to_string(),
            item_flags: ItemFlags(1),
            status,
        }
    }

    #[test]
    fn textes_de_statut() {
        assert_eq!(hint_status_text(HintStatus::FOUND), "(found)");
        assert_eq!(hint_status_text(HintStatus::UNSPECIFIED), "(unspecified)");
        assert_eq!(hint_status_text(HintStatus::NO_PRIORITY), "(no priority)");
        assert_eq!(hint_status_text(HintStatus::AVOID), "(avoid)");
        assert_eq!(hint_status_text(HintStatus::PRIORITY), "(priority)");
        assert_eq!(hint_status_text(HintStatus(7)), "(unknown)");
    }

    #[test]
    fn message_sans_entree() {
        let value = serde_json::to_value(hint_message(&hint("", HintStatus::PRIORITY))).unwrap();
        let expected = json!({
            "data": [
                {"text": "[Hint]: "},
                {"text": "2", "type": "player_id"},
                {"text": "'s "},
                {"text": "100", "player": 2, "flags": 1, "type": "item_id"},
                {"text": " is at "},
                {"text": "10", "player": 1, "type": "location_id"},
                {"text": " in "},
                {"text": "1", "type": "player_id"},
                {"text": "'s World"},
                {"text": ". "},
                {"text": "(priority)", "hint_status": 30, "type": "hint_status"}
            ],
            "type": "Hint",
            "receiving": 2,
            "item": {"class": "NetworkItem", "item": 100, "location": 10, "player": 1, "flags": 1},
            "found": false
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn message_avec_entree() {
        let value = serde_json::to_value(hint_message(&hint("Porte", HintStatus::AVOID))).unwrap();
        let parts = value["data"].as_array().unwrap();
        assert_eq!(parts.len(), 12);
        assert_eq!(parts[8], json!({"text": "'s World at "}));
        assert_eq!(parts[9], json!({"text": "Porte", "type": "entrance_name"}));
        assert_eq!(parts[11]["text"], "(avoid)");
    }
}
