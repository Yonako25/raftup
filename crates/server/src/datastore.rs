//! Stockage de données partagé entre les clients (`Get`, `Set`, `SetNotify`).
//!
//! Les clients y rangent des valeurs JSON libres sous une clé texte. `Set`
//! applique une suite d'opérations (`add`, `max`, `update`…) qui imitent les
//! opérateurs de Python du serveur officiel (`modify_functions`, 0.6.8).
//!
//! Une opération impossible (par exemple ajouter un texte à un nombre) fait
//! planter le serveur officiel, qui ferme la connexion : ici, elle renvoie
//! une erreur, et la tâche salle ferme la connexion de la même façon.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde_json::{Map, Number, Value};

/// Taille maximale d'un texte ou d'une liste produits par `mul` : au-delà, on
/// refuse plutôt que de remplir la mémoire (le serveur officiel ne limite pas).
const MAX_REPEAT_LEN: usize = 1 << 20;

/// Opération impossible sur ces valeurs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("opération « {operation} » impossible : {reason}")]
pub(crate) struct OperationError {
    pub operation: String,
    pub reason: String,
}

/// Les valeurs rangées par les clients.
#[derive(Debug, Default)]
pub(crate) struct DataStore {
    values: BTreeMap<String, Value>,
}

impl DataStore {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn set(&mut self, key: String, value: Value) {
        self.values.insert(key, value);
    }

    /// Stockage repris d'une sauvegarde.
    pub fn from_entries(entries: impl Iterator<Item = (String, Value)>) -> DataStore {
        DataStore {
            values: entries.collect(),
        }
    }

    /// Toutes les valeurs, par clé croissante (pour la sauvegarde).
    pub fn entries(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.values.iter()
    }
}

/// Comme [`apply`], et dit en plus si le résultat est le *même objet* Python
/// que `old`.
///
/// Ça compte pour reproduire un effet du serveur officiel : `remove`, `pop` et
/// `update` modifient la liste (ou le dictionnaire) sur place. Si la valeur
/// venait de `default`, le `default` renvoyé dans le `SetReply` est donc
/// modifié lui aussi. `default`, `max` et `min` peuvent aussi rendre l'objet
/// d'origine tel quel.
pub(crate) fn apply_tracked(
    operation: &str,
    old: Value,
    arg: &Value,
) -> Result<(Value, bool), OperationError> {
    let same_object = match operation {
        "default" | "remove" | "pop" | "update" => true,
        "max" => compare(arg, &old) != Some(Ordering::Greater),
        "min" => compare(arg, &old) != Some(Ordering::Less),
        _ => false,
    };
    apply(operation, old, arg).map(|value| (value, same_object))
}

/// Applique une opération de `Set` : `old` est la valeur actuelle, `arg` la
/// valeur donnée par le client.
pub(crate) fn apply(operation: &str, old: Value, arg: &Value) -> Result<Value, OperationError> {
    let fail = |reason: &str| OperationError {
        operation: operation.to_string(),
        reason: reason.to_string(),
    };
    match operation {
        "replace" => Ok(arg.clone()),
        "default" => Ok(old),
        "add" => add(&old, arg).ok_or_else(|| fail("types incompatibles")),
        "mul" => mul(&old, arg).ok_or_else(|| fail("types incompatibles ou résultat trop grand")),
        "pow" => pow(&old, arg).ok_or_else(|| fail("types incompatibles ou résultat invalide")),
        "mod" => modulo(&old, arg).ok_or_else(|| fail("types incompatibles ou division par zéro")),
        "floor" | "ceil" => {
            let up = operation == "ceil";
            round(&old, up).ok_or_else(|| fail("pas un nombre"))
        }
        "max" | "min" => {
            let order = compare(arg, &old).ok_or_else(|| fail("valeurs incomparables"))?;
            let take_new = if operation == "max" {
                order == Ordering::Greater
            } else {
                order == Ordering::Less
            };
            // Égalité : Python garde le premier argument, l'ancienne valeur.
            Ok(if take_new { arg.clone() } else { old })
        }
        "xor" | "or" | "and" => {
            bitwise(operation, &old, arg).ok_or_else(|| fail("types incompatibles"))
        }
        "left_shift" | "right_shift" => {
            let left = operation == "left_shift";
            shift(&old, arg, left).ok_or_else(|| fail("décalage invalide"))
        }
        "remove" => remove(old, arg).ok_or_else(|| fail("pas une liste")),
        "pop" => pop(old, arg).ok_or_else(|| fail("type ou position invalide")),
        "update" => update(old, arg).ok_or_else(|| fail("types incompatibles")),
        _ => Err(fail("opération inconnue")),
    }
}

// ----- Nombres ---------------------------------------------------------------

