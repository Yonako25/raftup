//! Conversion assistée d'un apworld en monde natif (ADR 0042).
//!
//! On part de l'export JSON d'un joueur (ADR 0028, [`crate::imported::Export`]) et on écrit
//! le texte d'un `world.toml` (ADR 0041). L'export est pris pour **un seul** jeu d'options :
//! le monde écrit est un instantané fidèle à ces options-là. Les options du jeu sont
//! recopiées **en commentaire**, et tout ce que l'outil a dû supposer est listé en tête du
//! fichier (« À vérifier »), pour que l'auteur finisse le travail à la main.
//!
//! La sortie est déterministe (même export ⇒ même texte) et suit l'ordre de création des
//! éléments dans l'apworld quand l'export le donne (`n`) : deux conversions avec des
//! options différentes se comparent avec un simple diff.
//!
//! Le texte produit est relu par [`worlds_native::parse_world`] avant d'être rendu : on ne
//! rend jamais un fichier que Raftup refuserait.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde_json::Value;
use thiserror::Error;

use crate::imported::{Export, ItemExport, LocationExport, OptionExport, PlayerExport, RuleExport};

/// Nombre maximal de combinaisons pour déplier « au moins N parmi… » (comme l'import).
pub const MAX_COMBINATIONS: usize = 256;

/// Le résultat d'une conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    /// Texte du `world.toml`.
    pub toml: String,
    /// Points à vérifier à la main (aussi écrits en tête du fichier).
    pub notes: Vec<String>,
}

/// Erreurs de la conversion.
#[derive(Debug, Error)]
pub enum ConvertError {
    #[error("aucun joueur n° {0} dans l'export")]
    UnknownPlayer(u32),
    #[error("datapackage du jeu « {0} » absent de l'export")]
    NoPackage(String),
    /// Ce que le format 1 ne sait pas écrire : rien n'est produit.
    #[error("conversion impossible : {0}")]
    Unsupported(String),
    /// Le texte produit est refusé par `worlds-native` (bug du convertisseur).
    #[error("monde produit illisible : {0}")]
    Invalid(String),
}

fn unsupported(message: impl Into<String>) -> ConvertError {
    ConvertError::Unsupported(message.into())
}

