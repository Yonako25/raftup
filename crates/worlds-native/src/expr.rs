//! Le mini-langage de règles : découpage en jetons, analyse, résolution des noms.
//!
//! Grammaire (voir `spec/monde-natif/README.md`) :
//!
//! ```text
//! expr       = or
//! or         = and { "or" and }
//! and        = unary { "and" unary }
//! unary      = "not" unary | comparison
//! comparison = primary [ op value ]
//! primary    = "true" | "false" | "(" expr ")" | call
//! ```
//!
//! L'analyse est une « descente récursive » : une fonction par ligne de la grammaire, qui
//! appelle celle de la ligne suivante. La profondeur est bornée ([`crate::MAX_DEPTH`]) : un
//! texte hostile très imbriqué ne fait pas déborder la pile.

use std::collections::BTreeMap;

use crate::model::{CompareOp, Count, Expr, ItemRef, LocationRef, OptionDef, OptionKind};
use crate::MAX_DEPTH;

/// Erreur dans une expression : `offset` est l'octet fautif dans le texte de l'expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExprError {
    pub offset: usize,
    pub message: String,
}

fn fail<T>(offset: usize, message: impl Into<String>) -> Result<T, ExprError> {
    Err(ExprError {
        offset,
        message: message.into(),
    })
}

/// Les noms qu'une règle peut citer, rangés pour la recherche.
#[derive(Debug, Default)]
pub(crate) struct Names {
    pub options: BTreeMap<String, u32>,
    pub option_defs: Vec<OptionDef>,
    pub items: BTreeMap<String, ItemRef>,
    pub groups: BTreeMap<String, u32>,
    pub regions: BTreeMap<String, u32>,
    pub locations: BTreeMap<String, LocationRef>,
    pub entrances: BTreeMap<String, u32>,
}

// ---- Jetons ----

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Str(String),
    Int(i64),
    LParen,
    RParen,
    Comma,
    Op(CompareOp),
    End,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    offset: usize,
}

/// Découpe le texte en jetons.
fn lex(text: &str) -> Result<Vec<Token>, ExprError> {
    let mut tokens = Vec::new();
    // `peekable` permet de regarder le caractère suivant sans le consommer.
    let mut chars = text.char_indices().peekable();
    while let Some(&(offset, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let tok = match c {
            '(' => {
                chars.next();
                Tok::LParen
            }
            ')' => {
                chars.next();
                Tok::RParen
            }
            ',' => {
                chars.next();
                Tok::Comma
            }
            '=' | '!' | '<' | '>' => {
                chars.next();
                let equal = matches!(chars.peek(), Some(&(_, '=')));
                if equal {
                    chars.next();
                }
                match (c, equal) {
                    ('=', true) => Tok::Op(CompareOp::Eq),
                    ('!', true) => Tok::Op(CompareOp::Ne),
                    ('<', true) => Tok::Op(CompareOp::Le),
                    ('<', false) => Tok::Op(CompareOp::Lt),
                    ('>', true) => Tok::Op(CompareOp::Ge),
                    ('>', false) => Tok::Op(CompareOp::Gt),
                    ('=', false) => return fail(offset, "« = » seul : écrire « == »"),
                    _ => return fail(offset, "« ! » seul : écrire « != » ou « not »"),
                }
            }
            '"' => {
                chars.next();
                let mut value = String::new();
                let mut closed = false;
                while let Some((at, c)) = chars.next() {
                    match c {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '\\' => match chars.next() {
                            Some((_, '"')) => value.push('"'),
                            Some((_, '\\')) => value.push('\\'),
                            _ => return fail(at, "échappement inconnu (seuls \\\" et \\\\)"),
                        },
                        _ => value.push(c),
                    }
                }
                if !closed {
                    return fail(offset, "guillemet ouvert jamais fermé");
                }
                Tok::Str(value)
            }
            c if c.is_ascii_digit() || c == '-' => {
                let mut digits = String::new();
                digits.push(c);
                chars.next();
                while let Some(&(_, d)) = chars.peek() {
                    if !d.is_ascii_digit() {
                        break;
                    }
                    digits.push(d);
                    chars.next();
                }
                match digits.parse::<i64>() {
                    Ok(n) => Tok::Int(n),
                    Err(_) => return fail(offset, format!("nombre « {digits} » invalide")),
                }
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut word = String::new();
                while let Some(&(_, d)) = chars.peek() {
                    if !(d.is_alphanumeric() || d == '_') {
                        break;
                    }
                    word.push(d);
                    chars.next();
                }
                Tok::Ident(word)
            }
            other => return fail(offset, format!("caractère « {other} » inattendu")),
        };
        tokens.push(Token { tok, offset });
    }
    tokens.push(Token {
        tok: Tok::End,
        offset: text.len(),
    });
    Ok(tokens)
}

// ---- Analyse ----

/// Un argument de fonction, avant d'en connaître le rôle.
enum Arg {
    Name(String),
    Int(i64),
    Option(u32),
}

struct Parser<'a> {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
    names: &'a Names,
}

