//! Deliberately small, read-only DAO query grammar; unsupported SQL fails closed.
use gridcraft_vbarun::{Error, Result, Value};

fn error() -> Error {
    Error::runtime(3075, "DAO: unsupported or invalid SELECT query")
}
#[derive(Debug, PartialEq)]
enum Token {
    Name(String),
    Literal(Value),
    Symbol(String),
}
fn tokens(sql: &str) -> Result<Vec<Token>> {
    if sql.len() > 65536 {
        return Err(error());
    }
    let mut input = sql.chars().peekable();
    let mut out = Vec::new();
    while let Some(c) = input.next() {
        if c.is_whitespace() {
            continue;
        }
        let token = match c {
            '[' | '\'' | '"' => {
                let end = if c == '[' { ']' } else { c };
                let mut text = String::new();
                loop {
                    match input.next() {
                        Some(ch) if ch == end => {
                            if input.peek() == Some(&end) {
                                input.next();
                                text.push(end);
                            } else {
                                break;
                            }
                        }
                        Some(ch) => text.push(ch),
                        None => return Err(error()),
                    }
                }
                if c == '[' { Token::Name(text) } else { Token::Literal(Value::String(text)) }
            }
            '*' | ',' | ';' => Token::Symbol(c.to_string()),
            '=' | '<' | '>' => {
                let mut op = c.to_string();
                if input.peek() == Some(&'=') || (c == '<' && input.peek() == Some(&'>')) {
                    op.push(input.next().ok_or_else(error)?);
                }
                Token::Symbol(op)
            }
            c if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' => {
                let mut n = c.to_string();
                while input.peek().is_some_and(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')) {
                    n.push(input.next().ok_or_else(error)?);
                }
                let n: f64 = n.parse().map_err(|_| error())?;
                if !n.is_finite() {
                    return Err(error());
                }
                Token::Literal(Value::Double(n))
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut s = c.to_string();
                while input.peek().is_some_and(|c| c.is_alphanumeric() || *c == '_') {
                    s.push(input.next().ok_or_else(error)?);
                }
                Token::Name(s)
            }
            _ => return Err(error()),
        };
        out.push(token);
    }
    Ok(out)
}
struct Parser {
    tokens: Vec<Token>,
    at: usize,
}
impl Parser {
    fn eat(&mut self, text: &str) -> bool {
        let yes = self.tokens.get(self.at).is_some_and(|t| match t {
            Token::Name(s) | Token::Symbol(s) => s.eq_ignore_ascii_case(text),
            _ => false,
        });
        if yes {
            self.at += 1;
        }
        yes
    }
    fn name(&mut self) -> Result<String> {
        let Some(Token::Name(s)) = self.tokens.get(self.at) else { return Err(error()) };
        self.at += 1;
        Ok(s.clone())
    }
    fn column(&mut self, columns: &[String]) -> Result<usize> {
        let name = self.name()?;
        columns.iter().position(|c| c.eq_ignore_ascii_case(&name)).ok_or_else(error)
    }
}
struct Predicate {
    col: usize,
    op: String,
    value: Value,
}
pub(super) struct Query {
    pub table: String,
    columns: Option<Vec<String>>,
    parser: Parser,
}
impl Query {
    pub fn parse(sql: &str) -> Result<Self> {
        if !sql.trim_start().get(..6).is_some_and(|s| s.eq_ignore_ascii_case("SELECT")) {
            return Ok(Self {
                table: sql.trim().trim_start_matches('[').trim_end_matches(']').into(),
                columns: None,
                parser: Parser { tokens: vec![], at: 0 },
            });
        }
        let mut p = Parser { tokens: tokens(sql)?, at: 0 };
        if !p.eat("SELECT") {
            return Ok(Self {
                table: sql.trim().trim_start_matches('[').trim_end_matches(']').into(),
                columns: None,
                parser: Parser { tokens: vec![], at: 0 },
            });
        }
        let columns = if p.eat("*") {
            None
        } else {
            let mut cols = vec![p.name()?];
            while p.eat(",") {
                cols.push(p.name()?);
            }
            Some(cols)
        };
        if !p.eat("FROM") {
            return Err(error());
        }
        let table = p.name()?;
        Ok(Self { table, columns, parser: p })
    }
    pub fn apply(mut self, names: &[String], rows: Vec<Vec<Value>>) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
        let projection = match self.columns {
            Some(cols) => cols.iter().map(|n| names.iter().position(|c| c.eq_ignore_ascii_case(n)).ok_or_else(error)).collect::<Result<Vec<_>>>()?,
            None => (0..names.len()).collect(),
        };
        if projection.len() > 256 {
            return Err(error());
        }
        let mut groups: Vec<Vec<Predicate>> = Vec::new();
        if self.parser.eat("WHERE") {
            groups.push(Vec::new());
            loop {
                let col = self.parser.column(names)?;
                let Some(Token::Symbol(op)) = self.parser.tokens.get(self.parser.at) else { return Err(error()) };
                if !["=", "<>", "<", ">", "<=", ">="].contains(&op.as_str()) {
                    return Err(error());
                }
                let op = op.clone();
                self.parser.at += 1;
                let Some(Token::Literal(value)) = self.parser.tokens.get(self.parser.at) else { return Err(error()) };
                groups.last_mut().ok_or_else(error)?.push(Predicate { col, op, value: value.clone() });
                self.parser.at += 1;
                if self.parser.eat("AND") {
                    continue;
                }
                if self.parser.eat("OR") {
                    groups.push(Vec::new());
                    continue;
                }
                break;
            }
        }
        let mut order = Vec::new();
        if self.parser.eat("ORDER") {
            if !self.parser.eat("BY") {
                return Err(error());
            }
            loop {
                let col = self.parser.column(names)?;
                let desc = self.parser.eat("DESC");
                if !desc {
                    self.parser.eat("ASC");
                }
                order.push((col, desc));
                if !self.parser.eat(",") {
                    break;
                }
            }
        }
        self.parser.eat(";");
        if self.parser.at != self.parser.tokens.len() {
            return Err(error());
        }
        let mut rows: Vec<_> = rows
            .into_iter()
            .filter(|row| {
                groups.is_empty()
                    || groups.iter().any(|group| {
                        group.iter().all(|p| {
                            let Some(v) = row.get(p.col).filter(|v| !matches!(v, Value::Null)) else { return false };
                            let cmp = compare(v, &p.value);
                            match p.op.as_str() {
                                "=" => cmp.is_eq(),
                                "<>" => !cmp.is_eq(),
                                "<" => cmp.is_lt(),
                                ">" => cmp.is_gt(),
                                "<=" => !cmp.is_gt(),
                                ">=" => !cmp.is_lt(),
                                _ => false,
                            }
                        })
                    })
            })
            .collect();
        rows.sort_by(|a, b| {
            for &(col, desc) in &order {
                let cmp = compare(a.get(col).unwrap_or(&Value::Null), b.get(col).unwrap_or(&Value::Null));
                if !cmp.is_eq() {
                    return if desc { cmp.reverse() } else { cmp };
                }
            }
            std::cmp::Ordering::Equal
        });
        let projected_bytes =
            rows.iter().fold(0usize, |total, row| projection.iter().fold(total, |n, col| n.saturating_add(row.get(*col).map_or(0, Value::memory))));
        if projected_bytes > 32 * 1024 * 1024 {
            return Err(Error { fatal: true, ..Error::runtime(3075, "VBA sandbox limit: DAO query projection") });
        }
        let names = projection.iter().filter_map(|i| names.get(*i).cloned()).collect();
        let rows = rows.into_iter().map(|r| projection.iter().map(|i| r.get(*i).cloned().ok_or_else(error)).collect()).collect::<Result<_>>()?;
        Ok((names, rows))
    }
}
fn compare(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Null, _) => std::cmp::Ordering::Less,
        (_, Value::Null) => std::cmp::Ordering::Greater,
        (Value::String(a), Value::String(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
        _ => match (a.number(), b.number()) {
            (Ok(a), Ok(b)) => {
                if a == b {
                    std::cmp::Ordering::Equal
                } else {
                    a.total_cmp(&b)
                }
            }
            _ => a.text().unwrap_or_default().cmp(&b.text().unwrap_or_default()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macro_dao_query_predicates_projection_and_typed_order() {
        assert!(compare(&Value::Double(-0.0), &Value::Double(0.0)).is_eq());
        let names = vec!["Joint".into(), "Fz".into()];
        let row = |s: &str, n: f64| vec![Value::String(s.into()), Value::Double(n)];
        let query = Query::parse(
            "SELECT Fz, Joint FROM [Support Reactions] WHERE Fz <= -2 AND Joint <> 'excluded' OR Joint = 'other' ORDER BY Fz DESC, Joint DESC",
        )
        .unwrap();
        let (cols, rows) = query
            .apply(&names, vec![row("excluded", -2.0), row("a", -10.0), row("b", -2.0), row("a", -2.0), row("other", 8.0), row("positive", 2.0)])
            .unwrap();
        assert_eq!(cols, ["Fz", "Joint"]);
        assert_eq!(
            rows,
            vec![
                vec![Value::Double(8.0), Value::String("other".into())],
                vec![Value::Double(-2.0), Value::String("b".into())],
                vec![Value::Double(-2.0), Value::String("a".into())],
                vec![Value::Double(-10.0), Value::String("a".into())]
            ]
        );
    }
    #[test]
    fn macro_dao_query_rejects_uncovered_sql_and_keeps_raw_table_names() {
        let names = vec!["X".into()];
        for sql in [
            "SELECT * FROM T WHERE X LIKE 'x'",
            "SELECT * FROM T ORDER BY Missing",
            "SELECT * FROM T; DELETE FROM T",
            "SELECT * FROM T WHERE X = 1 OR",
            "SELECT * FROM T WHERE (X=1)",
        ] {
            assert!(Query::parse(sql).and_then(|q| q.apply(&names, vec![])).is_err(), "{sql}");
        }
        assert_eq!(Query::parse("Object Geometry - Design Strips").unwrap().table, "Object Geometry - Design Strips");
        let sql = format!("SELECT {} FROM T", vec!["X"; 256].join(","));
        let error = Query::parse(&sql).unwrap().apply(&names, vec![vec![Value::String("x".repeat(200_000))]]).unwrap_err();
        assert!(error.fatal);
    }
}