/// Convertit le joueur `player` de l'export en monde natif.
///
/// `slot_data` : le `slot_data` de ce joueur dans la seed de la même génération (objet
/// JSON), recopié en valeurs littérales ; `None` : pas de `[[slot_data]]`.
pub fn convert(
    export: &Export,
    player: u32,
    slot_data: Option<&Value>,
) -> Result<Conversion, ConvertError> {
    let p = export
        .players
        .iter()
        .find(|p| p.slot == player)
        .ok_or(ConvertError::UnknownPlayer(player))?;
    let package = export
        .datapackage
        .get(&p.game)
        .ok_or_else(|| ConvertError::NoPackage(p.game.clone()))?;
    if p.custom_collect {
        return Err(unsupported(
            "le monde redéfinit collect / remove : ses objets ne se comptent pas par leur nom",
        ));
    }
    let mut notes = Vec::new();

    // ---- Lieux et événements, dans l'ordre de création ----
    let locations = in_creation_order(&p.locations, |l| l.n);
    let mut events: Vec<&LocationExport> = Vec::new();
    // Lieux à numéro : vides (remplis par le pool) ou posés d'avance (format 3, ADR 0054).
    let mut real: Vec<&LocationExport> = Vec::new();
    let mut placed = 0usize;
    for location in locations {
        match (location.address, &location.item) {
            (Some(_), None) => real.push(location),
            (Some(_), Some(item)) if item.player == player && item.code.is_some() => {
                placed += 1;
                real.push(location)
            }
            (Some(_), Some(item)) => {
                let kind = if item.code.is_none() {
                    "sans numéro"
                } else {
                    "d'un autre joueur"
                };
                return Err(unsupported(format!(
                    "le lieu « {} » a un numéro et porte « {} », un objet {kind} : le format \
                     ne pose d'avance qu'un objet à numéro du même joueur",
                    location.name, item.name
                )));
            }
            (None, Some(item)) if item.code.is_none() && item.player == player => {
                events.push(location)
            }
            (None, _) => {
                return Err(unsupported(format!(
                    "le lieu « {} » n'a pas de numéro et ne porte pas d'objet d'événement",
                    location.name
                )))
            }
        }
    }
    // Un objet d'événement posé par plusieurs événements (« Note Read » ×30) demande le
    // format 2 ; sinon on écrit le format 1, lu par plus de versions de Raftup.
    let mut event_items = BTreeSet::new();
    let mut shared_events = false;
    // Un événement qui donne un objet portant le nom d'un objet à numéro (Super Mario Land
    // 2) : Archipelago compte les deux ensemble, par leur nom ; le format 3 l'écrit tel quel.
    let mut numbered_events = false;
    for event in &events {
        let item = event.item.as_ref().map(|i| i.name.as_str()).unwrap_or("");
        if package.item_name_to_id.contains_key(item) {
            numbered_events = true;
            continue;
        }
        if !event_items.insert(item) {
            shared_events = true;
            continue;
        }
        if let Some(i) = &event.item {
            if i.classification & 1 == 0 {
                notes.push(format!(
                    "l'objet d'événement « {item} » n'est pas de progression dans l'apworld ; \
                     un événement natif l'est toujours"
                ));
            }
        }
    }

    // ---- Classification des objets : prise sur les exemplaires du joueur ----
    // Les objets posés d'avance comptent aussi : un objet qui n'existe que posé (« E1M1 -
    // Complete ») garde sa vraie classification au lieu de `filler`.
    // Même chose pour un événement qui donne un objet à numéro : la logique doit le
    // compter, il faut donc qu'il reste une progression.
    let preplaced = real
        .iter()
        .chain(events.iter())
        .filter_map(|l| l.item.as_ref())
        .filter(|i| i.code.is_some() || package.item_name_to_id.contains_key(&i.name));
    let mine: Vec<&ItemExport> = export
        .pool
        .iter()
        .chain(p.precollected.iter())
        .chain(preplaced)
        .filter(|i| i.player == player)
        .collect();
    let mut classes: BTreeMap<&str, u32> = BTreeMap::new();
    let mut mixed = BTreeSet::new();
    for item in &mine {
        // `entry(...).or_insert(...)` rend une référence modifiable vers la valeur rangée :
        // on y ajoute les drapeaux de chaque exemplaire (union des bits).
        let bits = classes
            .entry(item.name.as_str())
            .or_insert(item.classification);
        if *bits != item.classification {
            mixed.insert(item.name.as_str());
        }
        *bits |= item.classification;
    }
    // Le format n'a qu'une classification par objet. Garder l'union des drapeaux est sûr :
    // si un seul exemplaire fait progresser, tous sont placés selon la logique.
    for name in &mixed {
        notes.push(format!(
            "les exemplaires de « {name} » n'ont pas tous la même classification : \
             l'union de leurs drapeaux est gardée"
        ));
    }
    let unknown_class: Vec<&str> = package
        .item_name_to_id
        .keys()
        .map(String::as_str)
        .filter(|n| !classes.contains_key(n))
        .collect();
    if !unknown_class.is_empty() {
        notes.push(format!(
            "classification inconnue (objet ni dans le pool ni au départ, écrit « filler ») : {}",
            unknown_class.join(", ")
        ));
    }

    // ---- Règles ----
    let groups: Vec<(&str, BTreeSet<&str>)> = package
        .item_name_groups
        .iter()
        .filter(|(name, _)| name.as_str() != "Everything")
        .map(|(name, members)| (name.as_str(), members.iter().map(String::as_str).collect()))
        .collect();
    // Objets que le monde déclare (datapackage et objets d'événement). Une règle peut en
    // citer un autre, jamais créé (« CANNOT ACCESS » de Celeste 64, pour un passage
    // fermé) : Archipelago le compte à zéro, la conversion l'écrit `false`.
    let mut known: BTreeSet<&str> = package.item_name_to_id.keys().map(String::as_str).collect();
    known.extend(
        events
            .iter()
            .filter_map(|e| e.item.as_ref())
            .map(|i| i.name.as_str()),
    );
    let mut cited = Vec::new();
    p.goal.items(&mut cited);
    for entrance in &p.entrances {
        entrance.rule.items(&mut cited);
    }
    for location in &p.locations {
        location.rule.items(&mut cited);
    }
    let never: BTreeSet<&str> = cited
        .iter()
        .filter(|(who, item)| *who == player && !known.contains(item.as_str()))
        .map(|(_, item)| item.as_str())
        .collect();
    if !never.is_empty() {
        notes.push(format!(
            "objets cités par les règles mais jamais créés, écrits « false » : {}",
            never.iter().copied().collect::<Vec<_>>().join(", ")
        ));
    }
    let rules = Rules {
        player,
        groups: &groups,
        known: &known,
    };

    let mut out = String::new();
    // La plus petite version qui suffit : lue par plus de versions de Raftup.
    let format_version = if placed > 0 || numbered_events {
        3
    } else if shared_events {
        2
    } else {
        1
    };
    header(&mut out, p, format_version);
    let head_len = out.len();

    // ---- Entrées (on garde celles qui mènent quelque part) ----
    let mut entrances = Vec::new();
    for entrance in in_creation_order(&p.entrances, |e| e.n) {
        match &entrance.to {
            Some(to) => entrances.push((entrance, to.as_str())),
            None => notes.push(format!(
                "l'entrée « {} » n'est reliée à rien : ignorée",
                entrance.name
            )),
        }
    }
    unique(entrances.iter().map(|(e, _)| e.name.as_str()), "entrée")?;

    // ---- Autres points à vérifier ----
    // Un lieu posé d'avance n'a que faire de sa règle d'objet : le placement n'y va pas.
    if real
        .iter()
        .any(|l| l.item_rule.is_some() && l.item.is_none())
    {
        let names: Vec<&str> = real
            .iter()
            .filter(|l| l.item_rule.is_some() && l.item.is_none())
            .map(|l| l.name.as_str())
            .collect();
        notes.push(format!(
            "règles d'objet non converties (format 1), elles viennent peut-être du YAML \
             (local_items) : {}",
            names.join(", ")
        ));
    }
    if !p.early_items.is_empty() || !p.local_early_items.is_empty() {
        notes.push("objets à placer tôt ignorés : c'est une option du YAML".into());
    }
    if p.item_rule_error.is_some() {
        notes.push("une règle d'objet n'a pas pu être lue par l'export".into());
    }
    for item in &mine {
        if item.classification & !0b1111 != 0 {
            notes.push(format!(
                "drapeaux de classification inconnus sur « {} » : ignorés",
                item.name
            ));
            break;
        }
    }

    // ---- Pool : la fin faite d'un seul objet de remplissage devient [filler] ----
    let pool: Vec<&str> = export
        .pool
        .iter()
        .filter(|i| i.player == player)
        .map(|i| i.name.as_str())
        .collect();
    let to_fill = real.len() - placed;
    if pool.len() != to_fill {
        return Err(unsupported(format!(
            "{} objets dans le pool du joueur pour {to_fill} lieux à remplir : le format \
             remplit les lieux du joueur avec son propre pool",
            pool.len()
        )));
    }
    if placed > 0 {
        notes.push(format!(
            "{placed} objet(s) posé(s) d'avance (item = …) : vérifier qu'ils ne dépendent ni \
             des options ni du hasard de la génération"
        ));
    }
    let mut fixed = pool.len();
    let mut filler = None;
    if let Some(last) = pool.last() {
        if classes.get(last).copied() == Some(0) {
            while fixed > 0 && pool[fixed - 1] == *last {
                fixed -= 1;
            }
            filler = Some(*last);
            notes.push(format!(
                "fin du pool (« {last} » × {}) écrite en [filler] ; si le monde tire des \
                 pièges au hasard, ajouter trap et trap_percent",
                pool.len() - fixed
            ));
        }
    }

    // ---- Écriture ----
    let start_region = p.origin.as_str();
    out.push_str("[game]\n");
    let _ = writeln!(out, "name = {}", toml_str(&p.game));
    let _ = writeln!(out, "start_region = {}", toml_str(start_region));
    let _ = writeln!(out, "goal = {}", rule_str(&rules.text(&p.goal)?));
    out.push('\n');

    options_skeleton(&mut out, &p.options, &mut notes);

    out.push_str("# ---- Objets ----\n\n");
    for (name, id) in &package.item_name_to_id {
        out.push_str("[[items]]\n");
        let _ = writeln!(out, "name = {}", toml_str(name));
        let _ = writeln!(out, "id = {id}");
        let class = classes.get(name.as_str()).copied().unwrap_or(0);
        let _ = writeln!(out, "classification = {}", toml_str(&classification(class)));
        out.push('\n');
    }
    for (name, _) in &groups {
        let members = &package.item_name_groups[*name];
        out.push_str("[[item_groups]]\n");
        let _ = writeln!(out, "name = {}", toml_str(name));
        let _ = writeln!(
            out,
            "items = {}",
            toml_list(members.iter().map(String::as_str))
        );
        out.push('\n');
    }

    out.push_str("# ---- Régions ----\n\n");
    for region in in_creation_order_regions(p) {
        out.push_str("[[regions]]\n");
        let _ = writeln!(out, "name = {}", toml_str(region));
        out.push('\n');
    }

    out.push_str("# ---- Entrées ----\n\n");
    for (entrance, to) in &entrances {
        out.push_str("[[entrances]]\n");
        let _ = writeln!(out, "name = {}", toml_str(&entrance.name));
        let _ = writeln!(out, "from = {}", toml_str(&entrance.from));
        let _ = writeln!(out, "to = {}", toml_str(to));
        write_rule(&mut out, &rules.text(&entrance.rule)?);
        out.push('\n');
    }

    out.push_str("# ---- Lieux ----\n\n");
    for location in &real {
        out.push_str("[[locations]]\n");
        let _ = writeln!(out, "name = {}", toml_str(&location.name));
        let _ = writeln!(out, "id = {}", location.address.unwrap_or(0));
        let _ = writeln!(out, "region = {}", toml_str(&location.region));
        write_rule(&mut out, &rules.text(&location.rule)?);
        if let Some(item) = &location.item {
            let _ = writeln!(out, "item = {}", toml_str(&item.name));
        }
        match location.progress_type {
            1 => {}
            2 => out.push_str("progress = \"priority\"\n"),
            3 => out.push_str("progress = \"excluded\"\n"),
            other => {
                return Err(unsupported(format!(
                    "progress_type {other} inconnu sur « {} »",
                    location.name
                )))
            }
        }
        out.push('\n');
    }
    // Lieux du datapackage absents avec ces options : écrits quand même (le datapackage
    // ne dépend pas des options), jamais présents tant que l'auteur n'a pas mis le bon
    // `when`.
    let present: BTreeSet<&str> = real.iter().map(|l| l.name.as_str()).collect();
    let absent: Vec<(&String, &i64)> = package
        .location_name_to_id
        .iter()
        .filter(|(name, _)| !present.contains(name.as_str()))
        .collect();
    if !absent.is_empty() {
        let names: Vec<&str> = absent.iter().map(|(n, _)| n.as_str()).collect();
        notes.push(format!(
            "lieux absents avec ces options, écrits avec when = 'false' (région à corriger) : {}",
            names.join(", ")
        ));
        out.push_str("# Absents de l'instantané : gardés pour le datapackage.\n\n");
    }
    for (name, id) in absent {
        out.push_str("[[locations]]\n");
        let _ = writeln!(out, "name = {}", toml_str(name));
        let _ = writeln!(out, "id = {id}");
        let _ = writeln!(out, "region = {}", toml_str(start_region));
        out.push_str("when = 'false'\n\n");
    }
    for (name, members) in &package.location_name_groups {
        if name == "Everywhere" {
            continue;
        }
        out.push_str("[[location_groups]]\n");
        let _ = writeln!(out, "name = {}", toml_str(name));
        let _ = writeln!(
            out,
            "locations = {}",
            toml_list(members.iter().map(String::as_str))
        );
        out.push('\n');
    }

    out.push_str("# ---- Événements ----\n\n");
    for event in &events {
        out.push_str("[[events]]\n");
        let _ = writeln!(out, "name = {}", toml_str(&event.name));
        let _ = writeln!(out, "region = {}", toml_str(&event.region));
        write_rule(&mut out, &rules.text(&event.rule)?);
        let item = event.item.as_ref().map(|i| i.name.as_str()).unwrap_or("");
        let _ = writeln!(out, "item = {}", toml_str(item));
        out.push('\n');
    }

    out.push_str("# ---- Pool ----\n\n");
    for (name, count) in runs(&pool[..fixed]) {
        write_amount(&mut out, "pool", name, count);
    }
    if let Some(item) = filler {
        out.push_str("[filler]\n");
        let _ = writeln!(out, "item = {}\n", toml_str(item));
    }

    let start: Vec<&str> = p
        .precollected
        .iter()
        .filter(|i| i.player == player)
        .map(|i| i.name.as_str())
        .collect();
    if !start.is_empty() {
        out.push_str("# ---- Inventaire de départ ----\n\n");
        for (name, count) in runs(&start) {
            write_amount(&mut out, "start", name, count);
        }
    }

    if let Some(data) = slot_data {
        let Value::Object(map) = data else {
            return Err(unsupported("le slot_data n'est pas un objet JSON"));
        };
        out.push_str("# ---- slot_data (valeurs de l'instantané) ----\n\n");
        for (key, value) in map {
            match toml_value(value) {
                Some(text) => {
                    out.push_str("[[slot_data]]\n");
                    let _ = writeln!(out, "key = {}", toml_str(key));
                    let _ = writeln!(out, "value = {text}\n");
                }
                None => notes.push(format!(
                    "slot_data « {key} » : valeur null, que TOML ne sait pas écrire (ignorée)"
                )),
            }
        }
        notes.push("slot_data recopié en valeurs littérales : à relier aux options".into());
    }

    // Les points à vérifier vont en tête, juste après l'en-tête.
    let mut toml = String::new();
    let (head, body) = out.split_at(head_len);
    toml.push_str(head);
    if !notes.is_empty() {
        toml.push_str("# À vérifier :\n");
        for note in &notes {
            let _ = writeln!(toml, "# - {}", note.replace('\n', " "));
        }
        toml.push('\n');
    }
    toml.push_str(body);

    worlds_native::parse_world(&toml).map_err(|e| ConvertError::Invalid(e.to_string()))?;
    Ok(Conversion { toml, notes })
}

