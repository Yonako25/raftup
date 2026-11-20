//! Options d'un monde, telles que les décrit le générateur officiel (ADR 0042), et
//! écriture des YAML de test.
//!
//! Les YAML sont écrits à la main : chaque texte (nom d'option, jeu, choix) est mis entre
//! guillemets doubles au format JSON, que YAML lit tel quel. Aucun texte venu d'une
//! apworld ne peut donc casser la structure du fichier.

use std::fmt::Write as _;

/// Script Python lancé sur le `MultiWorld` (voir `launcher_core::generation::inspect_worlds`)
/// pour lister les options du monde du joueur 1.
pub const DESCRIBE_SCRIPT: &str = include_str!("describe_options.py");

/// Nombre maximal d'options gardées, et de choix par option.
pub const MAX_OPTIONS: usize = 500;
pub const MAX_CHOICES: usize = 200;

/// Nom du joueur des parties de test.
pub const PLAYER_NAME: &str = "Testeur";

/// La forme d'une option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionKind {
    /// Interrupteur (`true` / `false`).
    Toggle,
    /// Un nom parmi une liste (sans les alias).
    Choice(Vec<String>),
    /// Un entier entre deux bornes incluses.
    Range { start: i64, end: i64 },
    /// Liste, dictionnaire, texte libre… : non fuzzée, seulement abîmée.
    Other,
}

/// Une option d'un monde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionSpec {
    pub name: String,
    pub kind: OptionKind,
}

/// Lit la description écrite par [`DESCRIBE_SCRIPT`] : `{"game", "options": [...]}`.
/// Une option mal décrite devient `Other` plutôt que de tout refuser.
pub fn parse_description(value: &serde_json::Value) -> Result<Vec<OptionSpec>, String> {
    let options = value
        .get("options")
        .and_then(|o| o.as_array())
        .ok_or("description des options sans liste « options »")?;
    let mut specs = Vec::new();
    for option in options.iter().take(MAX_OPTIONS) {
        let Some(name) = option.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        if name.is_empty() || name.len() > 100 || name.chars().any(char::is_control) {
            continue;
        }
        let kind = match option.get("kind").and_then(|k| k.as_str()) {
            Some("toggle") => OptionKind::Toggle,
            Some("choice") => {
                let choices: Vec<String> = option
                    .get("choices")
                    .and_then(|c| c.as_array())
                    .map(|list| {
                        list.iter()
                            .filter_map(|c| c.as_str())
                            .filter(|c| !c.is_empty() && c.len() <= 100)
                            .take(MAX_CHOICES)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                if choices.is_empty() {
                    OptionKind::Other
                } else {
                    OptionKind::Choice(choices)
                }
            }
            Some("range") => {
                let start = option.get("start").and_then(|v| v.as_i64());
                let end = option.get("end").and_then(|v| v.as_i64());
                match (start, end) {
                    (Some(start), Some(end)) if start <= end => OptionKind::Range { start, end },
                    _ => OptionKind::Other,
                }
            }
            _ => OptionKind::Other,
        };
        specs.push(OptionSpec {
            name: name.to_string(),
            kind,
        });
    }
    Ok(specs)
}

/// Texte entre guillemets doubles, lisible par YAML (échappements JSON).
pub fn quote(text: &str) -> String {
    // `to_string` sur une `&str` ne peut pas échouer ; `unwrap_or_default` évite `unwrap`.
    serde_json::to_string(text).unwrap_or_default()
}

/// YAML d'une partie solo avec les options par défaut du jeu.
pub fn default_yaml(game: &str) -> String {
    yaml_with(game, &[])
}

/// YAML d'une partie solo : chaque couple est (nom de l'option, valeur YAML déjà écrite).
pub fn yaml_with(game: &str, values: &[(String, String)]) -> String {
    let mut text = format!(
        "name: {}\ngame: {}\n{}:\n",
        quote(PLAYER_NAME),
        quote(game),
        quote(game)
    );
    if values.is_empty() {
        text.push_str("  {}\n");
    }
    for (name, value) in values {
        // `writeln!` dans une `String` ne peut pas échouer : on ignore le résultat.
        let _ = writeln!(text, "  {}: {value}", quote(name));
    }
    text
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn description_lue_et_options_douteuses_ecartees() {
        let value = serde_json::json!({
            "game": "Jeu",
            "options": [
                {"name": "hard", "kind": "toggle"},
                {"name": "goal", "kind": "choice", "choices": ["a", "b"]},
                {"name": "count", "kind": "range", "start": 1, "end": 10},
                {"name": "bornes_inversees", "kind": "range", "start": 10, "end": 1},
                {"name": "vide", "kind": "choice", "choices": []},
                {"name": "items", "kind": "other"},
                {"kind": "toggle"},
                {"name": "a\nb", "kind": "toggle"}
            ]
        });
        let specs = parse_description(&value).unwrap();
        let kinds: Vec<(&str, &OptionKind)> =
            specs.iter().map(|s| (s.name.as_str(), &s.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("hard", &OptionKind::Toggle),
                ("goal", &OptionKind::Choice(vec!["a".into(), "b".into()])),
                ("count", &OptionKind::Range { start: 1, end: 10 }),
                ("bornes_inversees", &OptionKind::Other),
                ("vide", &OptionKind::Other),
                ("items", &OptionKind::Other),
            ]
        );
        assert!(parse_description(&serde_json::json!({})).is_err());
    }

    #[test]
    fn yaml_par_defaut_et_guillemets() {
        assert_eq!(
            default_yaml("Slime Rancher"),
            "name: \"Testeur\"\ngame: \"Slime Rancher\"\n\"Slime Rancher\":\n  {}\n"
        );
        // Un nom piégé reste un simple texte.
        let yaml = yaml_with("J\"eu", &[("a: b\n- c".to_string(), "true".to_string())]);
        assert_eq!(
            yaml,
            "name: \"Testeur\"\ngame: \"J\\\"eu\"\n\"J\\\"eu\":\n  \"a: b\\n- c\": true\n"
        );
    }
}
