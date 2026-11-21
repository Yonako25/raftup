//! Lecture sûre d'un monde natif : TOML → [`NativeWorld`] vérifié.
//!
//! Le fichier peut venir de n'importe où : on le lit comme une donnée hostile. Taille
//! bornée, nombre d'éléments plafonné, chaque nom vérifié, aucune panique. Chaque erreur
//! donne, quand c'est possible, la ligne et la colonne dans le fichier.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::ops::Range;
use std::path::Path;

use logic::{Classification, Progress};
use serde::Deserialize;
use toml::Spanned;

use crate::error::{LoadError, Position};
use crate::expr::{self, ExprError, Names};
use crate::model::{
    Amount, Choice, Count, EntranceDef, EventDef, Expr, Filler, GroupDef, ItemDef, ItemRef,
    LocationDef, LocationGroupDef, LocationRef, NativeWorld, OptionDef, OptionKind, RegionDef,
    SlotDataDef, SlotDataValue,
};
use crate::{
    FORMAT_VERSION, MAX_CHOICES, MAX_ELEMENTS, MAX_FILE_BYTES, MAX_NAME_CHARS, MAX_RULE_BYTES,
    MIN_FORMAT_VERSION,
};

// ---- Le format TOML, tel qu'il est écrit ----
//
// `deny_unknown_fields` : un champ mal orthographié est une erreur, pas un oubli silencieux.
// `Spanned<String>` garde, en plus du texte, sa place dans le fichier (pour les erreurs).