/// En-tête du fichier : d'où il vient, et pour quelles options.
fn header(out: &mut String, p: &PlayerExport, format_version: u32) {
    let _ = writeln!(
        out,
        "# {} au format de monde natif (format_version {format_version}), converti par Raftup \
         (ADR 0042).",
        p.game.replace('\n', " ")
    );
    out.push_str(
        "#\n# Instantané d'un seul jeu d'options : le monde ne dépend pas encore des options.\n\
         # Pour les rendre réglables, décommenter les [[options]] plus bas et écrire les `when`\n\
         # (comparer deux conversions avec des options différentes montre lesquels).\n",
    );
    let values: Vec<String> = p
        .options
        .iter()
        .filter_map(|o| o.value.map(|v| format!("{} = {v}", o.name)))
        .collect();
    if !values.is_empty() {
        let _ = writeln!(out, "# Options de l'instantané : {}.", values.join(", "));
    }
    let _ = write!(out, "\nformat_version = {format_version}\n\n");
}

/// Options du monde, recopiées en commentaire (squelette à décommenter).
fn options_skeleton(out: &mut String, options: &[OptionExport], notes: &mut Vec<String>) {
    if options.is_empty() {
        return;
    }
    out.push_str("# ---- Options de l'apworld (en commentaire : voir l'en-tête) ----\n#\n");
    // Options que chaque monde déclare, mais qu'Archipelago applique lui-même avant
    // l'export (le pool reçu en tient déjà compte) : ce ne sont pas des options du jeu.
    const SHARED: [&str; 1] = ["start_inventory_from_pool"];
    for o in options
        .iter()
        .filter(|o| !SHARED.contains(&o.name.as_str()))
    {
        let mut lines = vec![
            "[[options]]".to_string(),
            format!("name = {}", toml_str(&o.name)),
        ];
        if !o.display.is_empty() && o.display != o.name {
            lines.push(format!("display = {}", toml_str(&o.display)));
        }
        let default = o.default.unwrap_or(0);
        match o.kind.as_str() {
            "toggle" => {
                lines.push("type = \"toggle\"".into());
                lines.push(format!("default = {}", default != 0));
            }
            "range" => {
                lines.push("type = \"range\"".into());
                lines.push(format!("min = {}", o.min.unwrap_or(0)));
                lines.push(format!("max = {}", o.max.unwrap_or(0)));
                lines.push(format!("default = {default}"));
                if !o.special.is_empty() {
                    notes.push(format!(
                        "option « {} » : noms spéciaux non pris en charge ({})",
                        o.name,
                        o.special.join(", ")
                    ));
                }
            }
            "choice" => {
                lines.push("type = \"choice\"".into());
                let contiguous = o
                    .choices
                    .iter()
                    .enumerate()
                    .all(|(i, (v, _))| *v == i as i64);
                if !contiguous {
                    notes.push(format!(
                        "option « {} » : choix aux valeurs non consécutives, à renuméroter",
                        o.name
                    ));
                }
                lines.push(format!(
                    "choices = {}",
                    toml_list(o.choices.iter().map(|(_, n)| n.as_str()))
                ));
                let by_value: BTreeMap<i64, &str> =
                    o.choices.iter().map(|(v, n)| (*v, n.as_str())).collect();
                let aliases: Vec<String> = o
                    .aliases
                    .iter()
                    .filter_map(|(a, v)| {
                        by_value
                            .get(v)
                            .map(|n| format!("{} = {}", toml_key(a), toml_str(n)))
                    })
                    .collect();
                if !aliases.is_empty() {
                    lines.push(format!("aliases = {{ {} }}", aliases.join(", ")));
                }
                if let Some(name) = by_value.get(&default) {
                    lines.push(format!("default = {}", toml_str(name)));
                }
            }
            _ => {
                notes.push(format!(
                    "option « {} » d'un type que le format 1 ne connaît pas",
                    o.name
                ));
                continue;
            }
        }
        if let Some(value) = o.value {
            lines.push(format!("# valeur de l'instantané : {value}"));
        }
        for line in lines {
            let _ = writeln!(out, "# {line}");
        }
        out.push_str("#\n");
    }
    out.push('\n');
}

