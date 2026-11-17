//! Variables des profils (`{game_dir}`, `{address}`…), avec refus des variables inconnues.
//!
//! Syntaxe : `{nom}` est remplacé par la valeur de la variable `nom` ; `{{` donne
//! une accolade ouvrante littérale et `}}` une accolade fermante littérale.

use std::collections::BTreeMap;

/// Ce qui empêche de remplacer les variables d'un modèle.
#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    /// Le modèle cite une variable qui n'existe pas.
    #[error("variable inconnue : {name}")]
    UnknownVariable { name: String },
    /// Une accolade ouverte n'est jamais refermée.
    #[error("accolade non fermée dans le modèle")]
    Unclosed,
    /// La valeur d'une variable contient un caractère interdit (saut de ligne ou NUL).
    #[error("la valeur de la variable {name} contient un caractère interdit")]
    ForbiddenChar { name: String },
}

/// Les valeurs des variables. `BTreeMap` : ordre stable, donc sorties reproductibles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Vars(pub BTreeMap<String, String>);

impl Vars {
    /// Un ensemble vide.
    pub fn new() -> Vars {
        Vars::default()
    }

    /// Ajoute (ou remplace) une variable ; renvoie `self` pour pouvoir enchaîner les appels.
    pub fn with(mut self, name: &str, value: &str) -> Vars {
        self.0.insert(name.to_string(), value.to_string());
        self
    }
}

/// Un morceau du modèle une fois découpé.
// `'a` : durée de vie. `Var` emprunte un morceau du modèle d'origine au lieu de le copier ;
// le compilateur garantit que le modèle vit au moins aussi longtemps que les morceaux.
enum Piece<'a> {
    /// Texte à recopier tel quel.
    Text(String),
    /// Nom d'une variable à remplacer.
    Var(&'a str),
}

/// Découpe le modèle en morceaux. Commun à `expand` et `check`.
fn parse(template: &str) -> Result<Vec<Piece<'_>>, TemplateError> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    // `char_indices` donne la position en octets : on peut ensuite découper la
    // chaîne d'origine sans la copier.
    let mut chars = template.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '{' => {
                if let Some((_, '{')) = chars.peek() {
                    chars.next();
                    text.push('{');
                    continue;
                }
                // Cherche l'accolade fermante.
                let start = i + 1;
                let Some(len) = template[start..].find('}') else {
                    return Err(TemplateError::Unclosed);
                };
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                pieces.push(Piece::Var(&template[start..start + len]));
                // Saute le nom et l'accolade fermante.
                let end = start + len + 1;
                while let Some(&(j, _)) = chars.peek() {
                    if j >= end {
                        break;
                    }
                    chars.next();
                }
            }
            '}' => {
                // `}}` donne une accolade ; une `}` isolée est recopiée telle quelle.
                if let Some((_, '}')) = chars.peek() {
                    chars.next();
                }
                text.push('}');
            }
            other => text.push(other),
        }
    }
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    Ok(pieces)
}

/// Remplace chaque `{nom}` du modèle par sa valeur dans `vars`.
pub fn expand(template: &str, vars: &Vars) -> Result<String, TemplateError> {
    let mut out = String::new();
    for piece in parse(template)? {
        match piece {
            Piece::Text(t) => out.push_str(&t),
            Piece::Var(name) => {
                let Some(value) = vars.0.get(name) else {
                    return Err(TemplateError::UnknownVariable {
                        name: name.to_string(),
                    });
                };
                // Une valeur à plusieurs lignes ou avec NUL permettrait d'injecter
                // des lignes dans un fichier de config ou de tronquer un argument.
                if value.contains(['\n', '\r', '\0']) {
                    return Err(TemplateError::ForbiddenChar {
                        name: name.to_string(),
                    });
                }
                out.push_str(value);
            }
        }
    }
    Ok(out)
}

/// Vérifie qu'un modèle est bien formé et n'utilise que les variables `allowed`
/// (utilisé au chargement d'un profil, sans connaître les valeurs).
pub fn check(template: &str, allowed: &[&str]) -> Result<(), TemplateError> {
    for piece in parse(template)? {
        if let Piece::Var(name) = piece {
            if !allowed.contains(&name) {
                return Err(TemplateError::UnknownVariable {
                    name: name.to_string(),
                });
            }
        }
    }
    Ok(())
}
