//! `!hint` et `!hint_location` (`get_hints` de `ClientMessageProcessor`,
//! MultiServer.py 1775-1918).
//!
//! Étapes, comme le serveur officiel :
//! 1. sans argument : réaffiche ses indices et le coût d'un indice ;
//! 2. sinon, trouve les indices demandés (par numéro ou par nom, avec la
//!    recherche floue, groupes compris) ;
//! 3. les indices déjà connus sont gratuits ; parmi les nouveaux pas encore
//!    trouvés, on n'en paie qu'un par commande si les indices coûtent des points,
//!    en tirant d'abord les lieux non locaux de la sphère la plus basse.
//!
//! Écarts déclarés (ADR 0012, 0013) : tirage déterministe à la place du
//! Mersenne Twister ; liste des non indiçables limitée aux mondes de base ;
//! erreur d'arguments courte à la place d'une trace Python.

use multidata::MultiData;

use super::*;
use crate::command_line::py_isnumeric;
use crate::fuzzy::intended_text;
use crate::game::hint_cost;
use crate::hints::{deterministic_shuffle, is_non_hintable};
use crate::pyset::PySet;

/// Indices trouvés pour la demande, et le nom reconnu (pour les messages).
struct Lookup {
    hints: Vec<Hint>,
    name: String,
    /// Nom non indiçable : `hints` est vide et on prévient le joueur.
    blocked: bool,
}

/// Sphère d'un lieu (`get_sphere`) ; -1 si la seed n'a pas de sphères.
/// Un lieu absent des sphères fait planter le serveur officiel : ici -1 aussi.
fn sphere_of(data: &MultiData, player: SlotId, location: LocationId) -> i64 {
    data.spheres
        .iter()
        .position(|sphere| sphere.get(&player).is_some_and(|l| l.contains(&location)))
        .map_or(-1, |i| i as i64)
}

impl RoomTask {
    /// `!hint [objet]` (`for_location = false`) ou `!hint_location [lieu]`.
    pub(super) fn cmd_hint(&mut self, id: ClientId, input: &str, for_location: bool) {
        let Some(player) = self.player(id).cloned() else {
            return;
        };
        let (team, slot) = (player.team, player.slot);
        let points = self.hint_points(team, slot);
        let cost = i64::from(hint_cost(self.room.data(), slot));

        if input.is_empty() {
            let hints = self.game.recheck_own_hints(team, slot);
            let effects =
                self.game
                    .notify_hints(self.room.data(), team, &hints, false, false, Some(&[slot]));
            self.apply_hint_effects(team, effects);
            self.output(
                id,
                format!("A hint costs {cost} points. You have {points} points."),
            );
            return;
        }

        let command = if for_location {
            "hint_location"
        } else {
            "hint"
        };
        let lookup = if py_isnumeric(input) {
            match self.lookup_by_id(slot, team, input, for_location) {
                Some(lookup) => lookup,
                None => {
                    self.invalid_arguments(id, command);
                    return;
                }
            }
        } else {
            match self.lookup_by_name(slot, team, input, for_location) {
                Ok(lookup) => lookup,
                Err(Some(text)) => {
                    self.output(id, text);
                    return;
                }
                Err(None) => {
                    self.invalid_arguments(id, command);
                    return;
                }
            }
        };

        // Comme Python : le refus, PUIS la réponse d'une recherche vide.
        if lookup.blocked {
            self.output(
                id,
                format!("Sorry, \"{}\" is marked as non-hintable.", lookup.name),
            );
        }
        if lookup.hints.is_empty() {
            let text = if points >= cost {
                let what = if for_location { "location" } else { "item" };
                let thing = if for_location { "Location" } else { "Item" };
                format!(
                    "Nothing found for recognized {what} name \"{}\". \
                     {thing} appears to not exist in this multiworld.",
                    lookup.name
                )
            } else {
                format!(
                    "You can't afford the hint. You have {points} points and need at least {cost}."
                )
            };
            self.output(id, text);
            return;
        }
        self.pay_and_send(id, team, slot, points, cost, lookup.hints);
    }

