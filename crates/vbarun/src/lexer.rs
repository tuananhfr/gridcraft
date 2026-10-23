use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    Word(String),
    Number(f64),
    TypedNumber(f64, String),
    Text(String),
    Date(String),
    Symbol(String),
    Eol,
    Eof,
}
#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub kind: Kind,
    pub line: usize,
}
impl Token {
    pub fn is(&self, s: &str) -> bool {
        match &self.kind {
            Kind::Word(w) | Kind::Symbol(w) => w.eq_ignore_ascii_case(s),
            _ => false,
        }
    }
}
pub(crate) fn lex(source: &str) -> Result<Vec<Token>> {
    if source.len() > 4 * 1024 * 1024 {
        return Err(Error::limit("source bytes"));
    }
    let mut chars = source.chars().peekable();
    let mut tokens: Vec<Token> = Vec::new();
    let mut line = 1usize;
    let result = (|| {
        while let Some(c) = chars.next() {
            let start = line;
            let kind = match c {
                '\r' | ' ' | '\t' => continue,
                '\n' => {
                    line += 1;
                    Kind::Eol
                }
                '\'' => {
                    while chars.peek().is_some_and(|c| *c != '\n') {
                        chars.next();
                    }
                    continue;
                }
                '_' if chars.peek().is_some_and(|c| c.is_whitespace()) => {
                    while chars.peek().is_some_and(|c| matches!(c, ' ' | '\t' | '\r')) {
                        chars.next();
                    }
                    if chars.next() != Some('\n') {
                        return Err(Error::runtime(1002, "Expected line continuation newline"));
                    }
                    line += 1;
                    continue;
                }
                '"' => {
                    let mut s = String::new();
                    loop {
                        match chars.next() {
                            Some('"') if chars.peek() == Some(&'"') => {
                                chars.next();
                                s.push('"');
                            }
                            Some('"') => break,
                            Some('\n') | None => return Err(Error::runtime(1002, "Unterminated string")),
                            Some(c) => s.push(c),
                        }
                    }
                    Kind::Text(s)
                }
                '#' if tokens.iter().rev().take_while(|t| !matches!(t.kind, Kind::Eol)).any(|t| t.is("open")) => Kind::Symbol("#".into()),
                '#' => {
                    let mut s = String::new();
                    loop {
                        match chars.next() {
                            Some('#') => break,
                            Some('\n') | None => return Err(Error::runtime(1002, "Unterminated date")),
                            Some(c) => s.push(c),
                        }
                    }
                    Kind::Date(s)
                }
                '&' if chars.peek().is_some_and(|c| matches!(c, 'h' | 'H' | 'o' | 'O')) => {
                    let radix = if chars.next().is_some_and(|c| c.eq_ignore_ascii_case(&'h')) { 16 } else { 8 };
                    let mut s = String::new();
                    while chars.peek().is_some_and(|c| c.is_digit(radix)) {
                        if let Some(c) = chars.next() {
                            s.push(c);
                        }
                    }
                    Kind::Number(i64::from_str_radix(&s, radix).map_err(|_| Error::runtime(6, "Numeric literal overflow"))? as f64)
                }
                c if c.is_ascii_digit() || (c == '.' && chars.peek().is_some_and(|c| c.is_ascii_digit())) => {
                    let mut s = String::from(c);
                    while chars.peek().is_some_and(|c| c.is_ascii_digit() || *c == '.') {
                        if let Some(c) = chars.next() {
                            s.push(c);
                        }
                    }
                    if chars.peek().is_some_and(|c| matches!(c, 'e' | 'E' | 'd' | 'D')) {
                        chars.next();
                        s.push('e');
                        if chars.peek().is_some_and(|c| matches!(c, '+' | '-'))
                            && let Some(c) = chars.next()
                        {
                            s.push(c);
                        }
                        while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                            if let Some(c) = chars.next() {
                                s.push(c);
                            }
                        }
                    }
                    let n: f64 = s.parse().map_err(|_| Error::runtime(1002, "Invalid number"))?;
                    if !n.is_finite() {
                        return Err(Error::runtime(6, "Numeric literal overflow"));
                    }
                    if chars.peek().is_some_and(|c| matches!(c, '%' | '&' | '!' | '#' | '@')) {
                        let suffix = chars.next().ok_or_else(|| Error::runtime(1002, "Missing type suffix"))?;
                        Kind::TypedNumber(n, crate::parser::suffix_type(&suffix.to_string()))
                    } else {
                        Kind::Number(n)
                    }
                }
                c if c.is_alphabetic() || c == '_' => {
                    let mut s = String::from(c);
                    while chars.peek().is_some_and(|c| c.is_alphanumeric() || *c == '_') {
                        if let Some(c) = chars.next() {
                            s.push(c);
                        }
                    }
                    if s.eq_ignore_ascii_case("rem") {
                        while chars.peek().is_some_and(|c| *c != '\n') {
                            chars.next();
                        }
                        continue;
                    }
                    if chars.peek().is_some_and(|c| matches!(c, '$' | '%' | '!' | '@' | '#' | '&'))
                        && !(chars.peek() == Some(&'!') && chars.clone().nth(1).is_some_and(|c| c.is_alphabetic() || c == '_'))
                        && let Some(c) = chars.next()
                    {
                        s.push(c);
                    }
                    Kind::Word(s.to_ascii_lowercase())
                }
                c => {
                    let mut s = String::from(c);
                    if matches!((c, chars.peek()), (':', Some('=')) | ('<', Some('=' | '>')) | ('>', Some('=')))
                        && let Some(c) = chars.next()
                    {
                        s.push(c);
                    }
                    Kind::Symbol(s)
                }
            };
            tokens.push(Token { kind, line: start });
            if tokens.len() > 500_000 {
                return Err(Error::limit("source tokens"));
            }
        }
        tokens.push(Token { kind: Kind::Eol, line });
        tokens.push(Token { kind: Kind::Eof, line });
        Ok(tokens)
    })();
    result.map_err(|mut error: Error| {
        error.line = line;
        error
    })
}