/// Analyse une expression et résout ses noms.
pub(crate) fn parse(text: &str, names: &Names) -> Result<Expr, ExprError> {
    let tokens = lex(text)?;
    let mut parser = Parser {
        tokens,
        at: 0,
        depth: 0,
        names,
    };
    let expr = parser.or()?;
    let token = parser.peek();
    if token.tok != Tok::End {
        return fail(token.offset, "texte en trop après la règle");
    }
    Ok(expr)
}

/// Mots réservés : ils ne désignent jamais un objet ou une région sans guillemets.
fn is_keyword(word: &str) -> bool {
    matches!(word, "and" | "or" | "not" | "true" | "false")
}

impl Parser<'_> {
    /// Jeton courant (le dernier, `End`, est toujours là).
    fn peek(&self) -> Token {
        // `get` renvoie `None` au-delà de la fin ; on retombe alors sur `End`.
        self.tokens
            .get(self.at)
            .or_else(|| self.tokens.last())
            .cloned()
            .unwrap_or(Token {
                tok: Tok::End,
                offset: 0,
            })
    }

    fn next(&mut self) -> Token {
        let token = self.peek();
        if self.at < self.tokens.len() {
            self.at += 1;
        }
        token
    }

    fn is_word(&self, word: &str) -> bool {
        matches!(&self.peek().tok, Tok::Ident(w) if w == word)
    }

    fn expect(&mut self, tok: Tok, what: &str) -> Result<(), ExprError> {
        let token = self.next();
        if token.tok == tok {
            Ok(())
        } else {
            fail(token.offset, format!("{what} attendu"))
        }
    }

    /// Entre dans un niveau d'imbrication ; erreur au-delà de [`MAX_DEPTH`].
    fn enter(&mut self, offset: usize) -> Result<(), ExprError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return fail(
                offset,
                format!("règle trop imbriquée (plus de {MAX_DEPTH} niveaux)"),
            );
        }
        Ok(())
    }

    fn or(&mut self) -> Result<Expr, ExprError> {
        let mut parts = vec![self.and()?];
        while self.is_word("or") {
            self.next();
            parts.push(self.and()?);
        }
        Ok(if parts.len() == 1 {
            parts.remove(0)
        } else {
            Expr::Or(parts)
        })
    }

    fn and(&mut self) -> Result<Expr, ExprError> {
        let mut parts = vec![self.unary()?];
        while self.is_word("and") {
            self.next();
            parts.push(self.unary()?);
        }
        Ok(if parts.len() == 1 {
            parts.remove(0)
        } else {
            Expr::And(parts)
        })
    }

    fn unary(&mut self) -> Result<Expr, ExprError> {
        if !self.is_word("not") {
            return self.comparison();
        }
        let offset = self.next().offset;
        self.enter(offset)?;
        let inner = self.unary()?;
        self.depth -= 1;
        if !inner.is_static() {
            return fail(
                offset,
                "« not » ne s'applique qu'à une condition sur les options \
                 (une règle d'accès ne peut que devenir vraie quand l'inventaire grandit)",
            );
        }
        Ok(Expr::Not(Box::new(inner)))
    }

    fn comparison(&mut self) -> Result<Expr, ExprError> {
        let start = self.peek().offset;
        let is_option_call = self.is_word("option")
            && matches!(self.tokens.get(self.at + 1), Some(t) if t.tok == Tok::LParen);
        if !is_option_call {
            let expr = self.primary()?;
            let token = self.peek();
            if let Tok::Op(op) = token.tok {
                return fail(
                    token.offset,
                    format!("« {} » : seule une option se compare", op.symbol()),
                );
            }
            return Ok(expr);
        }
        self.next();
        let option = self.option_call()?;
        let def = self.option_def(option);
        let token = self.peek();
        let Tok::Op(op) = token.tok else {
            // Sans comparaison : seule une option oui/non a un sens.
            return match def.map(|d| &d.kind) {
                Some(OptionKind::Toggle) => Ok(Expr::Toggle(option)),
                _ => fail(
                    start,
                    "une option numérique ou à choix se compare : option(nom) == valeur",
                ),
            };
        };
        self.next();
        let value_token = self.next();
        let value = self.compare_value(option, &value_token)?;
        Ok(Expr::Compare { option, op, value })
    }

    /// Lit `(nom)` après le mot `option` ; renvoie la position de l'option.
    fn option_call(&mut self) -> Result<u32, ExprError> {
        self.expect(Tok::LParen, "« ( »")?;
        let token = self.next();
        let name = match token.tok {
            Tok::Ident(name) | Tok::Str(name) => name,
            _ => return fail(token.offset, "nom d'option attendu"),
        };
        let Some(&option) = self.names.options.get(&name) else {
            return fail(token.offset, format!("option « {name} » inconnue"));
        };
        self.expect(Tok::RParen, "« ) »")?;
        Ok(option)
    }

    fn option_def(&self, option: u32) -> Option<&OptionDef> {
        self.names.option_defs.get(option as usize)
    }

    /// Valeur à droite d'une comparaison, convertie en entier selon le type de l'option.
    fn compare_value(&self, option: u32, token: &Token) -> Result<i64, ExprError> {
        let Some(def) = self.option_def(option) else {
            return fail(token.offset, "option inconnue");
        };
        match (&def.kind, &token.tok) {
            (OptionKind::Choice { choices, .. }, Tok::Int(n)) => {
                if *n >= 0 && (*n as usize) < choices.len() {
                    Ok(*n)
                } else {
                    fail(token.offset, format!("choix n° {n} inexistant"))
                }
            }
            (_, Tok::Int(n)) => Ok(*n),
            (OptionKind::Toggle, Tok::Ident(w)) if w == "true" => Ok(1),
            (OptionKind::Toggle, Tok::Ident(w)) if w == "false" => Ok(0),
            (OptionKind::Choice { choices, aliases }, Tok::Ident(name) | Tok::Str(name)) => {
                crate::load::choice_index(choices, aliases, name).ok_or_else(|| ExprError {
                    offset: token.offset,
                    message: format!("choix « {name} » inconnu pour l'option « {} »", def.name),
                })
            }
            _ => fail(token.offset, "valeur attendue après la comparaison"),
        }
    }

    fn primary(&mut self) -> Result<Expr, ExprError> {
        let token = self.next();
        match token.tok {
            Tok::Ident(w) if w == "true" => Ok(Expr::Const(true)),
            Tok::Ident(w) if w == "false" => Ok(Expr::Const(false)),
            Tok::LParen => {
                self.enter(token.offset)?;
                let inner = self.or()?;
                self.depth -= 1;
                self.expect(Tok::RParen, "« ) »")?;
                Ok(inner)
            }
            Tok::Ident(function) if !is_keyword(&function) => {
                self.enter(token.offset)?;
                let args = self.args()?;
                self.depth -= 1;
                self.call(&function, token.offset, args)
            }
            Tok::End => fail(token.offset, "règle incomplète"),
            _ => fail(token.offset, "fonction, « ( », true ou false attendu"),
        }
    }

    /// Lit `( arg, arg, … )`. Chaque argument garde son octet de départ.
    fn args(&mut self) -> Result<Vec<(Arg, usize)>, ExprError> {
        self.expect(Tok::LParen, "« ( » après le nom de la fonction")?;
        let mut args = Vec::new();
        if self.peek().tok == Tok::RParen {
            self.next();
            return Ok(args);
        }
        loop {
            let token = self.next();
            let arg = match token.tok {
                Tok::Str(name) => Arg::Name(name),
                Tok::Int(n) => Arg::Int(n),
                Tok::Ident(w) if w == "option" && matches!(self.peek().tok, Tok::LParen) => {
                    Arg::Option(self.option_call()?)
                }
                Tok::Ident(w) if !is_keyword(&w) => Arg::Name(w),
                _ => return fail(token.offset, "argument attendu (nom, nombre ou option(…))"),
            };
            args.push((arg, token.offset));
            let token = self.next();
            match token.tok {
                Tok::Comma => continue,
                Tok::RParen => break,
                _ => return fail(token.offset, "« , » ou « ) » attendu"),
            }
        }
        Ok(args)
    }

    fn call(
        &self,
        function: &str,
        offset: usize,
        args: Vec<(Arg, usize)>,
    ) -> Result<Expr, ExprError> {
        let count = |n: usize| -> Result<(), ExprError> {
            if args.len() == n {
                Ok(())
            } else {
                fail(
                    offset,
                    format!("{function} attend {n} argument(s), {} donné(s)", args.len()),
                )
            }
        };
        match function {
            "has" => {
                if args.is_empty() || args.len() > 2 {
                    return fail(offset, "has attend un objet et, au besoin, un nombre");
                }
                let mut args = args.into_iter();
                let item = self.item(args.next())?;
                let count = match args.next() {
                    Some(arg) => self.count(arg)?,
                    None => Count::Fixed(1),
                };
                Ok(Expr::Has { item, count })
            }
            "has_count" => {
                count(2)?;
                let mut args = args.into_iter();
                let item = self.item(args.next())?;
                let count = match args.next() {
                    Some(arg) => self.count(arg)?,
                    None => Count::Fixed(1),
                };
                Ok(Expr::Has { item, count })
            }
            "has_all" | "has_any" => {
                if args.is_empty() {
                    return fail(offset, format!("{function} attend au moins un objet"));
                }
                let items = args
                    .into_iter()
                    .map(|arg| self.item(Some(arg)))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(if function == "has_all" {
                    Expr::HasAll(items)
                } else {
                    Expr::HasAny(items)
                })
            }
            "has_group" => {
                if args.is_empty() || args.len() > 2 {
                    return fail(
                        offset,
                        "has_group attend un groupe et, au besoin, un nombre",
                    );
                }
                let mut args = args.into_iter();
                let (name, at) = self.name(args.next(), offset)?;
                let Some(&group) = self.names.groups.get(&name) else {
                    return fail(at, format!("groupe d'objets « {name} » inconnu"));
                };
                let count = match args.next() {
                    Some(arg) => self.count(arg)?,
                    None => Count::Fixed(1),
                };
                Ok(Expr::HasGroup { group, count })
            }
            "can_reach" => {
                count(1)?;
                let (name, at) = self.name(args.into_iter().next(), offset)?;
                let region = self.names.regions.get(&name);
                let location = self.names.locations.get(&name);
                match (region, location) {
                    (Some(_), Some(_)) => fail(
                        at,
                        format!(
                            "« {name} » est une région et un lieu : écrire \
                             can_reach_region ou can_reach_location"
                        ),
                    ),
                    (Some(&r), None) => Ok(Expr::CanReachRegion(r)),
                    (None, Some(&l)) => Ok(Expr::CanReachLocation(l)),
                    (None, None) => fail(at, format!("région ou lieu « {name} » inconnu")),
                }
            }
            "can_reach_region" => {
                count(1)?;
                let (name, at) = self.name(args.into_iter().next(), offset)?;
                match self.names.regions.get(&name) {
                    Some(&r) => Ok(Expr::CanReachRegion(r)),
                    None => fail(at, format!("région « {name} » inconnue")),
                }
            }
            "can_reach_location" => {
                count(1)?;
                let (name, at) = self.name(args.into_iter().next(), offset)?;
                match self.names.locations.get(&name) {
                    Some(&l) => Ok(Expr::CanReachLocation(l)),
                    None => fail(at, format!("lieu « {name} » inconnu")),
                }
            }
            "can_reach_entrance" => {
                count(1)?;
                let (name, at) = self.name(args.into_iter().next(), offset)?;
                match self.names.entrances.get(&name) {
                    Some(&e) => Ok(Expr::CanReachEntrance(e)),
                    None => fail(at, format!("entrée « {name} » inconnue")),
                }
            }
            _ => fail(
                offset,
                format!(
                    "fonction « {function} » inconnue (has, has_count, has_all, has_any, \
                     has_group, can_reach, can_reach_region, can_reach_location, \
                     can_reach_entrance, option)"
                ),
            ),
        }
    }

    /// L'argument doit être un nom ; renvoie le nom et son octet de départ.
    fn name(&self, arg: Option<(Arg, usize)>, offset: usize) -> Result<(String, usize), ExprError> {
        match arg {
            Some((Arg::Name(name), at)) => Ok((name, at)),
            Some((_, at)) => fail(at, "nom attendu"),
            None => fail(offset, "nom attendu"),
        }
    }

    fn item(&self, arg: Option<(Arg, usize)>) -> Result<ItemRef, ExprError> {
        let (name, at) = self.name(arg, 0)?;
        match self.names.items.get(&name) {
            Some(&item) => Ok(item),
            None => fail(at, format!("objet « {name} » inconnu")),
        }
    }

    fn count(&self, (arg, at): (Arg, usize)) -> Result<Count, ExprError> {
        match arg {
            Arg::Int(n) => match u32::try_from(n) {
                Ok(n) => Ok(Count::Fixed(n)),
                Err(_) => fail(at, format!("nombre {n} hors bornes (0 à {})", u32::MAX)),
            },
            Arg::Option(option) => match self.option_def(option).map(|d| &d.kind) {
                Some(OptionKind::Range { .. }) => Ok(Count::Option(option)),
                _ => fail(at, "seule une option numérique (range) donne un nombre"),
            },
            Arg::Name(name) => fail(at, format!("nombre attendu, « {name} » trouvé")),
        }
    }
}

/// Analyse un « nombre » : un entier ou `option(nom)` d'une option `range`.
pub(crate) fn parse_count(text: &str, names: &Names) -> Result<Count, ExprError> {
    let tokens = lex(text)?;
    let mut parser = Parser {
        tokens,
        at: 0,
        depth: 0,
        names,
    };
    let token = parser.next();
    let arg = match token.tok {
        Tok::Int(n) => Arg::Int(n),
        Tok::Ident(w) if w == "option" => Arg::Option(parser.option_call()?),
        _ => return fail(token.offset, "nombre ou option(nom) attendu"),
    };
    let count = parser.count((arg, token.offset))?;
    let end = parser.peek();
    if end.tok != Tok::End {
        return fail(end.offset, "texte en trop après le nombre");
    }
    Ok(count)
}
