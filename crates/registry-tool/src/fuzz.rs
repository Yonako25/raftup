//! Cas de fuzzing déterministes (ADR 0042).
//!
//! Chaque cas est tiré par son propre générateur pseudo-aléatoire, initialisé par la
//! graine du fuzzing et le numéro du cas : le cas n° 7 est toujours le même, qu'on lance
//! 10 cas ou 40. Le générateur (splitmix64) est écrit ici, sans dépendance, pour que la
//! suite des nombres ne change jamais avec une mise à jour de bibliothèque.

use crate::options::{quote, yaml_with, OptionKind, OptionSpec, PLAYER_NAME};

/// Graine par défaut du fuzzing (« RAFTUP » en ASCII, puis 42).
pub const DEFAULT_FUZZ_SEED: u64 = 0x5241_4654_5550_0042;

/// Générateur pseudo-aléatoire splitmix64 : petit, rapide, et entièrement déterminé par
/// sa graine. Ce n'est pas un générateur cryptographique (inutile ici).
#[derive(Debug, Clone)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> SplitMix64 {
        SplitMix64(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        // `wrapping_*` : les débordements font le tour, comme le veut l'algorithme.
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Entier dans `0..n` (`n` > 0). Le léger biais du modulo est sans importance ici.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }

    /// Vrai avec une probabilité de `percent` %.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    /// Un élément d'une tranche non vide.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        // `'a` : l'élément rendu est emprunté à `items`.
        if items.is_empty() {
            return None;
        }
        items.get(self.below(items.len() as u64) as usize)
    }

    /// Entier entre `start` et `end` inclus.
    pub fn between(&mut self, start: i64, end: i64) -> i64 {
        // Calcul en i128 : `end - start` peut dépasser un i64.
        let width = (end as i128 - start as i128 + 1).max(1) as u128;
        let offset = (self.next_u64() as u128) % width;
        (start as i128 + offset as i128) as i64
    }
}

/// Sorte de cas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseKind {
    /// Options valides tirées au hasard : un refus du générateur est permis mais noté.
    ValidOptions,
    /// Valeurs limites ou hors bornes, mauvais types : le générateur doit refuser
    /// proprement (ou accepter), jamais planter ni boucler.
    EdgeValues,
    /// YAML abîmé : mêmes attentes.
    BrokenYaml,
}

impl CaseKind {
    pub fn label(self) -> &'static str {
        match self {
            CaseKind::ValidOptions => "options valides",
            CaseKind::EdgeValues => "valeurs limites",
            CaseKind::BrokenYaml => "YAML abîmé",
        }
    }
}

/// Un cas de fuzzing : un YAML et la graine de la génération.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzCase {
    /// Numéro du cas, à partir de 1.
    pub number: u32,
    pub kind: CaseKind,
    /// Graine donnée au générateur.
    pub seed: u64,
    pub yaml: String,
    /// Ce que le cas essaie, en quelques mots (pour le rapport).
    pub summary: String,
}

/// Le cas n° `number` (à partir de 1) pour ce jeu et ces options.
pub fn make_case(fuzz_seed: u64, number: u32, game: &str, options: &[OptionSpec]) -> FuzzCase {
    // Graine propre au cas : mélange de la graine du fuzzing et du numéro.
    let mut rng =
        SplitMix64::new(fuzz_seed ^ (u64::from(number)).wrapping_mul(0xA24B_AED4_963E_E407));
    let kind = match number % 4 {
        // Deux cas sur quatre avec des options valides : c'est le cas le plus utile.
        0 | 1 => CaseKind::ValidOptions,
        2 => CaseKind::EdgeValues,
        _ => CaseKind::BrokenYaml,
    };
    let seed = rng.below(1_000_000_000);
    let (yaml, summary) = match kind {
        CaseKind::ValidOptions => valid_options(&mut rng, game, options),
        CaseKind::EdgeValues => edge_values(&mut rng, game, options),
        CaseKind::BrokenYaml => broken_yaml(&mut rng, game, options),
    };
    FuzzCase {
        number,
        kind,
        seed,
        yaml,
        summary,
    }
}

