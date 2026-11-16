//! Constantes et petites fonctions partagées : messages texte, paquets d'erreur,
//! lecture de nombres et de versions.

use super::*;

/// Version minimale d'un client (`min_client_version` du serveur officiel).
pub(super) const MIN_CLIENT_VERSION: NetworkVersion = NetworkVersion {
    major: 0,
    minor: 5,
    build: 0,
};

/// Ancienne version minimale, pour les seeds d'avant 0.6.2.
pub(super) const OLD_MIN_CLIENT_VERSION: NetworkVersion = NetworkVersion {
    major: 0,
    minor: 1,
    build: 6,
};

/// Tags des outils qui ne jouent pas, et le verbe affiché quand ils arrivent.
/// L'ordre compte : c'est celui du serveur officiel (`_non_game_messages`).
pub(super) const NON_GAME_TAGS: [(&str, &str); 3] = [
    ("HintGame", "hinting"),
    ("Tracker", "tracking"),
    ("TextOnly", "viewing"),
];

pub(super) const TUTORIAL: &str =
    "Now that you are connected, you can use !help to list commands to run \
via the server. If your client supports it, you may have additional local commands you can \
list with /help.";

/// Nombre maximal de messages `ItemSend` par trame, comme le serveur officiel.
pub(super) const ITEM_SEND_CHUNK: usize = 140;

/// Lit un nombre écrit exactement comme Python l'écrirait (`7`, pas `07` ni `+7`).
pub(super) fn parse_exact(text: &str) -> Option<u32> {
    let number: u32 = text.parse().ok()?;
    (number.to_string() == text).then_some(number)
}

/// Lit `<équipe>_<slot>`, par exemple `0_3`.
pub(super) fn parse_team_slot(text: &str) -> Option<(TeamId, SlotId)> {
    let (team, slot) = text.split_once('_')?;
    Some((parse_exact(team)?, parse_exact(slot)?))
}

/// Imite `str.isprintable()` de Python : faux si le texte contient un caractère
/// de contrôle, un séparateur autre que l'espace, un caractère de format
/// (invisible) ou un caractère à usage privé.
pub(super) fn is_printable(text: &str) -> bool {
    text.chars().all(|c| {
        let code = c as u32;
        let separator = c != ' ' && c.is_whitespace();
        let format = matches!(code,
            0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F | 0x180E
            | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
            | 0xFEFF | 0xFFF9..=0xFFFB);
        let private = matches!(code, 0xE000..=0xF8FF | 0xF0000..=0x10FFFF);
        !(c.is_control() || separator || format || private)
    })
}

pub(super) fn encode(packets: &[ServerPacket]) -> Option<String> {
    match encode_server_frame(packets) {
        Ok(frame) => Some(frame),
        Err(err) => {
            error!("impossible d'écrire un paquet en JSON : {err}");
            None
        }
    }
}

pub(super) fn invalid_packet(kind: &str, original_cmd: &str, text: &str) -> ServerPacket {
    ServerPacket::InvalidPacket(InvalidPacket {
        kind: kind.to_string(),
        original_cmd: Some(original_cmd.to_string()),
        text: text.to_string(),
    })
}

/// `InvalidPacket` de type `cmd`, sans commande d'origine.
pub(super) fn invalid_cmd(text: &str) -> ServerPacket {
    ServerPacket::InvalidPacket(InvalidPacket {
        kind: "cmd".to_string(),
        original_cmd: None,
        text: text.to_string(),
    })
}

pub(super) fn to_set(tags: &[String]) -> BTreeSet<&str> {
    tags.iter().map(String::as_str).collect()
}

/// Verbe affiché pour un outil qui ne joue pas (`viewing`…), selon ses tags.
pub(super) fn non_game_verb(tags: &[String]) -> Option<&'static str> {
    NON_GAME_TAGS
        .iter()
        .find(|(tag, _)| tags.iter().any(|t| t == tag))
        .map(|(_, verb)| *verb)
}

/// Morceau de texte simple d'un `PrintJSON`.
pub(super) fn text_part(text: impl Into<String>) -> JsonMessagePart {
    JsonMessagePart {
        text: Some(text.into()),
        kind: None,
        color: None,
        flags: None,
        player: None,
        hint_status: None,
    }
}

/// `PrintJSON` d'un seul morceau de texte, sans genre.
pub(super) fn text_message(text: String) -> PrintJson {
    PrintJson {
        data: vec![text_part(text)],
        kind: None,
        receiving: None,
        item: None,
        found: None,
        team: None,
        slot: None,
        message: None,
        tags: None,
        countdown: None,
    }
}

/// Message texte d'un genre donné, qui concerne un slot (`Goal`, `Release`…).
pub(super) fn slot_message(text: String, kind: &str, team: TeamId, slot: SlotId) -> PrintJson {
    let mut message = text_message(text);
    message.kind = Some(kind.to_string());
    message.team = Some(team);
    message.slot = Some(slot);
    message
}

/// Message « Alice sent Objet to Bob (Lieu) » (`json_format_send_event`).
/// Les numéros sont remplacés par des noms chez le client, grâce au DataPackage.
pub(super) fn item_send_message(sent: &Sent) -> PrintJson {
    let item = sent.item;
    let player_part = |slot: SlotId| JsonMessagePart {
        kind: Some("player_id".to_string()),
        ..text_part(slot.to_string())
    };
    let item_part = |owner: SlotId| JsonMessagePart {
        kind: Some("item_id".to_string()),
        player: Some(owner),
        flags: Some(item.flags),
        ..text_part(item.item.to_string())
    };
    let mut parts = vec![player_part(item.player)];
    if item.player == sent.receiver {
        parts.push(text_part(" found their "));
        parts.push(item_part(item.player));
    } else {
        parts.push(text_part(" sent "));
        parts.push(item_part(sent.receiver));
        parts.push(text_part(" to "));
        parts.push(player_part(sent.receiver));
    }
    parts.push(text_part(" ("));
    parts.push(JsonMessagePart {
        kind: Some("location_id".to_string()),
        player: Some(item.player),
        ..text_part(item.location.to_string())
    });
    parts.push(text_part(")"));

    let mut message = text_message(String::new());
    message.data = parts;
    message.kind = Some("ItemSend".to_string());
    message.receiving = Some(sent.receiver);
    message.item = Some(item);
    message
}

/// Version sous forme de triplet, pour comparer comme Python compare des tuples.
pub(super) fn version_key(version: &NetworkVersion) -> (u32, u32, u32) {
    (version.major, version.minor, version.build)
}

/// `0.6.8`
pub(super) fn version_text(version: &NetworkVersion) -> String {
    format!("{}.{}.{}", version.major, version.minor, version.build)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imprimable_comme_python() {
        assert!(is_printable(""));
        assert!(is_printable("Bonjour à tous ! 🎉"));
        assert!(!is_printable("ligne\nsuivante"));
        assert!(!is_printable("tab\t"));
        assert!(!is_printable("espace\u{a0}insécable"));
        assert!(!is_printable("zéro\u{200b}largeur"));
    }
}