/// Éléments rangés par ordre de création (`n`), l'ordre de l'export départageant ; sans
/// `n`, l'ordre de l'export.
fn in_creation_order<T>(list: &[T], n: impl Fn(&T) -> Option<u64>) -> Vec<&T> {
    let mut sorted: Vec<(u64, usize, &T)> = list
        .iter()
        .enumerate()
        .map(|(i, x)| (n(x).unwrap_or(u64::MAX), i, x))
        .collect();
    sorted.sort_by_key(|(n, i, _)| (*n, *i));
    sorted.into_iter().map(|(_, _, x)| x).collect()
}

fn in_creation_order_regions(p: &PlayerExport) -> Vec<&str> {
    let mut sorted: Vec<(u64, usize, &str)> = p
        .regions
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let n = p.region_n.get(i).copied().flatten().unwrap_or(u64::MAX);
            (n, i, r.as_str())
        })
        .collect();
    sorted.sort_by_key(|(n, i, _)| (*n, *i));
    sorted.into_iter().map(|(_, _, r)| r).collect()
}

fn unique<'a>(names: impl Iterator<Item = &'a str>, what: &str) -> Result<(), ConvertError> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(unsupported(format!("{what} « {name} » en double")));
        }
    }
    Ok(())
}

/// Copies consécutives regroupées : `[a, a, b]` → `[(a, 2), (b, 1)]`.
fn runs<'a>(names: &[&'a str]) -> Vec<(&'a str, usize)> {
    let mut out: Vec<(&str, usize)> = Vec::new();
    for name in names {
        match out.last_mut() {
            Some((last, count)) if last == name => *count += 1,
            _ => out.push((name, 1)),
        }
    }
    out
}