/// Chaque option fuzzable reçoit, avec 70 % de chances, une valeur valide.
fn valid_options(rng: &mut SplitMix64, game: &str, options: &[OptionSpec]) -> (String, String) {
    let mut values = Vec::new();
    for option in options {
        if !rng.chance(70) {
            continue;
        }
        let value = match &option.kind {
            OptionKind::Toggle => {
                if rng.chance(50) {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            OptionKind::Choice(choices) => match rng.pick(choices) {
                Some(choice) => quote(choice),
                None => continue,
            },
            OptionKind::Range { start, end } => match rng.below(4) {
                0 => start.to_string(),
                1 => end.to_string(),
                _ => rng.between(*start, *end).to_string(),
            },
            OptionKind::Other => continue,
        };
        values.push((option.name.clone(), value));
    }
    let summary = format!("{} option(s) changée(s)", values.len());
    (yaml_with(game, &values), summary)
}

/// Une à trois options reçoivent une valeur interdite ou d'un mauvais type.
fn edge_values(rng: &mut SplitMix64, game: &str, options: &[OptionSpec]) -> (String, String) {
    const WRONG: [&str; 8] = [
        "\"raftup_valeur_inconnue\"",
        "-1",
        "1000000000000000000",
        "\"\"",
        "[1, 2, 3]",
        "{\"a\": \"b\"}",
        "null",
        "3.5",
    ];
    let mut values = Vec::new();
    let mut names = Vec::new();
    let count = 1 + rng.below(3);
    for _ in 0..count {
        let Some(option) = rng.pick(options) else {
            break;
        };
        let value = match &option.kind {
            // Juste hors des bornes, d'un côté ou de l'autre.
            OptionKind::Range { start, end } if rng.chance(50) => {
                if rng.chance(50) {
                    start.saturating_sub(1).to_string()
                } else {
                    end.saturating_add(1).to_string()
                }
            }
            _ => rng.pick(&WRONG).copied().unwrap_or("null").to_string(),
        };
        names.push(format!("{} = {value}", option.name));
        values.push((option.name.clone(), value));
    }
    if values.is_empty() {
        // Monde sans option : on abîme une option qui n'existe pas.
        values.push(("raftup_option_inconnue".to_string(), "true".to_string()));
        names.push("option inconnue".to_string());
    }
    (yaml_with(game, &values), names.join(", "))
}

/// Un YAML abîmé parmi plusieurs formes.
fn broken_yaml(rng: &mut SplitMix64, game: &str, options: &[OptionSpec]) -> (String, String) {
    let name = quote(PLAYER_NAME);
    let game_q = quote(game);
    match rng.below(9) {
        0 => (String::new(), "fichier vide".to_string()),
        1 => (
            format!("name: {name}\ngame: {game_q}\n"),
            "section du jeu absente".to_string(),
        ),
        2 => (
            format!("name: {name}\ngame: \"Raftup Jeu Inconnu\"\n\"Raftup Jeu Inconnu\": {{}}\n"),
            "jeu inconnu".to_string(),
        ),
        3 => (
            format!("name: {name}\ngame: {game_q}\n{game_q}:\n  \"cle: [non fermée\n"),
            "guillemets non fermés".to_string(),
        ),
        4 => {
            // Listes imbriquées sur 60 niveaux.
            let deep = format!("{}{}", "[".repeat(60), "]".repeat(60));
            let option = rng
                .pick(options)
                .map(|o| o.name.clone())
                .unwrap_or_else(|| "raftup".to_string());
            (
                format!(
                    "name: {name}\ngame: {game_q}\n{game_q}:\n  {}: {deep}\n",
                    quote(&option)
                ),
                "valeur imbriquée sur 60 niveaux".to_string(),
            )
        }
        5 => (
            format!(
                "name: {}\ngame: {game_q}\n{game_q}: {{}}\n",
                quote(&"x".repeat(5000))
            ),
            "nom de joueur de 5 000 caractères".to_string(),
        ),
        6 => (
            format!("name: {name}\ngame: {game_q}\n{game_q}:\n\t\"tabulation\": 1\n"),
            "indentation par tabulation".to_string(),
        ),
        7 => (
            format!("name: {name}\ngame: {game_q}\ngame: {game_q}\n{game_q}: [1, 2]\n"),
            "clé en double, section en liste".to_string(),
        ),
        _ => {
            // Octets pseudo-aléatoires imprimables.
            let noise: String = (0..200)
                .map(|_| char::from(b' ' + rng.below(95) as u8))
                .collect();
            (noise, "texte au hasard".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn options() -> Vec<OptionSpec> {
        vec![
            OptionSpec {
                name: "hard".into(),
                kind: OptionKind::Toggle,
            },
            OptionSpec {
                name: "goal".into(),
                kind: OptionKind::Choice(vec!["a".into(), "b".into()]),
            },
            OptionSpec {
                name: "count".into(),
                kind: OptionKind::Range { start: 1, end: 5 },
            },
        ]
    }

    #[test]
    fn splitmix64_suite_connue() {
        // Valeurs de référence de splitmix64 pour la graine 0.
        let mut rng = SplitMix64::new(0);
        assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.next_u64(), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn bornes_respectees() {
        let mut rng = SplitMix64::new(7);
        for _ in 0..1000 {
            let v = rng.between(-3, 3);
            assert!((-3..=3).contains(&v));
        }
        assert_eq!(rng.between(i64::MIN, i64::MIN), i64::MIN);
        let _ = rng.between(i64::MIN, i64::MAX);
    }

    #[test]
    fn meme_graine_memes_cas() {
        let a: Vec<FuzzCase> = (1..=12)
            .map(|n| make_case(DEFAULT_FUZZ_SEED, n, "Jeu", &options()))
            .collect();
        let b: Vec<FuzzCase> = (1..=12)
            .map(|n| make_case(DEFAULT_FUZZ_SEED, n, "Jeu", &options()))
            .collect();
        assert_eq!(a, b);
        // Une autre graine change au moins un cas.
        let c: Vec<FuzzCase> = (1..=12)
            .map(|n| make_case(1, n, "Jeu", &options()))
            .collect();
        assert_ne!(a, c);
        // Les trois sortes de cas sont représentées.
        for kind in [
            CaseKind::ValidOptions,
            CaseKind::EdgeValues,
            CaseKind::BrokenYaml,
        ] {
            assert!(a.iter().any(|case| case.kind == kind));
        }
    }

    #[test]
    fn options_valides_dans_leurs_bornes() {
        for n in (1..200).filter(|n| n % 4 <= 1) {
            let case = make_case(3, n, "Jeu", &options());
            assert_eq!(case.kind, CaseKind::ValidOptions);
            for line in case.yaml.lines().skip(3) {
                if line.trim() == "{}" {
                    continue;
                }
                let (key, value) = line.trim().split_once(": ").unwrap();
                match key {
                    "\"hard\"" => assert!(value == "true" || value == "false"),
                    "\"goal\"" => assert!(value == "\"a\"" || value == "\"b\""),
                    "\"count\"" => assert!((1..=5).contains(&value.parse::<i64>().unwrap())),
                    other => panic!("option inattendue {other}"),
                }
            }
        }
    }

    #[test]
    fn monde_sans_option() {
        for n in 1..=8 {
            let case = make_case(5, n, "Jeu", &[]);
            assert!(!case.summary.is_empty());
        }
    }
}