/// Un nombre comme Python le voit : entier (les booléens comptent pour 0 et 1)
/// ou à virgule.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Num {
    Int(i64),
    Float(f64),
}

fn num(value: &Value) -> Option<Num> {
    match value {
        Value::Bool(b) => Some(Num::Int(i64::from(*b))),
        Value::Number(n) => match n.as_i64() {
            Some(i) => Some(Num::Int(i)),
            // Un entier qui ne tient pas dans un i64 n'est pas géré.
            None if n.is_u64() => None,
            None => n.as_f64().map(Num::Float),
        },
        _ => None,
    }
}

fn as_float(n: Num) -> f64 {
    match n {
        // `as` : conversion volontaire d'entier en flottant, comme Python.
        Num::Int(i) => i as f64,
        Num::Float(f) => f,
    }
}

fn from_num(n: Num) -> Option<Value> {
    match n {
        Num::Int(i) => Some(Value::from(i)),
        // NaN ou infini : pas du JSON valide.
        Num::Float(f) => Number::from_f64(f).map(Value::Number),
    }
}

/// Opération sur deux nombres : entière si les deux sont entiers, sinon à virgule.
fn arith(
    a: &Value,
    b: &Value,
    int_op: impl Fn(i64, i64) -> Option<i64>,
    float_op: impl Fn(f64, f64) -> f64,
) -> Option<Value> {
    let result = match (num(a)?, num(b)?) {
        (Num::Int(x), Num::Int(y)) => Num::Int(int_op(x, y)?),
        (x, y) => Num::Float(float_op(as_float(x), as_float(y))),
    };
    from_num(result)
}

fn add(old: &Value, arg: &Value) -> Option<Value> {
    match (old, arg) {
        (Value::String(a), Value::String(b)) => Some(Value::String(format!("{a}{b}"))),
        (Value::Array(a), Value::Array(b)) => {
            let mut joined = a.clone();
            joined.extend(b.iter().cloned());
            Some(Value::Array(joined))
        }
        _ => arith(old, arg, i64::checked_add, |x, y| x + y),
    }
}

fn mul(old: &Value, arg: &Value) -> Option<Value> {
    // Texte ou liste multiplié par un entier : répétition (rien si ≤ 0).
    let repeat = |count: &Value| match num(count)? {
        Num::Int(n) => Some(usize::try_from(n.max(0)).unwrap_or(0)),
        Num::Float(_) => None,
    };
    match (old, arg) {
        (Value::String(s), n) | (n, Value::String(s)) if !n.is_string() => {
            let times = repeat(n)?;
            (s.len().saturating_mul(times) <= MAX_REPEAT_LEN)
                .then(|| Value::String(s.repeat(times)))
        }
        (Value::Array(items), n) | (n, Value::Array(items)) if !n.is_array() => {
            let times = repeat(n)?;
            if items.len().saturating_mul(times) > MAX_REPEAT_LEN {
                return None;
            }
            let mut out = Vec::with_capacity(items.len() * times);
            for _ in 0..times {
                out.extend(items.iter().cloned());
            }
            Some(Value::Array(out))
        }
        _ => arith(old, arg, i64::checked_mul, |x, y| x * y),
    }
}

fn pow(old: &Value, arg: &Value) -> Option<Value> {
    let result = match (num(old)?, num(arg)?) {
        (Num::Int(base), Num::Int(exp)) if exp >= 0 => {
            Num::Int(base.checked_pow(u32::try_from(exp).ok()?)?)
        }
        (x, y) => {
            let (x, y) = (as_float(x), as_float(y));
            // 0 puissance négative : division par zéro en Python.
            // Négatif puissance non entière : nombre complexe en Python.
            if (x == 0.0 && y < 0.0) || (x < 0.0 && y.fract() != 0.0) {
                return None;
            }
            Num::Float(x.powf(y))
        }
    };
    from_num(result)
}

/// Modulo de Python : le résultat a le signe du diviseur (`-7 % 3 == 2`).
fn modulo(old: &Value, arg: &Value) -> Option<Value> {
    arith(
        old,
        arg,
        |a, b| {
            let r = a.checked_rem(b)?;
            Some(if r != 0 && ((r < 0) != (b < 0)) {
                r + b
            } else {
                r
            })
        },
        |a, b| {
            let r = a % b;
            if r != 0.0 && ((r < 0.0) != (b < 0.0)) {
                r + b
            } else {
                r
            }
        },
    )
    // Division par zéro à virgule : NaN, refusé par `from_num`.
}

/// `math.floor` ou `math.ceil` : renvoie toujours un entier.
fn round(value: &Value, up: bool) -> Option<Value> {
    match num(value)? {
        Num::Int(i) => Some(Value::from(i)),
        Num::Float(f) => {
            let r = if up { f.ceil() } else { f.floor() };
            // Hors des bornes d'un i64 (ou NaN) : non géré.
            if !(-9.2e18..=9.2e18).contains(&r) {
                return None;
            }
            // `as` : la valeur est déjà entière et dans les bornes.
            Some(Value::from(r as i64))
        }
    }
}