#[derive(Deserialize)]
struct Probe {
    format_version: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorld {
    format_version: i64,
    game: RawGame,
    #[serde(default)]
    options: Vec<RawOption>,
    #[serde(default)]
    items: Vec<RawItem>,
    #[serde(default)]
    item_groups: Vec<RawGroup>,
    #[serde(default)]
    regions: Vec<RawRegion>,
    #[serde(default)]
    entrances: Vec<RawEntrance>,
    #[serde(default)]
    locations: Vec<RawLocation>,
    #[serde(default)]
    location_groups: Vec<RawLocationGroup>,
    #[serde(default)]
    events: Vec<RawEvent>,
    #[serde(default)]
    pool: Vec<RawAmount>,
    filler: Option<RawFiller>,
    #[serde(default)]
    start: Vec<RawAmount>,
    #[serde(default)]
    slot_data: Vec<RawSlotData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGame {
    name: Spanned<String>,
    start_region: Option<Spanned<String>>,
    goal: Spanned<String>,
}

/// Valeur simple (défaut d'une option). `untagged` : serde essaie chaque forme dans
/// l'ordre et garde la première qui convient.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawScalar {
    Bool(bool),
    Int(i64),
    Text(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOption {
    name: Spanned<String>,
    display: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    default: Option<RawScalar>,
    min: Option<i64>,
    max: Option<i64>,
    choices: Option<Vec<String>>,
    aliases: Option<BTreeMap<String, String>>,
}

/// Une valeur fixe, ou une liste de cas `{ when, value }`.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawChoice {
    Plain(String),
    Cases(Vec<RawCase>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCase {
    when: Option<String>,
    value: String,
}

/// Un nombre : entier, ou texte `"option(nom)"`.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawNumber {
    Int(i64),
    Text(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItem {
    name: Spanned<String>,
    id: i64,
    classification: RawChoice,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGroup {
    name: Spanned<String>,
    items: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLocationGroup {
    name: Spanned<String>,
    locations: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRegion {
    name: Spanned<String>,
    when: Option<Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntrance {
    name: Spanned<String>,
    from: String,
    to: String,
    rule: Option<Spanned<String>>,
    when: Option<Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLocation {
    name: Spanned<String>,
    id: i64,
    region: RawChoice,
    rule: Option<Spanned<String>>,
    progress: Option<String>,
    when: Option<Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEvent {
    name: Spanned<String>,
    region: RawChoice,
    rule: Option<Spanned<String>>,
    item: String,
    when: Option<Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAmount {
    item: Spanned<String>,
    count: Option<RawNumber>,
    when: Option<Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFiller {
    item: Spanned<String>,
    trap: Option<String>,
    trap_percent: Option<RawNumber>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSlotData {
    key: Spanned<String>,
    option: Option<String>,
    value: Option<serde_json::Value>,
}

/// Options communes à tous les jeux d'Archipelago : un monde ne peut pas en déclarer une
/// du même nom (le YAML les lit à part).
const RESERVED_OPTIONS: &[&str] = &[
    "progression_balancing",
    "accessibility",
    "local_items",
    "non_local_items",
    "start_inventory",
    "start_inventory_from_pool",
    "start_hints",
    "start_location_hints",
    "exclude_locations",
    "priority_locations",
    "item_links",
    "plando_items",
    "plando_connections",
    "plando_texts",
];

// ---- Points d'entrée ----

/// Lit un monde natif depuis un fichier `.toml`, ou un dossier qui contient `world.toml`.
pub fn load_path(path: &Path) -> Result<NativeWorld, LoadError> {
    let file = if path.is_dir() {
        path.join("world.toml")
    } else {
        path.to_path_buf()
    };
    let io = |e: std::io::Error| LoadError::Io {
        path: file.display().to_string(),
        message: e.to_string(),
    };
    let handle = std::fs::File::open(&file).map_err(io)?;
    // On lit au plus une limite + 1 octet : assez pour savoir que c'est trop gros, sans
    // jamais charger un fichier énorme en mémoire.
    let mut bytes = Vec::new();
    handle
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(LoadError::TooLarge {
            size: bytes.len() as u64,
            max: MAX_FILE_BYTES,
        });
    }
    let text = String::from_utf8(bytes).map_err(|_| LoadError::Io {
        path: file.display().to_string(),
        message: "le fichier n'est pas en UTF-8".into(),
    })?;
    parse_world(&text)
}

/// Lit un monde natif depuis son texte TOML.
pub fn parse_world(text: &str) -> Result<NativeWorld, LoadError> {
    if text.len() > MAX_FILE_BYTES {
        return Err(LoadError::TooLarge {
            size: text.len() as u64,
            max: MAX_FILE_BYTES,
        });
    }
    // Premier passage : seulement la version, pour refuser un format inconnu avec un
    // message clair au lieu d'une liste de champs inconnus.
    let probe: Probe = toml::from_str(text).map_err(|e| toml_error(text, &e))?;
    match probe.format_version {
        Some(MIN_FORMAT_VERSION..=FORMAT_VERSION) => {}
        Some(found) => return Err(LoadError::Format { found }),
        None => {
            return Err(LoadError::Invalid {
                pos: None,
                message: "format_version manquant".into(),
            })
        }
    }
    let raw: RawWorld = toml::from_str(text).map_err(|e| toml_error(text, &e))?;
    Loader { text }.world(raw)
}

fn toml_error(text: &str, error: &toml::de::Error) -> LoadError {
    LoadError::Toml {
        pos: error.span().map(|span| Position::of(text, span.start)),
        message: error.message().trim().to_string(),
    }
}

/// Position d'un nom de choix (sans tenir compte de la casse), ou d'un alias.
pub(crate) fn choice_index(
    choices: &[String],
    aliases: &[(String, u32)],
    name: &str,
) -> Option<i64> {
    if let Some(index) = choices.iter().position(|c| c.eq_ignore_ascii_case(name)) {
        return i64::try_from(index).ok();
    }
    aliases
        .iter()
        .find(|(alias, _)| alias.eq_ignore_ascii_case(name))
        .map(|(_, index)| i64::from(*index))
}

/// `hard_mode` → `Hard Mode`, comme le nom affiché par défaut d'Archipelago.
pub(crate) fn title_case(name: &str) -> String {
    name.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---- La conversion ----

struct Loader<'t> {
    text: &'t str,
}

/// Nombre d'éléments en `u32` (les listes sont déjà plafonnées à [`MAX_ELEMENTS`]).
fn index(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

impl Loader<'_> {
    fn pos(&self, span: Range<usize>) -> Option<Position> {
        Some(Position::of(self.text, span.start))
    }

    fn invalid(&self, span: Range<usize>, message: impl Into<String>) -> LoadError {
        LoadError::Invalid {
            pos: self.pos(span),
            message: message.into(),
        }
    }

    /// Octet où commence le contenu d'une chaîne TOML dont `span` couvre les guillemets.
    fn content_start(&self, span: &Range<usize>) -> usize {
        let token = self.text.get(span.clone()).unwrap_or("");
        if token.starts_with("\"\"\"") || token.starts_with("'''") {
            // Chaîne sur plusieurs lignes : TOML retire le saut de ligne qui suit l'ouverture.
            let after = span.start + 3;
            let rest = self.text.get(after..).unwrap_or("");
            if rest.starts_with("\r\n") {
                after + 2
            } else if rest.starts_with('\n') {
                after + 1
            } else {
                after
            }
        } else if token.starts_with('"') || token.starts_with('\'') {
            span.start + 1
        } else {
            span.start
        }
    }

    /// Vérifie un nom : non vide, pas trop long, sans caractère de contrôle.
    fn check_name(&self, name: &Spanned<String>, what: &str) -> Result<(), LoadError> {
        let value = name.get_ref();
        let problem = if value.is_empty() {
            Some("vide".to_string())
        } else if value.chars().count() > MAX_NAME_CHARS {
            Some(format!("plus de {MAX_NAME_CHARS} caractères"))
        } else if value.chars().any(char::is_control) {
            Some("caractère de contrôle".to_string())
        } else {
            None
        };
        match problem {
            Some(problem) => {
                Err(self.invalid(name.span(), format!("{what} « {value} » : nom {problem}")))
            }
            None => Ok(()),
        }
    }

    /// Plafonne la taille d'une liste.
    fn check_len(&self, len: usize, what: &str) -> Result<(), LoadError> {
        if len > MAX_ELEMENTS {
            return Err(LoadError::Invalid {
                pos: None,
                message: format!("trop de {what} : {len} (au plus {MAX_ELEMENTS})"),
            });
        }
        Ok(())
    }

    /// Analyse une expression écrite dans une chaîne avec sa position.
    fn expr(
        &self,
        source: &Spanned<String>,
        names: &Names,
        context: &str,
    ) -> Result<Expr, LoadError> {
        let span = source.span();
        let base = self.content_start(&span);
        self.expr_text(source.get_ref(), names, context, Some(base), span.start)
    }

    /// Analyse une expression ; `base` : octet du début du texte dans le fichier, s'il est
    /// connu ; sinon les erreurs pointent sur `fallback`.
    fn expr_text(
        &self,
        text: &str,
        names: &Names,
        context: &str,
        base: Option<usize>,
        fallback: usize,
    ) -> Result<Expr, LoadError> {
        if text.len() > MAX_RULE_BYTES {
            return Err(LoadError::Invalid {
                pos: Some(Position::of(self.text, fallback)),
                message: format!("{context} : règle de plus de {MAX_RULE_BYTES} octets"),
            });
        }
        expr::parse(text, names).map_err(|e| self.expr_error(e, context, base, fallback))
    }

    fn expr_error(
        &self,
        e: ExprError,
        context: &str,
        base: Option<usize>,
        fallback: usize,
    ) -> LoadError {
        let offset = match base {
            Some(base) => base + e.offset,
            None => fallback,
        };
        LoadError::Invalid {
            pos: Some(Position::of(self.text, offset)),
            message: format!("{context} : {}", e.message),
        }
    }

    /// Analyse une condition `when` : elle doit être statique.
    fn when(
        &self,
        source: &Option<Spanned<String>>,
        names: &Names,
        context: &str,
    ) -> Result<Option<Expr>, LoadError> {
        let Some(source) = source else {
            return Ok(None);
        };
        let expr = self.expr(source, names, context)?;
        if !expr.is_static() {
            return Err(self.invalid(
                source.span(),
                format!("{context} : une condition « when » ne dépend que des options"),
            ));
        }
        Ok(Some(expr))
    }

    /// Règle d'accès facultative (`true` par défaut).
    fn rule(
        &self,
        source: &Option<Spanned<String>>,
        names: &Names,
        context: &str,
    ) -> Result<Expr, LoadError> {
        match source {
            Some(source) => self.expr(source, names, context),
            None => Ok(Expr::Const(true)),
        }
    }

    /// Convertit une valeur fixe ou une liste de cas. `convert` traduit chaque valeur.
    ///
    /// `impl Fn(&str) -> Result<T, String>` : n'importe quelle fonction (ou fermeture) qui
    /// prend un texte et rend une valeur ou un message d'erreur.
    fn choice<T>(
        &self,
        raw: &RawChoice,
        names: &Names,
        context: &str,
        at: Range<usize>,
        convert: impl Fn(&str) -> Result<T, String>,
    ) -> Result<Choice<T>, LoadError> {
        let wrap = |message: String| self.invalid(at.clone(), format!("{context} : {message}"));
        match raw {
            RawChoice::Plain(value) => Ok(Choice::fixed(convert(value).map_err(wrap)?)),
            RawChoice::Cases(cases) => {
                if cases.is_empty() {
                    return Err(wrap("liste de cas vide".into()));
                }
                self.check_len(cases.len(), "cas")?;
                let mut out = Vec::new();
                for (i, case) in cases.iter().enumerate() {
                    let last = i + 1 == cases.len();
                    let when = match (&case.when, last) {
                        (Some(_), true) => {
                            return Err(wrap("le dernier cas ne doit pas avoir de « when »".into()))
                        }
                        (None, false) => {
                            return Err(wrap(format!("le cas n° {} doit avoir un « when »", i + 1)))
                        }
                        (Some(text), false) => {
                            let expr = self.expr_text(text, names, context, None, at.start)?;
                            if !expr.is_static() {
                                return Err(wrap(
                                    "une condition « when » ne dépend que des options".into(),
                                ));
                            }
                            Some(expr)
                        }
                        (None, true) => None,
                    };
                    out.push((when, convert(&case.value).map_err(wrap)?));
                }
                Ok(Choice { cases: out })
            }
        }
    }

    fn count(
        &self,
        raw: Option<&RawNumber>,
        names: &Names,
        context: &str,
        at: Range<usize>,
    ) -> Result<Count, LoadError> {
        match raw {
            None => Ok(Count::Fixed(1)),
            Some(RawNumber::Int(n)) => u32::try_from(*n)
                .map(Count::Fixed)
                .map_err(|_| self.invalid(at, format!("{context} : nombre {n} hors bornes"))),
            Some(RawNumber::Text(text)) => expr::parse_count(text, names)
                .map_err(|e| self.expr_error(e, context, None, at.start)),
        }
    }

    fn world(&self, raw: RawWorld) -> Result<NativeWorld, LoadError> {
        for (len, what) in [
            (raw.options.len(), "options"),
            (raw.items.len(), "objets"),
            (raw.item_groups.len(), "groupes d'objets"),
            (raw.regions.len(), "régions"),
            (raw.entrances.len(), "entrées"),
            (raw.locations.len(), "lieux"),
            (raw.location_groups.len(), "groupes de lieux"),
            (raw.events.len(), "événements"),
            (raw.pool.len(), "lignes du pool"),
            (raw.start.len(), "objets de départ"),
            (raw.slot_data.len(), "clés de slot_data"),
        ] {
            self.check_len(len, what)?;
        }
        self.check_name(&raw.game.name, "jeu")?;

        let mut names = Names::default();
        let options = self.options(&raw.options)?;
        for (i, option) in options.iter().enumerate() {
            names.options.insert(option.name.clone(), index(i));
        }
        names.option_defs = options.clone();

        // Espaces de noms : on remplit d'abord tous les noms, puis on analyse les règles,
        // qui peuvent citer un élément déclaré plus bas dans le fichier.
        let mut item_ids = BTreeSet::new();
        for (i, item) in raw.items.iter().enumerate() {
            self.check_name(&item.name, "objet")?;
            if names
                .items
                .insert(item.name.get_ref().clone(), ItemRef::Item(index(i)))
                .is_some()
            {
                return Err(self.invalid(
                    item.name.span(),
                    format!("objet « {} » en double", item.name.get_ref()),
                ));
            }
            if item.id <= 0 {
                return Err(self.invalid(
                    item.name.span(),
                    format!(
                        "objet « {} » : numéro {} (il doit être positif)",
                        item.name.get_ref(),
                        item.id
                    ),
                ));
            }
            if !item_ids.insert(item.id) {
                return Err(self.invalid(
                    item.name.span(),
                    format!(
                        "objet « {} » : numéro {} déjà pris",
                        item.name.get_ref(),
                        item.id
                    ),
                ));
            }
        }
        for (i, event) in raw.events.iter().enumerate() {
            self.check_name(&event.name, "événement")?;
            if event.item.is_empty() || event.item.chars().count() > MAX_NAME_CHARS {
                return Err(self.invalid(
                    event.name.span(),
                    format!(
                        "événement « {} » : nom d'objet invalide",
                        event.name.get_ref()
                    ),
                ));
            }
            match names.items.get(&event.item) {
                None => {
                    names
                        .items
                        .insert(event.item.clone(), ItemRef::Event(index(i)));
                }
                // Format 2 : un objet d'événement peut être posé par plusieurs événements
                // (« Note Read » ×30) ; le nom désigne le premier, l'objet est le même.
                Some(ItemRef::Event(_)) if raw.format_version >= 2 => {}
                Some(ItemRef::Event(_)) => {
                    return Err(self.invalid(
                        event.name.span(),
                        format!("événement « {} » : l'objet « {} » est déjà donné par un autre événement (permis à partir de format_version = 2)", event.name.get_ref(), event.item),
                    ));
                }
                Some(ItemRef::Item(_)) => {
                    return Err(self.invalid(
                        event.name.span(),
                        format!("événement « {} » : l'objet « {} » existe déjà (un objet d'événement est un nouvel objet)", event.name.get_ref(), event.item),
                    ));
                }
            }
        }
        for (i, group) in raw.item_groups.iter().enumerate() {
            self.check_name(&group.name, "groupe d'objets")?;
            if names
                .groups
                .insert(group.name.get_ref().clone(), index(i))
                .is_some()
            {
                return Err(self.invalid(
                    group.name.span(),
                    format!("groupe d'objets « {} » en double", group.name.get_ref()),
                ));
            }
        }
        for (i, region) in raw.regions.iter().enumerate() {
            self.check_name(&region.name, "région")?;
            if names
                .regions
                .insert(region.name.get_ref().clone(), index(i))
                .is_some()
            {
                return Err(self.invalid(
                    region.name.span(),
                    format!("région « {} » en double", region.name.get_ref()),
                ));
            }
        }
        for (i, entrance) in raw.entrances.iter().enumerate() {
            self.check_name(&entrance.name, "entrée")?;
            if names
                .entrances
                .insert(entrance.name.get_ref().clone(), index(i))
                .is_some()
            {
                return Err(self.invalid(
                    entrance.name.span(),
                    format!("entrée « {} » en double", entrance.name.get_ref()),
                ));
            }
        }
        let mut location_ids = BTreeSet::new();
        for (i, location) in raw.locations.iter().enumerate() {
            self.check_name(&location.name, "lieu")?;
            if names
                .locations
                .insert(
                    location.name.get_ref().clone(),
                    LocationRef::Location(index(i)),
                )
                .is_some()
            {
                return Err(self.invalid(
                    location.name.span(),
                    format!("lieu « {} » en double", location.name.get_ref()),
                ));
            }
            if location.id <= 0 {
                return Err(self.invalid(
                    location.name.span(),
                    format!(
                        "lieu « {} » : numéro {} (il doit être positif)",
                        location.name.get_ref(),
                        location.id
                    ),
                ));
            }
            if !location_ids.insert(location.id) {
                return Err(self.invalid(
                    location.name.span(),
                    format!(
                        "lieu « {} » : numéro {} déjà pris",
                        location.name.get_ref(),
                        location.id
                    ),
                ));
            }
        }
        for (i, event) in raw.events.iter().enumerate() {
            if names
                .locations
                .insert(event.name.get_ref().clone(), LocationRef::Event(index(i)))
                .is_some()
            {
                return Err(self.invalid(
                    event.name.span(),
                    format!(
                        "lieu « {} » en double (un événement est aussi un lieu)",
                        event.name.get_ref()
                    ),
                ));
            }
        }

        if raw.regions.is_empty() {
            return Err(LoadError::Invalid {
                pos: None,
                message: "aucune région".into(),
            });
        }

        // Objets.
        let mut items = Vec::new();
        for item in &raw.items {
            let context = format!("objet « {} »", item.name.get_ref());
            let classification = self.choice(
                &item.classification,
                &names,
                &context,
                item.name.span(),
                classification,
            )?;
            items.push(ItemDef {
                name: item.name.get_ref().clone(),
                id: item.id,
                classification,
            });
        }

        // Groupes.
        let real_item = |name: &str| -> Result<u32, String> {
            match names.items.get(name) {
                Some(ItemRef::Item(i)) => Ok(*i),
                Some(ItemRef::Event(_)) => Err(format!("« {name} » est un objet d'événement")),
                None => Err(format!("objet « {name} » inconnu")),
            }
        };
        let mut item_groups = Vec::new();
        for group in &raw.item_groups {
            self.check_len(group.items.len(), "membres d'un groupe")?;
            let mut members = Vec::new();
            for name in &group.items {
                let item = real_item(name).map_err(|m| {
                    self.invalid(
                        group.name.span(),
                        format!("groupe « {} » : {m}", group.name.get_ref()),
                    )
                })?;
                members.push(ItemRef::Item(item));
            }
            item_groups.push(GroupDef {
                name: group.name.get_ref().clone(),
                members,
            });
        }
        let mut location_groups = Vec::new();
        let mut location_group_names = BTreeSet::new();
        for group in &raw.location_groups {
            self.check_name(&group.name, "groupe de lieux")?;
            if !location_group_names.insert(group.name.get_ref().clone()) {
                return Err(self.invalid(
                    group.name.span(),
                    format!("groupe de lieux « {} » en double", group.name.get_ref()),
                ));
            }
            self.check_len(group.locations.len(), "membres d'un groupe")?;
            let mut members = Vec::new();
            for name in &group.locations {
                match names.locations.get(name) {
                    Some(LocationRef::Location(l)) => members.push(*l),
                    Some(LocationRef::Event(_)) => {
                        return Err(self.invalid(
                            group.name.span(),
                            format!(
                                "groupe « {} » : « {name} » est un événement",
                                group.name.get_ref()
                            ),
                        ))
                    }
                    None => {
                        return Err(self.invalid(
                            group.name.span(),
                            format!(
                                "groupe « {} » : lieu « {name} » inconnu",
                                group.name.get_ref()
                            ),
                        ))
                    }
                }
            }
            location_groups.push(LocationGroupDef {
                name: group.name.get_ref().clone(),
                members,
            });
        }

        // Régions, entrées, lieux, événements.
        let region_index = |name: &str| -> Result<u32, String> {
            names
                .regions
                .get(name)
                .copied()
                .ok_or_else(|| format!("région « {name} » inconnue"))
        };
        let mut regions = Vec::new();
        for region in &raw.regions {
            let context = format!("région « {} »", region.name.get_ref());
            regions.push(RegionDef {
                name: region.name.get_ref().clone(),
                when: self.when(&region.when, &names, &context)?,
            });
        }
        let mut entrances = Vec::new();
        for entrance in &raw.entrances {
            let context = format!("entrée « {} »", entrance.name.get_ref());
            let at = entrance.name.span();
            let from = region_index(&entrance.from)
                .map_err(|m| self.invalid(at.clone(), format!("{context} : {m}")))?;
            let to = region_index(&entrance.to)
                .map_err(|m| self.invalid(at.clone(), format!("{context} : {m}")))?;
            entrances.push(EntranceDef {
                name: entrance.name.get_ref().clone(),
                from,
                to,
                rule: self.rule(&entrance.rule, &names, &context)?,
                when: self.when(&entrance.when, &names, &context)?,
            });
        }
        let mut locations = Vec::new();
        for location in &raw.locations {
            let context = format!("lieu « {} »", location.name.get_ref());
            let at = location.name.span();
            let progress = match location.progress.as_deref() {
                None | Some("default") => Progress::Default,
                Some("priority") => Progress::Priority,
                Some("excluded") => Progress::Excluded,
                Some(other) => {
                    return Err(self.invalid(
                        at,
                        format!(
                        "{context} : progress « {other} » inconnu (default, priority, excluded)"
                    ),
                    ))
                }
            };
            locations.push(LocationDef {
                name: location.name.get_ref().clone(),
                id: location.id,
                region: self.choice(&location.region, &names, &context, at, region_index)?,
                rule: self.rule(&location.rule, &names, &context)?,
                progress,
                when: self.when(&location.when, &names, &context)?,
            });
        }
        let mut events = Vec::new();
        for event in &raw.events {
            let context = format!("événement « {} »", event.name.get_ref());
            events.push(EventDef {
                name: event.name.get_ref().clone(),
                region: self.choice(
                    &event.region,
                    &names,
                    &context,
                    event.name.span(),
                    region_index,
                )?,
                rule: self.rule(&event.rule, &names, &context)?,
                item: event.item.clone(),
                when: self.when(&event.when, &names, &context)?,
            });
        }

        // Pool, remplissage, départ.
        let amounts = |list: &[RawAmount], what: &str| -> Result<Vec<Amount>, LoadError> {
            let mut out = Vec::new();
            for amount in list {
                let context = format!("{what} « {} »", amount.item.get_ref());
                let item = real_item(amount.item.get_ref())
                    .map_err(|m| self.invalid(amount.item.span(), format!("{what} : {m}")))?;
                out.push(Amount {
                    item,
                    count: self.count(
                        amount.count.as_ref(),
                        &names,
                        &context,
                        amount.item.span(),
                    )?,
                    when: self.when(&amount.when, &names, &context)?,
                });
            }
            Ok(out)
        };
        let pool = amounts(&raw.pool, "pool")?;
        let start = amounts(&raw.start, "départ")?;
        let filler = match &raw.filler {
            None => None,
            Some(f) => {
                let at = f.item.span();
                let item = real_item(f.item.get_ref())
                    .map_err(|m| self.invalid(at.clone(), format!("filler : {m}")))?;
                let trap = match (&f.trap, &f.trap_percent) {
                    (None, None) => None,
                    (Some(trap), Some(percent)) => {
                        let trap = real_item(trap)
                            .map_err(|m| self.invalid(at.clone(), format!("filler : {m}")))?;
                        let percent =
                            self.count(Some(percent), &names, "filler : trap_percent", at.clone())?;
                        if let Count::Fixed(n) = percent {
                            if n > 100 {
                                return Err(self.invalid(
                                    at,
                                    "filler : trap_percent doit être entre 0 et 100",
                                ));
                            }
                        }
                        Some((trap, percent))
                    }
                    _ => {
                        return Err(
                            self.invalid(at, "filler : « trap » et « trap_percent » vont ensemble")
                        )
                    }
                };
                Some(Filler { item, trap })
            }
        };

        // slot_data.
        let mut slot_data = Vec::new();
        let mut keys = BTreeSet::new();
        for entry in &raw.slot_data {
            let at = entry.key.span();
            if !keys.insert(entry.key.get_ref().clone()) {
                return Err(self.invalid(
                    at,
                    format!("slot_data : clé « {} » en double", entry.key.get_ref()),
                ));
            }
            let value = match (&entry.option, &entry.value) {
                (Some(option), None) => match names.options.get(option) {
                    Some(&o) => SlotDataValue::Option(o),
                    None => {
                        return Err(self.invalid(
                            at,
                            format!(
                                "slot_data « {} » : option « {option} » inconnue",
                                entry.key.get_ref()
                            ),
                        ))
                    }
                },
                (None, Some(value)) => SlotDataValue::Literal(value.clone()),
                _ => {
                    return Err(self.invalid(
                        at,
                        format!(
                            "slot_data « {} » : écrire « option » ou « value » (un seul des deux)",
                            entry.key.get_ref()
                        ),
                    ))
                }
            };
            slot_data.push(SlotDataDef {
                key: entry.key.get_ref().clone(),
                value,
            });
        }

        // Jeu : départ et objectif.
        let start_region = match &raw.game.start_region {
            None => None,
            Some(name) => Some(
                region_index(name.get_ref())
                    .map_err(|m| self.invalid(name.span(), format!("start_region : {m}")))?,
            ),
        };
        let goal = self.expr(&raw.game.goal, &names, "objectif")?;

        let world = NativeWorld {
            format_version: raw.format_version,
            game: raw.game.name.get_ref().clone(),
            start_region,
            goal,
            options,
            items,
            item_groups,
            regions,
            entrances,
            locations,
            location_groups,
            events,
            pool,
            filler,
            start,
            slot_data,
        };
        self.check_reachable(&world, &raw.regions)?;
        Ok(world)
    }

    /// Toute région doit être atteignable depuis le départ en suivant les entrées (toutes,
    /// sans leurs règles ni leurs conditions).
    fn check_reachable(&self, world: &NativeWorld, raw: &[RawRegion]) -> Result<(), LoadError> {
        let origin = world.start_region.unwrap_or(0);
        let reached = crate::build::reachable_regions(
            world.regions.len(),
            origin,
            world.entrances.iter().map(|e| (e.from, e.to)),
        );
        for (i, region) in raw.iter().enumerate() {
            if !reached.get(i).copied().unwrap_or(false) {
                let start = world
                    .regions
                    .get(origin as usize)
                    .map(|r| r.name.as_str())
                    .unwrap_or("?");
                return Err(self.invalid(
                    region.name.span(),
                    format!("région « {} » inaccessible depuis « {start} » (aucune suite d'entrées n'y mène)", region.name.get_ref()),
                ));
            }
        }
        Ok(())
    }

    fn options(&self, raw: &[RawOption]) -> Result<Vec<OptionDef>, LoadError> {
        let mut out: Vec<OptionDef> = Vec::new();
        for option in raw {
            self.check_name(&option.name, "option")?;
            let name = option.name.get_ref();
            let at = option.name.span();
            let bad = |message: String| {
                self.invalid(at.clone(), format!("option « {name} » : {message}"))
            };
            if out.iter().any(|o| &o.name == name) {
                return Err(bad("en double".into()));
            }
            if RESERVED_OPTIONS.contains(&name.as_str()) {
                return Err(bad(
                    "nom réservé aux options communes à tous les jeux".into()
                ));
            }
            if name
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || c == '_'))
            {
                return Err(bad(
                    "nom fait de lettres, chiffres et « _ » seulement".into()
                ));
            }
            let not_for = |field: &str, present: bool| -> Result<(), LoadError> {
                if present {
                    Err(bad(format!(
                        "« {field} » ne va pas avec le type « {} »",
                        option.kind
                    )))
                } else {
                    Ok(())
                }
            };
            let (kind, default) = match option.kind.as_str() {
                "toggle" => {
                    not_for("min", option.min.is_some())?;
                    not_for("max", option.max.is_some())?;
                    not_for("choices", option.choices.is_some())?;
                    not_for("aliases", option.aliases.is_some())?;
                    let default = match &option.default {
                        None | Some(RawScalar::Bool(false)) => 0,
                        Some(RawScalar::Bool(true)) => 1,
                        _ => return Err(bad("défaut true ou false attendu".into())),
                    };
                    (OptionKind::Toggle, default)
                }
                "range" => {
                    not_for("choices", option.choices.is_some())?;
                    not_for("aliases", option.aliases.is_some())?;
                    let (Some(min), Some(max)) = (option.min, option.max) else {
                        return Err(bad("« min » et « max » obligatoires".into()));
                    };
                    if min > max {
                        return Err(bad(format!("min {min} plus grand que max {max}")));
                    }
                    let default = match &option.default {
                        None => min,
                        Some(RawScalar::Int(n)) => *n,
                        _ => return Err(bad("défaut entier attendu".into())),
                    };
                    if !(min..=max).contains(&default) {
                        return Err(bad(format!("défaut {default} hors de {min}..={max}")));
                    }
                    (OptionKind::Range { min, max }, default)
                }
                "choice" => {
                    not_for("min", option.min.is_some())?;
                    not_for("max", option.max.is_some())?;
                    let choices = option.choices.clone().unwrap_or_default();
                    if choices.is_empty() {
                        return Err(bad("« choices » vide".into()));
                    }
                    if choices.len() > MAX_CHOICES {
                        return Err(bad(format!("plus de {MAX_CHOICES} choix")));
                    }
                    let mut seen = BTreeSet::new();
                    for choice in &choices {
                        if choice.is_empty() || choice.chars().count() > MAX_NAME_CHARS {
                            return Err(bad(format!("choix « {choice} » invalide")));
                        }
                        if !seen.insert(choice.to_ascii_lowercase()) {
                            return Err(bad(format!("choix « {choice} » en double")));
                        }
                    }
                    let mut aliases = Vec::new();
                    // `BTreeMap` : alias parcourus par ordre alphabétique, donc stable.
                    for (alias, target) in option.aliases.iter().flatten() {
                        if !seen.insert(alias.to_ascii_lowercase()) {
                            return Err(bad(format!(
                                "alias « {alias} » déjà pris par un choix ou un alias"
                            )));
                        }
                        let Some(target) =
                            choices.iter().position(|c| c.eq_ignore_ascii_case(target))
                        else {
                            return Err(bad(format!(
                                "alias « {alias} » vers un choix inconnu « {target} »"
                            )));
                        };
                        aliases.push((alias.clone(), index(target)));
                    }
                    let default = match &option.default {
                        None => 0,
                        Some(RawScalar::Text(text)) => choice_index(&choices, &aliases, text)
                            .ok_or_else(|| bad(format!("défaut « {text} » n'est pas un choix")))?,
                        Some(RawScalar::Int(n)) if *n >= 0 && (*n as usize) < choices.len() => *n,
                        _ => return Err(bad("défaut : nom d'un choix attendu".into())),
                    };
                    (OptionKind::Choice { choices, aliases }, default)
                }
                other => {
                    return Err(bad(format!(
                        "type « {other} » inconnu (toggle, range, choice)"
                    )))
                }
            };
            out.push(OptionDef {
                name: name.clone(),
                display: option.display.clone().unwrap_or_else(|| title_case(name)),
                kind,
                default,
            });
        }
        Ok(out)
    }
}

/// Lit une classification : `progression`, `useful`, `filler`, `trap`, `skip_balancing`,
/// combinées par `+`.
fn classification(text: &str) -> Result<Classification, String> {
    let mut c = Classification::FILLER;
    let mut filler = false;
    for word in text.split('+').map(str::trim) {
        match word {
            "progression" => c.progression = true,
            "useful" => c.useful = true,
            "trap" => c.trap = true,
            "skip_balancing" => c.skip_balancing = true,
            "progression_skip_balancing" => {
                c.progression = true;
                c.skip_balancing = true;
            }
            "filler" => filler = true,
            other => {
                return Err(format!(
                    "classification « {other} » inconnue (progression, useful, filler, trap, skip_balancing)"
                ))
            }
        }
    }
    if filler && c != Classification::FILLER {
        return Err("« filler » ne se combine avec rien".into());
    }
    if c.skip_balancing && !c.progression {
        return Err("« skip_balancing » demande « progression »".into());
    }
    Ok(c)
}
