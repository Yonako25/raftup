//! Explication d'un échec : quel lieu ou quel objectif est inatteignable, et pourquoi.

use std::fmt;

use logic::{MultiWorld, Reachability, RegionId, Rule, State, SweepResult};

/// Ce qui n'est pas atteint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Un lieu (par son nom).
    Location(String),
    /// La condition de victoire du joueur.
    Goal,
}

/// Un lieu ou un objectif inatteignable, avec la clause qui bloque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    /// Nom du joueur.
    pub slot: String,
    pub target: Target,
    /// Clause de règle qui échoue, écrite avec les noms du monde,
    /// par exemple `Has("Key")` ou `région « Castle » : entrée « Forest -> Castle » : Has("Key")`.
    pub clause: String,
}

impl fmt::Display for Blocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.target {
            Target::Location(name) => write!(
                f,
                "{} : lieu « {} » inatteignable ({})",
                self.slot, name, self.clause
            ),
            Target::Goal => write!(
                f,
                "{} : objectif inatteignable ({})",
                self.slot, self.clause
            ),
        }
    }
}

/// Liste ce qui n'est pas atteint à la fin d'un balayage : chaque lieu qui doit l'être
/// (tous avec `accessibility: full` ; sinon voir `MultiWorld::must_reach`), puis
/// l'objectif de chaque joueur. Vide si tout est atteint.
///
/// Ordre : lieux dans l'ordre de création, puis joueurs dans l'ordre.
pub fn explain(world: &MultiWorld, result: &SweepResult) -> Vec<Blocked> {
    let mut blocked = Vec::new();
    for id in result.unreachable_locations(world) {
        if !world.must_reach(id) {
            continue;
        }
        let Some(location) = world.location(id) else {
            continue;
        };
        let clause = if result.reach.region(location.region) {
            failing_clause(world, &location.rule, &result.state, &result.reach)
        } else {
            region_blocker(world, location.region, &result.state, &result.reach)
        };
        blocked.push(Blocked {
            slot: slot_name(world, location.slot),
            target: Target::Location(location.name.clone()),
            clause,
        });
    }
    for slot_id in world.slot_ids() {
        if result.goal_reached(world, slot_id) {
            continue;
        }
        if let Some(slot) = world.slot(slot_id) {
            blocked.push(Blocked {
                slot: slot.name.clone(),
                target: Target::Goal,
                clause: failing_clause(world, &slot.goal, &result.state, &result.reach),
            });
        }
    }
    blocked
}

fn slot_name(world: &MultiWorld, slot: logic::SlotId) -> String {
    world
        .slot(slot)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| format!("joueur n° {}", slot.raw()))
}

/// Pourquoi une région n'est pas atteinte : les entrées qui y mènent depuis une région
/// atteinte et dont la règle échoue.
fn region_blocker(
    world: &MultiWorld,
    region: RegionId,
    state: &State,
    reach: &Reachability,
) -> String {
    let region_name = world.region(region).map(|r| r.name.as_str()).unwrap_or("?");
    let doors: Vec<String> = world
        .entrances()
        .iter()
        .filter(|e| e.to == region && reach.region(e.from))
        .map(|e| {
            format!(
                "entrée « {} » : {}",
                e.name,
                failing_clause(world, &e.rule, state, reach)
            )
        })
        .collect();
    if doors.is_empty() {
        format!("région « {region_name} » : aucune entrée depuis une région atteinte")
    } else {
        format!("région « {region_name} » : {}", doors.join(" ; "))
    }
}

/// La plus petite partie de la règle qui échoue : dans un `And`, on descend dans le premier
/// enfant faux ; ailleurs (`Or`, feuille), on décrit la règle entière.
fn failing_clause(world: &MultiWorld, rule: &Rule, state: &State, reach: &Reachability) -> String {
    if let Rule::And(children) = rule {
        if let Some(child) = children.iter().find(|c| !c.eval(world, state, reach)) {
            return failing_clause(world, child, state, reach);
        }
    }
    describe(world, rule)
}

/// Écrit une règle avec les noms du monde, dans le vocabulaire de `rule_builder`.
pub(crate) fn describe(world: &MultiWorld, rule: &Rule) -> String {
    let item = |id: logic::ItemKindId| {
        world
            .item_kind(id)
            .map(|k| format!("\"{}\"", k.name))
            .unwrap_or_else(|| format!("objet n° {}", id.raw()))
    };
    let items = |ids: &[logic::ItemKindId]| ids.iter().map(|id| item(*id)).collect::<Vec<_>>();
    match rule {
        Rule::True => "True".to_string(),
        Rule::False => "False".to_string(),
        Rule::Has { item: id, count: 1 } => format!("Has({})", item(*id)),
        Rule::Has { item: id, count } => format!("Has({}, {count})", item(*id)),
        Rule::HasAll(ids) => format!("HasAll({})", items(ids).join(", ")),
        Rule::HasAny(ids) => format!("HasAny({})", items(ids).join(", ")),
        Rule::HasAllCounts(pairs) => {
            let parts: Vec<String> = pairs
                .iter()
                .map(|(id, n)| format!("{}: {n}", item(*id)))
                .collect();
            format!("HasAllCounts({})", parts.join(", "))
        }
        Rule::HasGroup { group, count } => {
            let name = world
                .item_group(*group)
                .map(|g| g.name.clone())
                .unwrap_or_else(|| format!("groupe n° {}", group.raw()));
            format!("HasGroup(\"{name}\", {count})")
        }
        Rule::CanReachRegion(id) => {
            let name = world.region(*id).map(|r| r.name.as_str()).unwrap_or("?");
            format!("CanReachRegion(\"{name}\")")
        }
        Rule::CanReachLocation(id) => {
            let name = world.location(*id).map(|l| l.name.as_str()).unwrap_or("?");
            format!("CanReachLocation(\"{name}\")")
        }
        Rule::CanReachEntrance(id) => {
            let name = world.entrance(*id).map(|e| e.name.as_str()).unwrap_or("?");
            format!("CanReachEntrance(\"{name}\")")
        }
        Rule::And(children) => join(world, children, " & ", "True"),
        Rule::Or(children) => join(world, children, " | ", "False"),
    }
}

fn join(world: &MultiWorld, children: &[Rule], sep: &str, empty: &str) -> String {
    if children.is_empty() {
        return empty.to_string();
    }
    let parts: Vec<String> = children.iter().map(|c| describe(world, c)).collect();
    format!("({})", parts.join(sep))
}