fn bitwise(operation: &str, old: &Value, arg: &Value) -> Option<Value> {
    // Deux booléens : le résultat reste un booléen, comme en Python.
    if let (Value::Bool(a), Value::Bool(b)) = (old, arg) {
        return Some(Value::Bool(match operation {
            "xor" => a ^ b,
            "or" => *a || *b,
            _ => *a && *b,
        }));
    }
    // `or` de deux dictionnaires : fusion (Python 3.9+), le second l'emporte.
    if let (Value::Object(a), Value::Object(b), "or") = (old, arg, operation) {
        let mut merged = a.clone();
        for (k, v) in b {
            merged.insert(k.clone(), v.clone());
        }
        return Some(Value::Object(merged));
    }
    let (Num::Int(a), Num::Int(b)) = (num(old)?, num(arg)?) else {
        return None;
    };
    Some(Value::from(match operation {
        "xor" => a ^ b,
        "or" => a | b,
        _ => a & b,
    }))
}

fn shift(old: &Value, arg: &Value, left: bool) -> Option<Value> {
    let (Num::Int(a), Num::Int(n)) = (num(old)?, num(arg)?) else {
        return None;
    };
    // Décalage négatif : erreur en Python.
    let n = u32::try_from(n).ok()?;
    if left {
        // Les bits perdus changeraient la valeur : Python passerait en grand entier.
        let shifted = a.checked_shl(n)?;
        (shifted >> n == a).then_some(Value::from(shifted))
    } else {
        // Décalage à droite arithmétique, arrondi vers le bas comme en Python.
        Some(Value::from(a >> n.min(63)))
    }
}

// ----- Listes et dictionnaires -----------------------------------------------

/// `list.remove` : retire la première valeur égale, s'il y en a une.
fn remove(old: Value, arg: &Value) -> Option<Value> {
    let Value::Array(mut items) = old else {
        return None;
    };
    if let Some(position) = items.iter().position(|item| python_eq(item, arg)) {
        items.remove(position);
    }
    Some(Value::Array(items))
}

/// `pop` : retire l'élément à une position (liste) ou une clé (dictionnaire).
fn pop(old: Value, arg: &Value) -> Option<Value> {
    match old {
        Value::Array(mut items) => {
            let Some(Num::Int(index)) = num(arg) else {
                return None;
            };
            let len = i64::try_from(items.len()).ok()?;
            if index >= len {
                // Position trop grande : liste inchangée.
                return Some(Value::Array(items));
            }
            // Position négative : depuis la fin, comme en Python.
            let real = if index < 0 { index + len } else { index };
            items.remove(usize::try_from(real).ok()?);
            Some(Value::Array(items))
        }
        Value::Object(mut map) => {
            // Clé absente (ou pas un texte, donc forcément absente) : inchangé.
            if let Value::String(key) = arg {
                map.shift_remove(key);
            }
            Some(Value::Object(map))
        }
        _ => None,
    }
}

/// `update` : ajoute à une liste les éléments absents, ou fusionne deux dictionnaires.
fn update(old: Value, arg: &Value) -> Option<Value> {
    match old {
        Value::Array(mut items) => {
            // Python construit un ensemble de la liste : ses éléments doivent
            // être « hachables » (ni liste, ni dictionnaire).
            if items.iter().any(|i| i.is_array() || i.is_object()) {
                return None;
            }
            let entries: Vec<Value> = match arg {
                Value::Array(entries) => entries.clone(),
                // Parcourir un dictionnaire donne ses clés.
                Value::Object(map) => map.keys().cloned().map(Value::String).collect(),
                Value::String(s) => s.chars().map(|c| Value::String(c.to_string())).collect(),
                _ => return None,
            };
            let existing = items.clone();
            for entry in entries {
                if !existing.iter().any(|e| python_eq(e, &entry)) {
                    items.push(entry);
                }
            }
            Some(Value::Array(items))
        }
        Value::Object(mut map) => {
            let Value::Object(entries) = arg else {
                return None;
            };
            for (k, v) in entries {
                map.insert(k.clone(), v.clone());
            }
            Some(Value::Object(map))
        }
        _ => None,
    }
}

// ----- Comparaisons ------------------------------------------------------------

/// Égalité de Python : `1 == 1.0 == True`.
fn python_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| python_eq(p, q))
        }
        (Value::Object(x), Value::Object(y)) => objects_eq(x, y),
        _ => match (num(a), num(b)) {
            (Some(x), Some(y)) => compare_nums(x, y) == Ordering::Equal,
            _ => a == b,
        },
    }
}