fn write_amount(out: &mut String, table: &str, item: &str, count: usize) {
    let _ = writeln!(out, "[[{table}]]");
    let _ = writeln!(out, "item = {}", toml_str(item));
    if count != 1 {
        let _ = writeln!(out, "count = {count}");
    }
    out.push('\n');
}

fn write_rule(out: &mut String, rule: &str) {
    if rule != "true" {
        let _ = writeln!(out, "rule = {}", rule_str(rule));
    }
}

/// Classification d'Archipelago (drapeaux) → texte du format natif.
fn classification(bits: u32) -> String {
    let names = [
        (0b0001, "progression"),
        (0b0010, "useful"),
        (0b0100, "trap"),
        (0b1000, "skip_balancing"),
    ];
    let parts: Vec<&str> = names
        .iter()
        .filter(|(bit, _)| bits & bit != 0)
        .map(|(_, name)| *name)
        .collect();
    if parts.is_empty() {
        "filler".into()
    } else {
        parts.join("+")
    }
}

// ---- Règles : RuleExport → texte du mini-langage ----

/// Une règle en cours d'écriture : un terme, ou un « et » / « ou » de règles.
enum Text {
    Atom(String),
    And(Vec<Text>),
    Or(Vec<Text>),
}

impl Text {
    fn render(&self, inside_and: bool) -> String {
        match self {
            Text::Atom(s) => s.clone(),
            Text::And(parts) if parts.is_empty() => "true".into(),
            Text::Or(parts) if parts.is_empty() => "false".into(),
            Text::And(parts) if parts.len() == 1 => parts[0].render(inside_and),
            Text::Or(parts) if parts.len() == 1 => parts[0].render(inside_and),
            Text::And(parts) => parts
                .iter()
                .map(|p| p.render(true))
                .collect::<Vec<_>>()
                .join(" and "),
            Text::Or(parts) => {
                let text = parts
                    .iter()
                    .map(|p| p.render(false))
                    .collect::<Vec<_>>()
                    .join(" or ");
                // `and` est plus prioritaire que `or` : un « ou » dans un « et » se
                // met entre parenthèses.
                if inside_and {
                    format!("({text})")
                } else {
                    text
                }
            }
        }
    }
}

