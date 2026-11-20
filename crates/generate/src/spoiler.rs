//! Spoiler du moteur Rust : le fichier `AP_<seed>_Spoiler.txt` rangé dans le zip de la seed.
//!
//! Le format s'inspire de celui d'Archipelago 0.6.8 (`Spoiler.to_file` dans
//! `BaseClasses.py`) : en-tête, options de chaque joueur, placement de chaque lieu, puis
//! le parcours par sphères. Les titres des sections restent en anglais, comme chez
//! Archipelago, pour qu'un joueur habitué s'y retrouve.
//!
//! Écarts voulus :
//! - l'en-tête nomme Raftup comme générateur (ADR 0027, § 3), avec la version
//!   d'Archipelago dont les mondes sont reproduits ;
//! - le parcours liste **tous** les objets de progression de chaque sphère ; Archipelago
//!   le réduit au plus court chemin vers la victoire ;
//! - pas de section « Paths » ni « Entrances » (aucun mélange d'entrées pour l'instant).

use logic::{sweep, ItemKindId, LocationId, MultiWorld, SlotId, State};

use crate::GenerateRequest;

/// Largeur de la colonne des noms, comme Archipelago (`'%s:'.ljust(33)` environ).
const LABEL_WIDTH: usize = 33;

/// Un joueur, tel qu'écrit dans le spoiler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpoilerPlayer {
    pub name: String,
    pub game: String,
    /// Options lisibles (nom, valeur), dans l'ordre du monde.
    pub options: Vec<(String, String)>,
}

/// Une ligne du parcours : le lieu (absent pour l'inventaire de départ) et l'objet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpoilerStep {
    pub location: Option<String>,
    pub item: String,
}

/// Tout ce que contient le spoiler, avant mise en texte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spoiler {
    /// Nom de la seed (20 chiffres).
    pub seed_name: String,
    pub players: Vec<SpoilerPlayer>,
    /// (lieu, objet) pour chaque lieu à numéro réseau, dans l'ordre de création.
    pub locations: Vec<(String, String)>,
    /// Sphère 0 = objets de progression de départ ; sphère n = objets de progression
    /// trouvés dans les lieux qui s'ouvrent au n-ième tour du balayage.
    pub playthrough: Vec<Vec<SpoilerStep>>,
}

/// Nom du générateur écrit en tête du spoiler.
pub fn generator_label() -> String {
    format!(
        "Raftup {} (moteur Rust), compatible avec Archipelago {}.{}.{}",
        env!("CARGO_PKG_VERSION"),
        crate::multidata_build::GENERATOR_VERSION.major,
        crate::multidata_build::GENERATOR_VERSION.minor,
        crate::multidata_build::GENERATOR_VERSION.build,
    )
}

/// Nom d'un élément suivi du joueur entre parenthèses, quand il y a plusieurs joueurs
/// (même règle qu'Archipelago).
fn with_player(world: &MultiWorld, name: &str, slot: SlotId) -> String {
    if world.slots().len() > 1 {
        let player = world.slot(slot).map(|s| s.name.as_str()).unwrap_or("?");
        format!("{name} ({player})")
    } else {
        name.to_string()
    }
}

fn item_name(world: &MultiWorld, item: ItemKindId) -> String {
    match world.item_kind(item) {
        Some(kind) => with_player(world, &kind.name, kind.slot),
        None => "?".to_string(),
    }
}

fn location_name(world: &MultiWorld, location: LocationId) -> String {
    match world.location(location) {
        Some(loc) => with_player(world, &loc.name, loc.slot),
        None => "?".to_string(),
    }
}

fn is_progression(world: &MultiWorld, item: ItemKindId) -> bool {
    world
        .item_kind(item)
        .is_some_and(|kind| kind.classification.progression)
}

