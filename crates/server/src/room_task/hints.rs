//! Indices : `CreateHints`, `UpdateHint` et l'envoi des effets d'un indice
//! (`on_new_hint`, `on_changed_hints` et la fin de `notify_hints` du serveur officiel).

use ap_protocol::client::{CreateHints, UpdateHint};

use super::*;
use crate::game::{slot_set, HintEffects};
use crate::hints::hint_message;

/// Statut valide au sens de Python : `HintStatus(valeur)` lève `ValueError`
/// pour tout nombre qui n'est pas l'un des cinq statuts.
fn valid_status(value: i64) -> Option<HintStatus> {
    [
        HintStatus::UNSPECIFIED,
        HintStatus::NO_PRIORITY,
        HintStatus::AVOID,
        HintStatus::PRIORITY,
        HintStatus::FOUND,
    ]
    .into_iter()
    .find(|s| i64::from(s.0) == value)
}

/// Entier au sens de Python (`isinstance(x, int)`) : un nombre entier, ou un
/// booléen (`True` vaut 1 et `False` 0 en Python).
fn py_int(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

/// `HintStatus(valeur)` de Python pour une valeur JSON quelconque.
///
/// La recherche se fait par égalité : `30.0` et `True` (== 1) sont cherchés
/// comme `30` et `1`. Échec : `Err(repr(valeur))`, pour le message
/// « Unknown Status: {repr} is not a valid HintStatus ».
fn py_hint_status(value: &Value) -> Result<HintStatus, String> {
    let number = py_int(value).or_else(|| {
        value
            .as_f64()
            // Un flottant rond est égal à l'entier ; borné pour la conversion.
            .filter(|f| f.fract() == 0.0 && f.abs() < 1e15)
            .map(|f| f as i64)
    });
    number
        .and_then(valid_status)
        .ok_or_else(|| python::repr(value))
}

impl RoomTask {
    /// Points d'indice d'un slot (`get_slot_points`).
    pub(super) fn hint_points(&self, team: TeamId, slot: SlotId) -> i64 {
        let per_check = self.room.data().server_options.location_check_points;
        self.game
            .slot_points(self.room.data(), team, slot, per_check)
    }

    /// Envoie ce que `notify_hints` demande, dans l'ordre du serveur officiel.
    ///
    /// 1. Pour chaque slot qui reçoit un nouvel indice (`on_new_hint`) : le
    ///    `SetReply` de `_read_hints_<équipe>_<slot>` aux abonnés, puis un
    ///    `RoomUpdate { hint_points }` aux clients du slot ;
    /// 2. puis, pour chaque slot concerné, une seule trame avec un `PrintJSON`
    ///    par indice, aux clients qui acceptent le texte.
    pub(super) fn apply_hint_effects(&self, team: TeamId, effects: HintEffects) {
        for (hint_team, slot) in effects.new_hint_slots {
            let value = hints_value(&self.game.hints(hint_team, slot));
            self.notify_read_key(format!("_read_hints_{hint_team}_{slot}"), value);
            let update = RoomUpdate {
                hint_points: Some(self.hint_points(hint_team, slot)),
                ..RoomUpdate::default()
            };
            self.broadcast_slot(hint_team, slot, vec![ServerPacket::RoomUpdate(update)]);
        }
        for (slot, hints) in effects.messages {
            let packets: Vec<ServerPacket> = hints
                .iter()
                .map(|h| ServerPacket::PrintJson(hint_message(h)))
                .collect();
            self.broadcast(&packets, |p| p.team == team && p.slot == slot && !p.no_text);
        }
    }

    /// `CreateHints` bien formé.
    pub(super) fn on_create_hints(&mut self, id: ClientId, request: CreateHints) {
        let status = match request.status {
            None => Ok(HintStatus::UNSPECIFIED),
            Some(s) => valid_status(i64::from(s.0)).ok_or_else(|| s.0.to_string()),
        };
        self.create_hints(id, &request.locations, request.player, status);
    }

    /// `CreateHints` lu champ par champ, comme Python lit son dictionnaire :
    /// statut de n'importe quel type (`HintStatus(valeur)`), `player` à `null`
    /// ou d'un autre type (plantage au premier lieu : fermeture). Sans liste
    /// de lieux lisible, le serveur officiel plante aussi.
    pub(super) fn on_bad_create_hints(&mut self, id: ClientId, raw: Option<&Value>) {
        let Some(Value::Object(map)) = raw else {
            return self.close(id, "CreateHints : paquet mal formé");
        };
        let locations = map
            .get("locations")
            .and_then(|v| serde_json::from_value::<Vec<LocationId>>(v.clone()).ok());
        let Some(locations) = locations else {
            return self.close(id, "CreateHints : lieux illisibles");
        };
        // Un joueur qui n'est pas un numéro de slot ne correspond à aucun
        // monde : `u32::MAX` n'existe pas, `create_hints` fermera la connexion
        // au premier lieu, comme le `KeyError` de Python.
        let player = map.get("player").map(|v| {
            py_int(v)
                .and_then(|p| SlotId::try_from(p).ok())
                .unwrap_or(SlotId::MAX)
        });
        let status = match map.get("status") {
            None => Ok(HintStatus::UNSPECIFIED),
            Some(value) => py_hint_status(value),
        };
        self.create_hints(id, &locations, player, status);
    }

    /// Cœur de `CreateHints` (MultiServer.py, `elif cmd == 'CreateHints'`).
    /// `status` : le statut lu, ou `Err(repr)` d'une valeur que Python refuse.
    fn create_hints(
        &mut self,
        id: ClientId,
        locations: &[LocationId],
        player: Option<SlotId>,
        status: Result<HintStatus, String>,
    ) {
        let Some(client) = self.player(id).cloned() else {
            return;
        };
        let refuse = |task: &Self, text: &str| {
            task.send(id, vec![invalid_packet("arguments", "CreateHints", text)]);
        };
        let location_player = player.unwrap_or(client.slot);
        if locations.is_empty() {
            return refuse(self, "CreateHints: No locations specified.");
        }
        let status = match status {
            Ok(status) => status,
            Err(shown) => {
                let text = format!("Unknown Status: {shown} is not a valid HintStatus");
                return refuse(self, &text);
            }
        };

        let data = self.room.data();
        let mut hints = Vec::new();
        for location in locations {
            // Joueur inconnu ou lieu absent de son monde : le serveur officiel
            // plante (`KeyError`) et ferme la connexion.
            let Some(world) = data.locations.get(&location_player) else {
                return self.close(id, "CreateHints : joueur inconnu");
            };
            if location_player != client.slot && !world.contains_key(location) {
                let text = "CreateHints: One or more of the locations do not exist for the \
                            specified off-world player. Please refrain from hinting other \
                            slot's locations that you don't know contain your items.";
                return refuse(self, text);
            }
            let Some(placed) = world.get(location) else {
                return self.close(id, "CreateHints : lieu inconnu");
            };
            if !slot_set(data, placed.player).contains(&client.slot) {
                if status != HintStatus::UNSPECIFIED {
                    let text = "CreateHints: Must use \"unspecified\"/None status for items from \
                                other players.";
                    return refuse(self, text);
                }
                if client.slot != location_player {
                    let text = "CreateHints: Can only create hints for own items or own locations.";
                    return refuse(self, text);
                }
            }
            hints.extend(self.game.collect_hint_location_id(
                data,
                client.team,
                location_player,
                *location,
                Some(status),
            ));
        }
        // Comme le serveur officiel : `only_new` ne change pas le statut d'un
        // indice déjà connu.
        let effects =
            self.game
                .notify_hints(self.room.data(), client.team, &hints, true, true, None);
        self.apply_hint_effects(client.team, effects);
    }

    /// `UpdateHint` bien formé.
    pub(super) fn on_update_hint(&mut self, id: ClientId, request: UpdateHint) {
        let status = request.status.map(|s| i64::from(s.0));
        self.update_hint(id, i64::from(request.player), request.location, status);
    }

    /// `UpdateHint` dont un champ n'a pas le bon type pour nous.
    ///
    /// Serveur officiel : champ absent = plantage (fermeture) ; champ présent
    /// d'un autre type que nombre (ou `null` pour `status`) = `InvalidPacket` ;
    /// nombres hors de notre plage : traités normalement.
    pub(super) fn on_bad_update_hint(&mut self, id: ClientId, raw: Option<&Value>) {
        let field = |name: &str| raw.and_then(|r| r.get(name));
        let (Some(player), Some(location), Some(status)) =
            (field("player"), field("location"), field("status"))
        else {
            return self.close(id, "UpdateHint : champ absent");
        };
        // `isinstance(x, int)` : un booléen compte comme un entier en Python.
        let is_int = |v: &Value| v.is_i64() || v.is_u64() || v.is_boolean();
        if !is_int(player) || !is_int(location) || !(status.is_null() || is_int(status)) {
            self.send(
                id,
                vec![invalid_packet("arguments", "UpdateHint", "UpdateHint")],
            );
            return;
        }
        // Entiers valides mais hors plage (négatifs, énormes) : aucun indice ne
        // correspond ; un statut hors plage est refusé s'il y a un indice.
        let player = py_int(player).unwrap_or(-1);
        let location = py_int(location).unwrap_or(-1);
        let status = if status.is_null() {
            None
        } else {
            Some(py_int(status).unwrap_or(-1))
        };
        self.update_hint(id, player, location, status);
    }

    /// Cœur de `UpdateHint` (MultiServer.py, `elif cmd == 'UpdateHint'`).
    /// Ni `PrintJSON` ni `RoomUpdate` : seulement les `SetReply` de `_read_hints_…`.
    fn update_hint(
        &mut self,
        id: ClientId,
        player: i64,
        location: LocationId,
        status: Option<i64>,
    ) {
        let Some(client) = self.player(id).cloned() else {
            return;
        };
        let refuse = |task: &Self, text: &str| {
            task.send(id, vec![invalid_packet("arguments", "UpdateHint", text)]);
        };
        // Aucun indice : ignoré sans réponse.
        let Ok(finding) = SlotId::try_from(player) else {
            return;
        };
        let Some(hint) = self.game.get_hint(client.team, finding, location) else {
            return;
        };
        if !slot_set(self.room.data(), hint.receiving_player).contains(&client.slot) {
            return refuse(self, "UpdateHint: No Permission");
        }
        let Some(status) = status else {
            return;
        };
        let Some(status) = valid_status(status) else {
            return refuse(self, "UpdateHint: Invalid Status");
        };
        if status == HintStatus::FOUND {
            let text = "UpdateHint: Cannot manually update status to \"HINT_FOUND\"";
            return refuse(self, text);
        }
        let changed =
            self.game
                .update_hint(self.room.data(), client.team, finding, location, status);
        for (team, slot) in changed {
            let value = hints_value(&self.game.hints(team, slot));
            self.notify_read_key(format!("_read_hints_{team}_{slot}"), value);
        }
    }
}