struct Rules<'a> {
    player: u32,
    /// Groupes d'objets déclarés (sans « Everything »), pour `has_from_list`.
    groups: &'a [(&'a str, BTreeSet<&'a str>)],
    /// Objets déclarés par le monde ; un autre nom vaut « jamais possédé ».
    known: &'a BTreeSet<&'a str>,
}

impl Rules<'_> {
    fn text(&self, rule: &RuleExport) -> Result<String, ConvertError> {
        Ok(self.tree(rule)?.render(false))
    }

    fn same_player(&self, player: u32) -> Result<(), ConvertError> {
        if player == self.player {
            Ok(())
        } else {
            Err(unsupported(format!(
                "une règle lit l'inventaire du joueur {player} : le format 1 ne décrit qu'un monde"
            )))
        }
    }

    /// `has(item, count)`, ou `false` pour un objet jamais créé (compté à zéro).
    fn has(&self, item: &str, count: u32) -> String {
        if count > 0 && !self.known.contains(item) {
            "false".into()
        } else {
            has(item, count)
        }
    }

    /// Les objets déclarés de la liste (on retire ceux qui ne seront jamais possédés).
    fn known_of<'b>(&self, items: &'b [String]) -> Vec<&'b str> {
        items
            .iter()
            .map(String::as_str)
            .filter(|i| self.known.contains(i))
            .collect()
    }

    fn tree(&self, rule: &RuleExport) -> Result<Text, ConvertError> {
        let atom = |s: String| Ok(Text::Atom(s));
        match rule {
            RuleExport::True => atom("true".into()),
            RuleExport::False => atom("false".into()),
            RuleExport::Opaque { why } => Err(unsupported(format!("règle opaque ({why})"))),
            RuleExport::And { of } => Ok(Text::And(self.all(of)?)),
            RuleExport::Or { of } => Ok(Text::Or(self.all(of)?)),
            RuleExport::AtLeast { count, of } => {
                let parts = self.all(of)?;
                at_least(*count as usize, parts)
            }
            RuleExport::Has {
                player,
                item,
                count,
            } => {
                self.same_player(*player)?;
                atom(self.has(item, *count))
            }
            // Liste vide : le format refuse `has_all()` et `has_any()`. Comme en Python,
            // « tous parmi rien » est vrai et « un parmi rien » est faux (Mega Man 2 et 3).
            RuleExport::HasAll { player, items } if items.is_empty() => {
                self.same_player(*player)?;
                atom("true".into())
            }
            RuleExport::HasAny { player, items } if items.is_empty() => {
                self.same_player(*player)?;
                atom("false".into())
            }
            RuleExport::HasAll { player, items } => {
                self.same_player(*player)?;
                if self.known_of(items).len() < items.len() {
                    return atom("false".into());
                }
                atom(call("has_all", items.iter().map(String::as_str)))
            }
            RuleExport::HasAny { player, items } => {
                self.same_player(*player)?;
                let known = self.known_of(items);
                if known.is_empty() {
                    return atom("false".into());
                }
                atom(call("has_any", known.into_iter()))
            }
            RuleExport::HasAllCounts { player, items } => {
                self.same_player(*player)?;
                Ok(Text::And(
                    items
                        .iter()
                        .map(|(i, n)| Text::Atom(self.has(i, *n)))
                        .collect(),
                ))
            }
            RuleExport::HasAnyCount { player, items } => {
                self.same_player(*player)?;
                Ok(Text::Or(
                    items
                        .iter()
                        .map(|(i, n)| Text::Atom(self.has(i, *n)))
                        .collect(),
                ))
            }
            RuleExport::HasFromList {
                player,
                items,
                count,
            } => {
                self.same_player(*player)?;
                let set: BTreeSet<&str> = items.iter().map(String::as_str).collect();
                if let Some((group, _)) = self.groups.iter().find(|(_, m)| *m == set) {
                    let mut text = format!("has_group({}", name(group));
                    if *count != 1 {
                        let _ = write!(text, ", {count}");
                    }
                    text.push(')');
                    return atom(text);
                }
                match count {
                    0 => atom("true".into()),
                    1 => match self.known_of(items) {
                        known if known.is_empty() => atom("false".into()),
                        known => atom(call("has_any", known.into_iter())),
                    },
                    _ => Err(unsupported(format!(
                        "somme d'exemplaires sur une liste qui n'est pas un groupe déclaré \
                         ({count} parmi {})",
                        items.join(", ")
                    ))),
                }
            }
            RuleExport::HasFromListUnique {
                player,
                items,
                count,
            } => {
                self.same_player(*player)?;
                let parts = items.iter().map(|i| Text::Atom(has(i, 1))).collect();
                at_least(*count as usize, parts)
            }
            RuleExport::CanReachLocation { player, location } => {
                self.same_player(*player)?;
                atom(format!("can_reach_location({})", name(location)))
            }
            RuleExport::CanReachRegion { player, region } => {
                self.same_player(*player)?;
                atom(format!("can_reach_region({})", name(region)))
            }
            RuleExport::CanReachEntrance { player, entrance } => {
                self.same_player(*player)?;
                atom(format!("can_reach_entrance({})", name(entrance)))
            }
        }
    }

    fn all(&self, rules: &[RuleExport]) -> Result<Vec<Text>, ConvertError> {
        rules.iter().map(|r| self.tree(r)).collect()
    }
}