    /// Demande par numéro. `None` : Python planterait (chiffres non ASCII).
    fn lookup_by_id(
        &self,
        slot: SlotId,
        team: TeamId,
        input: &str,
        for_location: bool,
    ) -> Option<Lookup> {
        // `int()` de Python lit aussi d'autres chiffres Unicode, mais
        // `isnumeric()` accepte des caractères qu'`int()` refuse (« ½ ») : on
        // se limite aux chiffres ASCII et on refuse le reste.
        if !input.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let data = self.room.data();
        // Trop grand pour un i64 : aucun objet ni lieu ne peut avoir ce numéro.
        let Ok(number) = input.parse::<i64>() else {
            return Some(Lookup {
                hints: Vec::new(),
                name: "None".to_string(),
                blocked: false,
            });
        };
        let game = self.slot_game(slot);
        let package = data.datapackage.get(&game);
        // Le dictionnaire Python id -> nom garde le dernier nom d'un même id.
        let name = package.and_then(|p| {
            let names = if for_location {
                &p.location_name_to_id
            } else {
                &p.item_name_to_id
            };
            names
                .iter()
                .rev()
                .find(|(_, id)| **id == number)
                .map(|(name, _)| name.clone())
        });
        let name = name.unwrap_or_else(|| "None".to_string());
        if is_non_hintable(&game, &name) {
            return Some(Lookup {
                hints: Vec::new(),
                name,
                blocked: true,
            });
        }
        let hints = if for_location {
            self.game.collect_hint_location_id(
                data,
                team,
                slot,
                number,
                Some(HintStatus::UNSPECIFIED),
            )
        } else {
            self.game.collect_hints(data, team, slot, number, None)
        };
        Some(Lookup {
            hints,
            name,
            blocked: false,
        })
    }

    /// Demande par nom. `Err(Some(texte))` : réponse à renvoyer telle quelle ;
    /// `Err(None)` : Python planterait.
    fn lookup_by_name(
        &self,
        slot: SlotId,
        team: TeamId,
        input: &str,
        for_location: bool,
    ) -> Result<Lookup, Option<String>> {
        let data = self.room.data();
        let game = self.slot_game(slot);
        let Some(package) = data.datapackage.get(&game) else {
            return Err(Some(
                "Can't look up item/location for unknown game. Hint for ID instead.".to_string(),
            ));
        };
        // Candidats : les noms puis les groupes. Python en fait un ensemble
        // (ordre au hasard) ; ici l'ordre du datapackage, stable.
        let (names, groups) = if for_location {
            (&package.location_name_to_id, &package.location_name_groups)
        } else {
            (&package.item_name_to_id, &package.item_name_groups)
        };
        let mut candidates: Vec<&str> = names.keys().map(String::as_str).collect();
        for group in groups.keys() {
            if !names.contains_key(group) {
                candidates.push(group);
            }
        }
        let (name, usable, response) = intended_text(input, &candidates);
        if !usable {
            return Err(Some(response));
        }
        if is_non_hintable(&game, &name) {
            return Ok(Lookup {
                hints: Vec::new(),
                name,
                blocked: true,
            });
        }
        let status = Some(HintStatus::UNSPECIFIED);
        let mut hints = Vec::new();
        if !for_location && package.item_name_groups.contains_key(&name) {
            for item_name in &package.item_name_groups[&name] {
                if let Some(item) = package.item_name_to_id.get(item_name) {
                    hints.extend(self.game.collect_hints(data, team, slot, *item, None));
                }
            }
        } else if let Some(item) = package.item_name_to_id.get(&name).filter(|_| !for_location) {
            hints = self.game.collect_hints(data, team, slot, *item, None);
        } else if let Some(group) = package.location_name_groups.get(&name) {
            for location_name in group {
                if let Some(location) = package.location_name_to_id.get(location_name) {
                    hints.extend(
                        self.game
                            .collect_hint_location_id(data, team, slot, *location, status),
                    );
                }
            }
        } else {
            // Nom de lieu ; absent du jeu : `KeyError` en Python.
            let location = package.location_name_to_id.get(&name).ok_or(None)?;
            hints = self
                .game
                .collect_hint_location_id(data, team, slot, *location, status);
        }
        Ok(Lookup {
            hints,
            name,
            blocked: false,
        })
    }

