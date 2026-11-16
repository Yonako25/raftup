//! Recherche floue du serveur officiel : port de `get_intended_text` et
//! `get_fuzzy_results` (`Utils.py`, Archipelago 0.6.8).
//!
//! Elle sert à deviner un nom de joueur, de jeu, d'objet ou de lieu mal tapé,
//! avec exactement les mêmes seuils et les mêmes messages que le serveur
//! officiel (les clients affichent ces messages tels quels).

use std::collections::BTreeMap;

/// Distance de Damerau-Levenshtein NON restreinte (algorithme de Lowrance-Wagner),
/// celle de `jellyfish.damerau_levenshtein_distance` : insertion, suppression,
/// remplacement et transposition de deux caractères voisins, chacune à coût 1,
/// y compris quand d'autres éditions s'intercalent (« ca » -> « abc » = 2).
///
/// On l'écrit à la main plutôt que d'ajouter `strsim` : une dépendance de moins,
/// et la correspondance avec le script Python de référence se lit ligne à ligne.
fn damerau_levenshtein(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    let max_dist = n + m;
    // Tableau (n + 2) x (m + 2) : la ligne et la colonne 0 portent une valeur
    // « infinie » qui interdit les transpositions hors limites.
    let width = m + 2;
    let mut d = vec![0usize; (n + 2) * width];
    let idx = |i: usize, j: usize| i * width + j;
    d[idx(0, 0)] = max_dist;
    for i in 0..=n {
        d[idx(i + 1, 0)] = max_dist;
        d[idx(i + 1, 1)] = i;
    }
    for j in 0..=m {
        d[idx(0, j + 1)] = max_dist;
        d[idx(1, j + 1)] = j;
    }
    // Dernière ligne (1-based) où chaque caractère de `a` a été vu.
    // `BTreeMap` plutôt que `HashMap` : ordre déterministe (règle du projet).
    let mut last_row: BTreeMap<char, usize> = BTreeMap::new();
    for i in 1..=n {
        let mut last_match_col = 0usize;
        for j in 1..=m {
            let i1 = last_row.get(&b[j - 1]).copied().unwrap_or(0);
            let j1 = last_match_col;
            let cost = if a[i - 1] == b[j - 1] {
                last_match_col = j;
                0
            } else {
                1
            };
            let substitution = d[idx(i, j)] + cost;
            let insertion = d[idx(i + 1, j)] + 1;
            let deletion = d[idx(i, j + 1)] + 1;
            // Transposition : (i - i1 - 1) suppressions + 1 échange + (j - j1 - 1) insertions.
            let transposition = d[idx(i1, j1)] + (i - i1 - 1) + 1 + (j - j1 - 1);
            d[idx(i + 1, j + 1)] = substitution.min(insertion).min(deletion).min(transposition);
        }
        last_row.insert(a[i - 1], i);
    }
    d[idx(n + 1, m + 1)]
}

/// Ratio de ressemblance de `get_fuzzy_ratio` : 1.01 si les textes sont
/// identiques, sinon `1 - distance / max(longueurs)` sur les textes en minuscules.
///
/// Comme en Python, la distance se calcule sur les minuscules mais le
/// dénominateur utilise la longueur des textes d'origine (en caractères).
fn fuzzy_ratio(word1: &str, word2: &str) -> f64 {
    if word1 == word2 {
        return 1.01;
    }
    let a: Vec<char> = word1.to_lowercase().chars().collect();
    let b: Vec<char> = word2.to_lowercase().chars().collect();
    let longest = word1.chars().count().max(word2.chars().count());
    // Les deux textes sont différents, donc `longest` n'est jamais 0.
    1.0 - damerau_levenshtein(&a, &b) as f64 / longest as f64
}

/// `get_fuzzy_results(input, candidates, limit=2)` : les deux meilleurs
/// candidats avec leur score entier (`int(ratio * 100)`, tronqué vers zéro).
fn top_two(input: &str, candidates: &[&str]) -> Vec<(String, i64)> {
    let mut scored: Vec<(&str, f64)> = candidates
        .iter()
        .map(|candidate| (*candidate, fuzzy_ratio(input, candidate)))
        .collect();
    // `sort_by` est stable (comme `sorted` de Python, y compris avec reverse=True) :
    // à score égal, l'ordre d'origine des candidats est conservé.
    scored.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(2)
        .map(|(text, ratio)| (text.to_string(), (ratio * 100.0) as i64))
        .collect()
}

/// `get_intended_text` : renvoie (texte retenu, utilisable, message).
///
/// `candidates` ne devrait jamais être vide (le serveur officiel plante dans ce
/// cas) ; ici on renvoie l'entrée telle quelle, non utilisable.
pub(crate) fn intended_text(input: &str, candidates: &[&str]) -> (String, bool, String) {
    let picks = top_two(input, candidates);
    let didnt_find = |best: &str, score: i64| {
        format!("Didn't find something that closely matches '{input}', did you mean '{best}'? ({score}% sure)")
    };
    match picks.as_slice() {
        [] => (
            input.to_string(),
            false,
            format!("Didn't find something that closely matches '{input}'"),
        ),
        [(best, score)] => {
            if *score > 90 {
                (best.clone(), true, "Only Option Match".to_string())
            } else {
                (best.clone(), false, didnt_find(best, *score))
            }
        }
        [(best, score), (_, second)] => {
            let dif = score - second;
            if *score == 101 {
                (best.clone(), true, "Perfect Match".to_string())
            } else if *score == 100 {
                (
                    best.clone(),
                    true,
                    "Case Insensitive Perfect Match".to_string(),
                )
            } else if *score < 75 {
                (best.clone(), false, didnt_find(best, *score))
            } else if dif > 5 {
                (best.clone(), true, "Close Match".to_string())
            } else {
                (
                    best.clone(),
                    false,
                    format!(
                        "Too many close matches for '{input}', did you mean '{best}'? ({score}% sure)"
                    ),
                )
            }
        }
        // `take(2)` garantit au plus deux éléments.
        _ => (input.to_string(), false, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::Value;

    fn strings(v: &Value) -> Vec<String> {
        v.as_array()
            .expect("tableau")
            .iter()
            .map(|x| x.as_str().expect("texte").to_string())
            .collect()
    }

    #[test]
    fn matches_python_vectors() {
        let cases: Value = serde_json::from_str(include_str!("../tests/data/fuzzy-vectors.json"))
            .expect("fuzzy-vectors.json valide");
        let cases = cases.as_array().expect("tableau de cas");
        assert!(cases.len() >= 150);
        for case in cases {
            let input = case["input"].as_str().expect("input");
            let candidates = strings(&case["candidates"]);
            let expected = &case["expected"];
            let expected = (
                expected[0].as_str().expect("texte").to_string(),
                expected[1].as_bool().expect("booleen"),
                expected[2].as_str().expect("message").to_string(),
            );
            let refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
            assert_eq!(
                intended_text(input, &refs),
                expected,
                "entrée {input:?}, candidats {candidates:?}"
            );
        }
    }

    #[test]
    fn transposition_is_unrestricted() {
        let a: Vec<char> = "ca".chars().collect();
        let b: Vec<char> = "abc".chars().collect();
        assert_eq!(damerau_levenshtein(&a, &b), 2);
    }
}