fn has(item: &str, count: u32) -> String {
    match count {
        0 => "true".into(),
        1 => format!("has({})", name(item)),
        n => format!("has({}, {n})", name(item)),
    }
}

fn call<'a>(function: &str, names: impl Iterator<Item = &'a str>) -> String {
    let args: Vec<String> = names.map(name).collect();
    format!("{function}({})", args.join(", "))
}

/// « Au moins `count` des règles » : un « ou » des « et » de chaque combinaison.
fn at_least(count: usize, parts: Vec<Text>) -> Result<Text, ConvertError> {
    let n = parts.len();
    if count == 0 {
        return Ok(Text::Atom("true".into()));
    }
    if count > n {
        return Ok(Text::Atom("false".into()));
    }
    if count == n {
        return Ok(Text::And(parts));
    }
    // Les règles sont rendues une fois ; chaque combinaison recopie leur texte.
    let rendered: Vec<String> = parts.iter().map(|p| p.render(true)).collect();
    let mut combos = Vec::new();
    let mut chosen: Vec<usize> = (0..count).collect();
    loop {
        if combos.len() >= MAX_COMBINATIONS {
            return Err(unsupported(format!(
                "« au moins {count} parmi {n} » donne plus de {MAX_COMBINATIONS} combinaisons"
            )));
        }
        combos.push(Text::And(
            chosen
                .iter()
                .map(|i| Text::Atom(rendered[*i].clone()))
                .collect(),
        ));
        // Combinaison suivante, dans l'ordre lexicographique.
        let Some(i) = (0..count).rev().find(|&i| chosen[i] < n - count + i) else {
            break;
        };
        chosen[i] += 1;
        for j in i + 1..count {
            chosen[j] = chosen[j - 1] + 1;
        }
    }
    Ok(Text::Or(combos))
}

