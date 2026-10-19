//! Localized formula syntax accepted at user-entry boundaries; file syntax stays English.

use crate::{Expr, ParseError, parser};

/// Formula-entry syntax of a regional format ([`FormulaLocale::of`]). Every difference between
/// locales is a field of its [`Syntax`] row; nothing else matches on the variant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FormulaLocale {
    #[default]
    En,
    Es,
    Vi,
}

/// One locale's formula notation.
struct Syntax {
    list: char,
    decimal: char,
    array_column: char,
    array_row: char,
    /// Canonical → localized function names; empty where Excel keeps the English names.
    functions: &'static [(&'static str, &'static str)],
    /// Localized TRUE and FALSE, where they differ.
    booleans: Option<(&'static str, &'static str)>,
}

const EN: Syntax = Syntax { list: ',', decimal: '.', array_column: ',', array_row: ';', functions: &[], booleans: None };
const ES: Syntax = Syntax { list: ';', decimal: ',', array_column: '\\', array_row: ';', functions: SPANISH, booleans: Some(("VERDADERO", "FALSO")) };
// Vietnamese Excel keeps English function names and TRUE/FALSE. With `,` as the decimal point,
// array columns take `\` and rows `;`, as in other decimal-comma locales (Excel's
// International(xlColumnSeparator); e.g. `=ADDRESS(33;{1\2;3\4})`, Microsoft Tech Community #3731448).
const VI: Syntax = Syntax { list: ';', decimal: ',', array_column: '\\', array_row: ';', functions: &[], booleans: None };

const SPANISH: &[(&str, &str)] = &[
    ("SUM", "SUMA"),
    ("AVERAGE", "PROMEDIO"),
    ("IF", "SI"),
    ("COUNT", "CONTAR"),
    ("SUMIF", "SUMAR.SI"),
    ("COUNTIF", "CONTAR.SI"),
    ("VLOOKUP", "BUSCARV"),
    ("TODAY", "HOY"),
    ("TRUE", "VERDADERO"),
    ("FALSE", "FALSO"),
];

impl FormulaLocale {
    /// The formula syntax of a regional format.
    pub const fn of(region: gridcraft_core::Region) -> Self {
        match region {
            gridcraft_core::Region::EnUs => Self::En,
            gridcraft_core::Region::EsEs => Self::Es,
            gridcraft_core::Region::ViVn => Self::Vi,
        }
    }
    const fn syntax(self) -> &'static Syntax {
        match self {
            Self::En => &EN,
            Self::Es => &ES,
            Self::Vi => &VI,
        }
    }
    pub fn list_separator(self) -> char {
        self.syntax().list
    }
    pub fn decimal_separator(self) -> char {
        self.syntax().decimal
    }
    pub fn array_column_separator(self) -> char {
        self.syntax().array_column
    }
    pub fn array_row_separator(self) -> char {
        self.syntax().array_row
    }
    /// Whether entry and display are the canonical file syntax (nothing to convert).
    pub fn is_canonical(self) -> bool {
        let s = self.syntax();
        (s.list, s.decimal, s.array_column, s.array_row) == (EN.list, EN.decimal, EN.array_column, EN.array_row) && !self.translates_names()
    }
    fn translates_names(self) -> bool {
        let s = self.syntax();
        !s.functions.is_empty() || s.booleans.is_some()
    }
    pub fn function_name(self, canonical: &str) -> &str {
        self.syntax().functions.iter().find(|(en, _)| en.eq_ignore_ascii_case(canonical)).map(|(_, local)| *local).unwrap_or(canonical)
    }
    pub fn canonical_name(self, localized: &str) -> &str {
        self.syntax().functions.iter().find(|(_, local)| local.eq_ignore_ascii_case(localized)).map(|(en, _)| *en).unwrap_or(localized)
    }
    pub(crate) fn has_boolean_aliases(self) -> bool {
        self.syntax().booleans.is_some()
    }
    pub(crate) fn boolean_alias(self, name: &str) -> Option<bool> {
        let (t, f) = self.syntax().booleans?;
        if name.eq_ignore_ascii_case(t) {
            Some(true)
        } else if name.eq_ignore_ascii_case(f) {
            Some(false)
        } else {
            None
        }
    }
    fn boolean_name(self, value: bool) -> Option<&'static str> {
        self.syntax().booleans.map(|(t, f)| if value { t } else { f })
    }
}

/// Parses UI input using the selected function table and separators. Existing names and
/// LET/LAMBDA bindings take precedence over aliases. File and API syntax uses [`crate::parse`].
pub fn parse_input(src: &str, locale: FormulaLocale, known_name: impl Fn(&str) -> bool) -> Result<Expr, ParseError> {
    let mut expr = parser::parse_syntax(src, locale)?;
    if locale.translates_names() {
        normalize(&mut expr, locale, &known_name, &mut Vec::new(), 0, false)?;
    }
    Ok(expr)
}

