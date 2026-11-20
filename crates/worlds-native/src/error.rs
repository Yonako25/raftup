//! Erreurs de la lecture et de la compilation d'un monde natif.

use std::fmt;

use thiserror::Error;

/// Une position dans le fichier : ligne et colonne, comptées à partir de 1.
///
/// La colonne compte des caractères (pas des octets) : « é » compte pour un.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

impl Position {
    /// Position de l'octet `offset` dans `text`.
    ///
    /// Un `offset` au-delà de la fin (ou au milieu d'un caractère) est ramené au dernier
    /// caractère entier qui le précède : jamais de panique.
    pub fn of(text: &str, offset: usize) -> Position {
        let mut line = 1;
        let mut column = 1;
        // `char_indices` donne chaque caractère avec son octet de départ.
        for (start, c) in text.char_indices() {
            if start >= offset {
                break;
            }
            if c == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        Position { line, column }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ligne {}, colonne {}", self.line, self.column)
    }
}

/// Écrit « ligne L, colonne C : message », ou seulement le message sans position.
fn located(pos: &Option<Position>, message: &str) -> String {
    match pos {
        Some(pos) => format!("{pos} : {message}"),
        None => message.to_string(),
    }
}

/// Le fichier n'a pas pu être lu comme un monde natif.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LoadError {
    #[error("fichier trop gros : {size} octets (au plus {max})")]
    TooLarge { size: u64, max: usize },
    #[error("lecture de {path} : {message}")]
    Io { path: String, message: String },
    /// Le texte n'est pas du TOML valide, ou un champ a le mauvais type.
    #[error("{}", located(.pos, .message))]
    Toml {
        pos: Option<Position>,
        message: String,
    },
    #[error(
        "format_version {found} inconnu (versions comprises : {} à {})",
        crate::MIN_FORMAT_VERSION,
        crate::FORMAT_VERSION
    )]
    Format { found: i64 },
    /// Le TOML est bien formé, mais le monde est incohérent (nom en double, référence
    /// inconnue, règle mal typée…).
    #[error("{}", located(.pos, .message))]
    Invalid {
        pos: Option<Position>,
        message: String,
    },
}

impl LoadError {
    /// Position de l'erreur dans le fichier, si on la connaît.
    pub fn position(&self) -> Option<Position> {
        match self {
            LoadError::Toml { pos, .. } | LoadError::Invalid { pos, .. } => *pos,
            _ => None,
        }
    }
}

/// Le monde n'a pas pu être construit avec ces options.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BuildError {
    /// Valeur d'option refusée (hors bornes, choix inconnu).
    #[error("option « {option} » : {message}")]
    BadOption { option: String, message: String },
    /// Avec ces options, le monde est incohérent (lieu dans une région absente…).
    #[error("{0}")]
    Invalid(String),
    #[error("monde mal construit : {0}")]
    Logic(#[from] logic::LogicError),
}