fn objects_eq(x: &Map<String, Value>, y: &Map<String, Value>) -> bool {
    x.len() == y.len()
        && x.iter()
            .all(|(k, v)| y.get(k).is_some_and(|w| python_eq(v, w)))
}

fn compare_nums(x: Num, y: Num) -> Ordering {
    match (x, y) {
        (Num::Int(a), Num::Int(b)) => a.cmp(&b),
        _ => as_float(x)
            .partial_cmp(&as_float(y))
            .unwrap_or(Ordering::Equal),
    }
}

/// Ordre de Python pour `max` et `min` : nombres entre eux, textes entre eux,
/// listes élément par élément. `None` si Python lèverait une erreur.
fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        (Value::Array(x), Value::Array(y)) => {
            for (p, q) in x.iter().zip(y) {
                if python_eq(p, q) {
                    continue;
                }
                return compare(p, q);
            }
            Some(x.len().cmp(&y.len()))
        }
        _ => Some(compare_nums(num(a)?, num(b)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn op(operation: &str, old: Value, arg: Value) -> Option<Value> {
        apply(operation, old, &arg).ok()
    }

    #[test]
    fn operations_numeriques() {
        assert_eq!(op("add", json!(2), json!(3)), Some(json!(5)));
        assert_eq!(op("add", json!(2), json!(0.5)), Some(json!(2.5)));
        assert_eq!(op("add", json!(true), json!(1)), Some(json!(2)));
        assert_eq!(op("mul", json!(4), json!(-2)), Some(json!(-8)));
        assert_eq!(op("pow", json!(2), json!(10)), Some(json!(1024)));
        assert_eq!(op("pow", json!(2), json!(-1)), Some(json!(0.5)));
        assert_eq!(op("mod", json!(-7), json!(3)), Some(json!(2)));
        assert_eq!(op("mod", json!(7), json!(-3)), Some(json!(-2)));
        assert_eq!(op("mod", json!(7), json!(0)), None);
        assert_eq!(op("floor", json!(2.7), json!(null)), Some(json!(2)));
        assert_eq!(op("ceil", json!(-2.7), json!(null)), Some(json!(-2)));
        assert_eq!(op("max", json!(3), json!(5)), Some(json!(5)));
        assert_eq!(op("min", json!(3), json!(5)), Some(json!(3)));
        assert_eq!(op("max", json!(3), json!(3.0)), Some(json!(3)));
        assert_eq!(op("xor", json!(6), json!(3)), Some(json!(5)));
        assert_eq!(op("or", json!(true), json!(false)), Some(json!(true)));
        assert_eq!(op("left_shift", json!(1), json!(4)), Some(json!(16)));
        assert_eq!(op("right_shift", json!(-9), json!(1)), Some(json!(-5)));
        assert_eq!(op("left_shift", json!(1), json!(-1)), None);
        assert_eq!(op("add", json!(1), json!("a")), None);
        assert_eq!(op("inconnue", json!(1), json!(1)), None);
    }

    #[test]
    fn operations_textes_et_listes() {
        assert_eq!(op("add", json!("ab"), json!("cd")), Some(json!("abcd")));
        assert_eq!(op("mul", json!("ab"), json!(3)), Some(json!("ababab")));
        assert_eq!(op("mul", json!([1]), json!(2)), Some(json!([1, 1])));
        assert_eq!(op("add", json!([1]), json!([2])), Some(json!([1, 2])));
        assert_eq!(
            op("remove", json!([1, 2, 1]), json!(1.0)),
            Some(json!([2, 1]))
        );
        assert_eq!(op("remove", json!([1]), json!(5)), Some(json!([1])));
        assert_eq!(op("pop", json!([1, 2, 3]), json!(-1)), Some(json!([1, 2])));
        assert_eq!(
            op("pop", json!([1, 2, 3]), json!(9)),
            Some(json!([1, 2, 3]))
        );
        assert_eq!(
            op("pop", json!({"a": 1, "b": 2}), json!("a")),
            Some(json!({"b": 2}))
        );
        assert_eq!(
            op("update", json!([1, 2]), json!([2, 3, 3])),
            Some(json!([1, 2, 3, 3]))
        );
        assert_eq!(
            op("update", json!({"a": 1}), json!({"b": 2})),
            Some(json!({"a": 1, "b": 2}))
        );
        assert_eq!(
            op("or", json!({"a": 1}), json!({"a": 2})),
            Some(json!({"a": 2}))
        );
        assert_eq!(op("max", json!("abc"), json!("abd")), Some(json!("abd")));
        assert_eq!(op("max", json!([1, 2]), json!([1, 3])), Some(json!([1, 3])));
        assert_eq!(op("update", json!([[1]]), json!([2])), None);
    }
}