    /// Fin de `get_hints` : indices déjà connus gratuits, un seul nouvel
    /// indice payé par commande quand les indices coûtent des points.
    fn pay_and_send(
        &mut self,
        id: ClientId,
        team: TeamId,
        slot: SlotId,
        points: i64,
        cost: i64,
        hints: Vec<Hint>,
    ) {
        // Comme le serveur officiel, avec des `set` Python (ADR 0018) :
        // `new_hints = set(hints) - own` puis `old_hints = list(set(hints) - new_hints)`.
        // Leur ordre de parcours fixe celui des messages.
        let own = self.game.hint_set(team, slot);
        let new_set = PySet::from_ordered(hints.iter().cloned()).difference(&own);
        let old_hints: Vec<Hint> = PySet::from_ordered(hints)
            .difference(&new_set)
            .iter()
            .cloned()
            .collect();
        let new_hints: Vec<Hint> = new_set.iter().cloned().collect();

        if new_hints.is_empty() {
            let effects =
                self.game
                    .notify_hints(self.room.data(), team, &old_hints, false, false, None);
            self.apply_hint_effects(team, effects);
            self.output(id, "Hint was previously used, no points deducted.");
            return;
        }

        let (found_hints, mut not_found): (Vec<Hint>, Vec<Hint>) =
            new_hints.into_iter().partition(|h| h.found);
        let mut can_pay: u32 = if not_found.is_empty() || cost == 0 {
            1000
        } else {
            // `//` de Python arrondit vers le bas, comme `div_euclid` avec un
            // diviseur positif (les points peuvent être négatifs).
            u32::from(points.div_euclid(cost) > 0)
        };

        // Tirage (ADR 0012) : ordre de départ fixe, mélange déterministe, puis
        // les deux tris stables du serveur officiel. On prend par la fin :
        // sphère la plus basse, et à sphère égale un lieu non local.
        not_found.sort_by_key(|h| (h.finding_player, h.location));
        let seed_text = format!(
            "{}|{team}|{slot}|{}",
            self.room.data().seed_name,
            self.game.hints_used(team, slot)
        );
        deterministic_shuffle(&mut not_found, &seed_text);
        not_found.sort_by_key(|h| h.receiving_player != h.finding_player);
        let data = self.room.data();
        not_found.sort_by_key(|h| std::cmp::Reverse(sphere_of(data, h.finding_player, h.location)));

        let mut hints = found_hints;
        hints.extend(old_hints);
        while can_pay > 0 {
            let Some(hint) = not_found.pop() else {
                break;
            };
            hints.push(hint);
            can_pay -= 1;
            self.game.add_hints_used(team, slot, 1);
        }

        let effects = self
            .game
            .notify_hints(self.room.data(), team, &hints, false, false, None);
        self.apply_hint_effects(team, effects);

        if !not_found.is_empty() {
            let points = self.hint_points(team, slot);
            let text = if !hints.is_empty() && cost != 0 && points.div_euclid(cost) == 0 {
                format!(
                    "There may be more hintables, however, you cannot afford to pay for any more.  \
                     You have {points} and need at least {cost}."
                )
            } else if !hints.is_empty() {
                "There may be more hintables, you can rerun the command to find more.".to_string()
            } else {
                format!(
                    "You can't afford the hint. You have {points} points and need at least {cost}."
                )
            };
            self.output(id, text);
        }
    }
}
