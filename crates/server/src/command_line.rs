//! Découpage d'une ligne de commande comme le fait le serveur officiel
//! (`CommandProcessor.__call__`, `MultiServer.py`, Archipelago 0.6.8) :
//! `shlex.split(raw, comments=False)` en mode POSIX, avec repli sur
//! `raw.split()` quand une citation n'est pas fermée.
//!
//! Le découpeur est écrit d'après la documentation de `shlex` (automate à états),
//! sans reprise de code de CPython.

/// Espaces reconnus par `shlex` (seulement ces quatre, pas les espaces Unicode).
fn is_shlex_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// Espaces reconnus par `str.split()` / `str.isspace()` de Python : les espaces
/// Unicode, plus les séparateurs ASCII U+001C à U+001F que Rust ne compte pas.
fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// État de l'automate de `shlex` (mode POSIX, `whitespace_split` activé).
/// Une `enum` Rust remplace les caractères-états du code Python.
#[derive(Clone, Copy, PartialEq)]
enum State {
    /// Entre deux mots.
    Space,
    /// Dans un mot, hors guillemets.
    Word,
    /// Dans des guillemets simples : tout est littéral.
    SingleQuote,
    /// Dans des guillemets doubles : `\` n'échappe que `"` et `\`.
    DoubleQuote,
    /// Après un `\`, avec l'état à retrouver ensuite.
    Escape(Return),
}

/// États auxquels on revient après un échappement (un `State` ne peut pas se
/// contenir lui-même sans indirection, d'où ce petit type à part).
#[derive(Clone, Copy, PartialEq)]
enum Return {
    Word,
    DoubleQuote,
}

/// Port de `shlex.split(raw, comments=False)` en mode POSIX.
/// Renvoie `None` quand Python lèverait `ValueError` (citation ou échappement
/// non terminé).
fn shlex_split(raw: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    // « Mot commencé » : vrai dès qu'un caractère ou une paire de guillemets
    // (même vide) a été vu, pour que `''` produise un mot vide.
    let mut started = false;
    let mut state = State::Space;

    for c in raw.chars() {
        state = match state {
            State::Space | State::Word => {
                if is_shlex_space(c) {
                    if started {
                        tokens.push(std::mem::take(&mut token));
                        started = false;
                    }
                    State::Space
                } else if c == '\\' {
                    started = true;
                    State::Escape(Return::Word)
                } else if c == '\'' {
                    started = true;
                    State::SingleQuote
                } else if c == '"' {
                    started = true;
                    State::DoubleQuote
                } else {
                    started = true;
                    token.push(c);
                    State::Word
                }
            }
            State::SingleQuote => {
                if c == '\'' {
                    State::Word
                } else {
                    token.push(c);
                    State::SingleQuote
                }
            }
            State::DoubleQuote => {
                if c == '"' {
                    State::Word
                } else if c == '\\' {
                    State::Escape(Return::DoubleQuote)
                } else {
                    token.push(c);
                    State::DoubleQuote
                }
            }
            State::Escape(back) => match back {
                Return::Word => {
                    token.push(c);
                    State::Word
                }
                Return::DoubleQuote => {
                    // Dans des guillemets doubles, `\` ne disparaît que devant `"` ou `\`;
                    // devant tout autre caractère, l'antislash reste dans le mot.
                    if c != '"' && c != '\\' {
                        token.push('\\');
                    }
                    token.push(c);
                    State::DoubleQuote
                }
            },
        };
    }

    match state {
        State::SingleQuote | State::DoubleQuote | State::Escape(_) => None,
        State::Space | State::Word => {
            if started {
                tokens.push(token);
            }
            Some(tokens)
        }
    }
}

/// `shlex.split(raw, comments=False)` avec le repli du serveur officiel : si la
/// citation n'est pas fermée, on découpe simplement sur les espaces.
pub(crate) fn split_command(raw: &str) -> Vec<String> {
    match shlex_split(raw) {
        Some(words) => words,
        None => raw
            .split(is_py_space)
            .filter(|w| !w.is_empty())
            .map(String::from)
            .collect(),
    }
}

/// `raw.split(maxsplit=1)[1]` de Python : le texte après le premier mot, sans les
/// espaces de tête mais avec ceux de fin. `None` s'il n'y a pas de second élément.
pub(crate) fn raw_argument(raw: &str) -> Option<String> {
    let after_first = raw.trim_start_matches(is_py_space);
    let word_end = after_first.find(is_py_space)?;
    let rest = after_first[word_end..].trim_start_matches(is_py_space);
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

/// Approximation de `str.isnumeric()` : non vide et tous les caractères numériques
/// Unicode (`char::is_numeric` couvre les catégories Nd, Nl et No, comme Python).
// Pour `!hint` (T7b).
#[allow(dead_code)]
pub(crate) fn py_isnumeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(char::is_numeric)
}

/// Les `n` premiers caractères de `s` (`s[:n]` de Python : on compte des
/// caractères, pas des octets, pour ne jamais couper un caractère en deux).
pub(crate) fn truncate_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::Value;

    #[test]
    fn matches_python_vectors() {
        let cases: Value = serde_json::from_str(include_str!("../tests/data/shlex-vectors.json"))
            .expect("shlex-vectors.json valide");
        let cases = cases.as_array().expect("tableau de cas");
        assert!(cases.len() >= 80);
        for case in cases {
            let raw = case["raw"].as_str().expect("raw");
            let split: Vec<String> = case["split"]
                .as_array()
                .expect("tableau")
                .iter()
                .map(|x| x.as_str().expect("texte").to_string())
                .collect();
            let arg = case["arg"].as_str().map(String::from);
            assert_eq!(split_command(raw), split, "split de {raw:?}");
            assert_eq!(raw_argument(raw), arg, "arg de {raw:?}");
        }
    }

    #[test]
    fn isnumeric_and_truncate() {
        assert!(py_isnumeric("123"));
        assert!(py_isnumeric("²"));
        assert!(!py_isnumeric(""));
        assert!(!py_isnumeric("12a"));
        assert!(!py_isnumeric("-1"));
        assert_eq!(truncate_chars("éàü€x", 3), "éàü");
        assert_eq!(truncate_chars("ab", 10), "ab");
    }
}