/// Construit le spoiler d'un monde **déjà rempli**.
///
/// `start_items` : objets de départ de chaque joueur (ordre des slots) ; `start` :
/// l'inventaire qu'ils forment, point de départ du balayage.
pub(crate) fn build(
    world: &MultiWorld,
    request: &GenerateRequest,
    seed_name: &str,
    start: &State,
    start_items: &[Vec<ItemKindId>],
) -> Spoiler {
    let players = request
        .players
        .iter()
        .map(|player| {
            let mut options = vec![(
                "Progression Balancing".to_string(),
                player.progression_balancing.to_string(),
            )];
            options.extend(player.options.spoiler_options());
            SpoilerPlayer {
                name: player.name.clone(),
                game: player.options.game().to_string(),
                options,
            }
        })
        .collect();

    let locations = world
        .location_ids()
        .filter_map(|id| {
            let location = world.location(id)?;
            // Les événements (sans numéro réseau) ne sont pas des lieux du jeu.
            location.address?;
            let item = location.item.map(|item| item_name(world, item));
            Some((
                location_name(world, id),
                item.unwrap_or_else(|| "(vide)".to_string()),
            ))
        })
        .collect();

    // Sphère 0 : la progression de départ. Ensuite, une sphère par tour du balayage.
    let mut playthrough = vec![start_items
        .iter()
        .flatten()
        .filter(|item| is_progression(world, **item))
        .map(|item| SpoilerStep {
            location: None,
            item: item_name(world, *item),
        })
        .collect::<Vec<_>>()];
    // `start.clone()` : `sweep` prend l'inventaire par valeur et le modifie ; on garde
    // l'original intact pour l'appelant.
    for sphere in sweep(world, start.clone()).spheres {
        let steps: Vec<SpoilerStep> = sphere
            .iter()
            .filter_map(|id| {
                let item = world.location(*id)?.item?;
                is_progression(world, item).then(|| SpoilerStep {
                    location: Some(location_name(world, *id)),
                    item: item_name(world, item),
                })
            })
            .collect();
        // Un tour sans progression n'apprend rien au joueur : on ne l'écrit pas.
        if !steps.is_empty() {
            playthrough.push(steps);
        }
    }

    Spoiler {
        seed_name: seed_name.to_string(),
        players,
        locations,
        playthrough,
    }
}

impl Spoiler {
    /// Le texte du fichier. `playthrough` : écrire aussi le parcours par sphères
    /// (niveau de spoiler 2 ou plus chez Archipelago).
    pub fn to_text(&self, playthrough: bool) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "{}  -  Seed: {}\n\n",
            generator_label(),
            self.seed_name
        ));
        out.push_str(&label_line("Filling Algorithm", "raftup"));
        out.push_str(&label_line("Players", &self.players.len().to_string()));
        for (index, player) in self.players.iter().enumerate() {
            out.push_str(&format!("\nPlayer {}: {}\n", index + 1, player.name));
            out.push_str(&label_line("Game", &player.game));
            for (name, value) in &player.options {
                out.push_str(&label_line(name, value));
            }
        }

        out.push_str("\n\nLocations:\n\n");
        for (location, item) in &self.locations {
            out.push_str(&format!("{location}: {item}\n"));
        }

        if playthrough {
            out.push_str("\n\nPlaythrough:\n\n");
            for (number, sphere) in self.playthrough.iter().enumerate() {
                // Sphère 0 vide (aucune progression de départ) : rien à écrire.
                if sphere.is_empty() {
                    continue;
                }
                out.push_str(&format!("{number}: {{\n"));
                for step in sphere {
                    match &step.location {
                        Some(location) => out.push_str(&format!("  {location}: {}\n", step.item)),
                        None => out.push_str(&format!("  {}\n", step.item)),
                    }
                }
                out.push_str("}\n\n");
            }
        }
        out
    }
}

/// « Nom: » complété d'espaces jusqu'à la colonne des valeurs.
fn label_line(name: &str, value: &str) -> String {
    let label = format!("{name}:");
    format!("{label:<LABEL_WIDTH$}{value}\n")
}