// ---- Écriture du TOML ----

const RESERVED: [&str; 5] = ["and", "or", "not", "true", "false"];

/// Un nom dans une règle : mot simple si possible, sinon entre guillemets.
fn name(s: &str) -> String {
    let mut chars = s.chars();
    let simple = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !RESERVED.contains(&s);
    if simple {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// Chaîne TOML « basique », échappée.
fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Une règle : chaîne littérale `'…'` si possible (positions d'erreur exactes).
fn rule_str(rule: &str) -> String {
    if rule.contains('\'') || rule.chars().any(char::is_control) {
        toml_str(rule)
    } else {
        format!("'{rule}'")
    }
}

/// Clé TOML : nue si possible, sinon entre guillemets.
fn toml_key(s: &str) -> String {
    let bare = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        s.to_string()
    } else {
        toml_str(s)
    }
}

fn toml_list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let parts: Vec<String> = items.map(toml_str).collect();
    format!("[{}]", parts.join(", "))
}

/// Valeur JSON → valeur TOML en ligne ; `None` pour `null` (inconnu en TOML).
fn toml_value(value: &Value) -> Option<String> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => toml_str(s),
        Value::Array(items) => {
            let parts: Option<Vec<String>> = items.iter().map(toml_value).collect();
            format!("[{}]", parts?.join(", "))
        }
        Value::Object(map) => {
            let parts: Option<Vec<String>> = map
                .iter()
                .map(|(k, v)| toml_value(v).map(|v| format!("{} = {v}", toml_key(k))))
                .collect();
            format!("{{ {} }}", parts?.join(", "))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noms_simples_ou_entre_guillemets() {
        assert_eq!(name("Sword"), "Sword");
        assert_eq!(name("Health Upgrade"), "\"Health Upgrade\"");
        assert_eq!(name("and"), "\"and\"");
        assert_eq!(name("1up"), "\"1up\"");
        assert_eq!(name("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn au_moins_deux_parmi_trois() {
        let parts = ["A", "B", "C"]
            .iter()
            .map(|s| Text::Atom(format!("has({s})")))
            .collect();
        let text = at_least(2, parts).unwrap().render(false);
        assert_eq!(
            text,
            "has(A) and has(B) or has(A) and has(C) or has(B) and has(C)"
        );
    }

    #[test]
    fn ou_dans_et_entre_parentheses() {
        let t = Text::And(vec![
            Text::Atom("has(A)".into()),
            Text::Or(vec![
                Text::Atom("has(B)".into()),
                Text::Atom("has(C)".into()),
            ]),
        ]);
        assert_eq!(t.render(false), "has(A) and (has(B) or has(C))");
    }

    #[test]
    fn valeurs_json_en_toml() {
        let v = serde_json::json!({"a b": [1, true, "x"], "c": {"d": 2.5}});
        assert_eq!(
            toml_value(&v).unwrap(),
            "{ \"a b\" = [1, true, \"x\"], c = { d = 2.5 } }"
        );
        assert_eq!(toml_value(&Value::Null), None);
    }

    #[test]
    fn classifications() {
        assert_eq!(classification(0), "filler");
        assert_eq!(classification(3), "progression+useful");
        assert_eq!(classification(9), "progression+skip_balancing");
    }
}
