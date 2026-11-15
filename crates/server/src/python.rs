//! Écrire des valeurs comme Python les affiche (`repr()`, `str()`, `type()`).
//!
//! Le serveur officiel glisse ces formes dans certains messages : « a rejoint »
//! avec la liste des tags, ou le paquet fautif dans un `InvalidPacket`. Pour
//! envoyer exactement le même texte, on les reproduit ici.
//!
//! Limite connue : les caractères Unicode que Python juge « non imprimables »
//! (par exemple U+00A0) ne sont pas échappés ; seuls les caractères de contrôle
//! ASCII le sont.

use serde_json::{Number, Value};

/// Écrit une liste de textes comme Python l'affiche : `['AP', 'TextOnly']`.
pub(crate) fn list_repr(items: &[String]) -> String {
    let parts: Vec<String> = items.iter().map(|s| str_repr(s)).collect();
    format!("[{}]", parts.join(", "))
}

/// `repr()` d'un texte en Python : guillemets simples, sauf si le texte en
/// contient et pas de doubles. Caractères de contrôle échappés.
pub(crate) fn str_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `repr()` d'une valeur JSON lue par Python (`json.loads`).
pub(crate) fn repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(number) => number_repr(number),
        Value::String(text) => str_repr(text),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(repr).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(map) => {
            // Python garde l'ordre des clés du JSON, comme `preserve_order`.
            let parts: Vec<String> = map
                .iter()
                .map(|(key, value)| format!("{}: {}", str_repr(key), repr(value)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
    }
}

/// `str()` d'une valeur : comme `repr()`, sauf qu'un texte s'écrit sans guillemets.
/// C'est ce que donne `f"{valeur}"` en Python.
pub(crate) fn str_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => repr(other),
    }
}

/// Nom du type Python d'une valeur JSON : `int`, `float`, `list`…
pub(crate) fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(number) if is_float_text(&number.to_string()) => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// Python lit un nombre JSON comme un `float` s'il a un point ou un exposant.
fn is_float_text(text: &str) -> bool {
    text.contains(['.', 'e', 'E'])
}

fn number_repr(number: &Number) -> String {
    // Avec `arbitrary_precision`, `to_string` rend le texte reçu, sans arrondi :
    // les grands entiers restent exacts, comme en Python.
    let text = number.to_string();
    if !is_float_text(&text) {
        // `-0` est l'entier 0 pour Python.
        return if text.trim_start_matches('-').bytes().all(|b| b == b'0') {
            "0".to_string()
        } else {
            text
        };
    }
    match text.parse::<f64>() {
        Ok(x) => float_repr(x),
        Err(_) => text,
    }
}

/// `repr()` d'un `float` en Python : le plus court texte qui redonne le même
/// nombre, avec `.0` pour un nombre rond, et l'écriture scientifique quand
/// l'exposant est inférieur à -4 ou au moins 16 (`1e+16`, `1e-05`).
pub(crate) fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    // `{:e}` donne déjà les chiffres les plus courts : « 1.5e300 », « -2e-7 ».
    let scientific = format!("{x:e}");
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        return scientific;
    };
    if !(-4..16).contains(&exponent) {
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!("{mantissa}e{sign}{:02}", exponent.abs());
    }
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if exponent >= 0 {
        // `exponent` est entre 0 et 15 : la conversion en `usize` est sûre.
        let int_len = usize::try_from(exponent).unwrap_or(0) + 1;
        if digits.len() <= int_len {
            out.push_str(&digits);
            out.push_str(&"0".repeat(int_len - digits.len()));
            out.push_str(".0");
        } else {
            out.push_str(&digits[..int_len]);
            out.push('.');
            out.push_str(&digits[int_len..]);
        }
    } else {
        let zeros = usize::try_from(-exponent - 1).unwrap_or(0);
        out.push_str("0.");
        out.push_str(&"0".repeat(zeros));
        out.push_str(&digits);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn textes_et_listes() {
        assert_eq!(list_repr(&[]), "[]");
        let tags = vec!["AP".to_string(), "TextOnly".to_string()];
        assert_eq!(list_repr(&tags), "['AP', 'TextOnly']");
        assert_eq!(str_repr("l'eau"), "\"l'eau\"");
        assert_eq!(str_repr("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(str_repr("a\\b\n"), "'a\\\\b\\n'");
    }

    /// Valeurs vérifiées avec `repr()` de Python 3.12.
    #[test]
    fn flottants() {
        let cases = [
            (1.0, "1.0"),
            (0.5, "0.5"),
            (-2.5, "-2.5"),
            (0.1, "0.1"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e+16"),
            (1.5e300, "1.5e+300"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (-1.25e-7, "-1.25e-07"),
            (123.456, "123.456"),
            (-0.0, "-0.0"),
            (1794764264.5, "1794764264.5"),
        ];
        for (x, expected) in cases {
            assert_eq!(float_repr(x), expected, "{x}");
        }
    }

    #[test]
    fn valeurs_json() {
        let value: Value = serde_json::from_str(
            r#"{"pas": "de cmd", "n": 1e300, "i": -0, "g": 123456789012345678901234567890,
                "e": [null, true, false, "l'eau", 2.50], "o": {}}"#,
        )
        .unwrap_or_default();
        assert_eq!(
            repr(&value),
            "{'pas': 'de cmd', 'n': 1e+300, 'i': 0, 'g': 123456789012345678901234567890, \
             'e': [None, True, False, \"l'eau\", 2.5], 'o': {}}"
        );
        assert_eq!(str_of(&json!("texte")), "texte");
        assert_eq!(str_of(&json!([1, "a"])), "[1, 'a']");
    }

    #[test]
    fn types() {
        let value: Value =
            serde_json::from_str("[5, 5.0, 1e3, null, true, [], {}, \"x\"]").unwrap_or_default();
        let names: Vec<&str> = value
            .as_array()
            .map(|items| items.iter().map(type_name).collect())
            .unwrap_or_default();
        assert_eq!(
            names,
            ["int", "float", "float", "NoneType", "bool", "list", "dict", "str"]
        );
    }
}