/// Prints canonical expressions for editing. Name collisions retain the canonical spelling
/// so displaying then committing a formula never changes a defined-name reference.
pub fn print_input(expr: &Expr, locale: FormulaLocale, known_name: impl Fn(&str) -> bool) -> Result<String, ParseError> {
    // Check before cloning/printing: a flat operator chain can produce a deep AST even
    // though the parser itself did not recurse deeply.
    let mut pending = vec![(expr, 0usize)];
    while let Some((node, depth)) = pending.pop() {
        if depth > parser::MAX_DEPTH {
            return Err(ParseError { msg: "formula is nested too deeply".into(), pos: 0 });
        }
        match node {
            Expr::Unary(_, child) | Expr::Paren(child) => pending.push((child, depth + 1)),
            Expr::Binary(_, left, right) => {
                pending.push((left, depth + 1));
                pending.push((right, depth + 1));
            }
            Expr::Call(_, args) => pending.extend(args.iter().map(|e| (e, depth + 1))),
            Expr::Invoke(callee, args) => {
                pending.push((callee, depth + 1));
                pending.extend(args.iter().map(|e| (e, depth + 1)));
            }
            Expr::Array(rows) => pending.extend(rows.iter().flatten().map(|e| (e, depth + 1))),
            _ => {}
        }
    }
    if locale.is_canonical() {
        return Ok(crate::print(expr));
    }
    let mut expr = expr.clone();
    if locale.translates_names() {
        normalize(&mut expr, locale, &known_name, &mut Vec::new(), 0, true)?;
    }
    let canonical = crate::print(&expr);
    let tokens = crate::lexer::tokenize(&canonical).map_err(|e| ParseError { msg: e.msg, pos: e.pos })?;
    let mut out = String::new();
    let mut array_depth = 0usize;
    for token in tokens {
        use crate::lexer::Tok;
        let text = canonical.get(token.start..token.end).unwrap_or("");
        match token.tok {
            Tok::LBrace => {
                array_depth += 1;
                out.push_str(text);
            }
            Tok::RBrace => {
                array_depth = array_depth.saturating_sub(1);
                out.push_str(text);
            }
            Tok::Number(_) => out.push_str(&text.replace('.', &locale.decimal_separator().to_string())),
            Tok::Comma => out.push(if array_depth > 0 { locale.array_column_separator() } else { locale.list_separator() }),
            Tok::Semicolon if array_depth > 0 => out.push(locale.array_row_separator()),
            _ => out.push_str(text),
        }
    }
    Ok(out)
}

fn normalize(
    expr: &mut Expr,
    locale: FormulaLocale,
    known_name: &impl Fn(&str) -> bool,
    bindings: &mut Vec<String>,
    depth: usize,
    display: bool,
) -> Result<(), ParseError> {
    if depth > parser::MAX_DEPTH {
        return Err(ParseError { msg: "formula is nested too deeply".into(), pos: 0 });
    }
    let bound = |name: &str, bindings: &[String]| bindings.iter().rev().any(|n| n.eq_ignore_ascii_case(name)) || known_name(name);
    match expr {
        Expr::Bool(value) if display => {
            if let Some(alias) = locale.boolean_name(*value)
                && !bound(alias, bindings)
            {
                *expr = Expr::Name(alias.into());
            }
        }
        Expr::Name(name) if !display && !bound(name, bindings) => {
            if let Some(value) = locale.boolean_alias(name) {
                *expr = Expr::Bool(value);
            }
        }
        Expr::Call(name, args) if name == "LET" && args.len() >= 3 && !args.len().is_multiple_of(2) => {
            let saved = bindings.len();
            if let Some((body, pairs)) = args.split_last_mut() {
                for pair in pairs.chunks_mut(2) {
                    if let [declaration, value] = pair {
                        // The new name is not in scope in its own value expression.
                        normalize(value, locale, known_name, bindings, depth + 1, display)?;
                        if let Expr::Name(name) = declaration {
                            bindings.push(name.clone());
                        }
                    }
                }
                normalize(body, locale, known_name, bindings, depth + 1, display)?;
            }
            bindings.truncate(saved);
        }
        Expr::Call(name, args) if name == "LAMBDA" => {
            let saved = bindings.len();
            if let Some((body, params)) = args.split_last_mut() {
                for param in params {
                    if let Expr::Name(name) = param {
                        bindings.push(name.clone());
                    }
                }
                normalize(body, locale, known_name, bindings, depth + 1, display)?;
            }
            bindings.truncate(saved);
        }
        Expr::Call(name, args) => {
            let alias = if display { locale.function_name(name) } else { locale.canonical_name(name) };
            if !bound(name, bindings) && (!display || !bound(alias, bindings)) {
                *name = alias.to_string();
            }
            for arg in args {
                normalize(arg, locale, known_name, bindings, depth + 1, display)?;
            }
        }
        Expr::Invoke(callee, args) => {
            normalize(callee, locale, known_name, bindings, depth + 1, display)?;
            for arg in args {
                normalize(arg, locale, known_name, bindings, depth + 1, display)?;
            }
        }
        Expr::Unary(_, child) | Expr::Paren(child) => normalize(child, locale, known_name, bindings, depth + 1, display)?,
        Expr::Binary(_, left, right) => {
            normalize(left, locale, known_name, bindings, depth + 1, display)?;
            normalize(right, locale, known_name, bindings, depth + 1, display)?;
        }
        Expr::Array(rows) if display => {
            for row in rows {
                for child in row {
                    normalize(child, locale, known_name, bindings, depth + 1, display)?;
                }
            }
        }
        // Array elements have already been parsed as literals. Sheet/structured references
        // contain user-defined labels and must not be translated.
        _ => {}
    }
    Ok(())
}
