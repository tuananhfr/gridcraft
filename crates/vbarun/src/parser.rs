use crate::{
    Error, Result, Value,
    lexer::{self, Kind, Token},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Program {
    pub modules: Vec<Module>,
}
#[derive(Clone, Debug)]
pub struct Module {
    pub name: String,
    pub option_explicit: bool,
    pub option_base: i32,
    pub compare_text: bool,
    pub(crate) declarations: Vec<Decl>,
    pub(crate) procedures: BTreeMap<String, Procedure>,
}
#[derive(Clone, Debug)]
pub(crate) struct Procedure {
    pub public: bool,
    pub name: String,
    pub params: Vec<Param>,
    pub ty: String,
    pub body: Vec<Stmt>,
}
#[derive(Clone, Debug)]
pub(crate) struct Param {
    pub decl: Decl,
    pub by_val: bool,
    pub optional: bool,
    pub param_array: bool,
    pub default: Option<Expr>,
}
#[derive(Clone, Debug)]
pub(crate) struct Decl {
    pub public: bool,
    pub line: usize,
    pub name: String,
    pub ty: String,
    pub bounds: Option<Vec<(Option<Expr>, Expr)>>,
    pub initial: Option<Expr>,
    pub constant: bool,
    pub persistent: bool,
}
#[derive(Clone, Debug)]
pub(crate) enum Expr {
    Literal(Value),
    Name(String),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Member(Box<Expr>, String),
    Invoke(Box<Expr>, Vec<Arg>),
    With,
}
#[derive(Clone, Debug)]
pub(crate) struct Arg {
    pub name: Option<String>,
    pub expr: Expr,
}
#[derive(Clone, Debug)]
pub(crate) struct Stmt {
    pub line: usize,
    pub kind: Statement,
}
#[derive(Clone, Debug)]
pub(crate) enum Statement {
    Decl(Vec<Decl>),
    Assign(Expr, Expr),
    SetAssign(Expr, Expr),
    Call(Expr),
    If(Vec<(Expr, Vec<Stmt>)>, Vec<Stmt>),
    Select(Expr, Vec<(Vec<Case>, Vec<Stmt>)>, Vec<Stmt>),
    For(String, Expr, Expr, Expr, Vec<Stmt>),
    Each(String, Expr, Vec<Stmt>),
    Do(Option<(bool, Expr)>, Vec<Stmt>, Option<(bool, Expr)>),
    With(Expr, Vec<Stmt>),
    Exit(String),
    Label(String),
    Goto(String),
    OnError(String),
    Resume(String),
    Redim(bool, Vec<Decl>),
    Erase(Vec<Expr>),
    Mid(Expr, Expr, Option<Expr>, Expr),
    Print(Vec<Expr>),
    Stop,
    Unsupported(String),
    Randomize(Option<Expr>),
}
#[derive(Clone, Debug)]
pub(crate) enum Case {
    Value(Expr),
    Range(Expr, Expr),
    Is(String, Expr),
}
impl Program {
    pub fn parse(name: &str, source: &str) -> Result<Self> {
        Ok(Self { modules: vec![Module::parse(name, source)?] })
    }
    pub fn from_project(project: &gridcraft_vba::Project) -> Result<Self> {
        Ok(Self { modules: project.modules.iter().map(|m| Module::parse(&m.name, &m.source)).collect::<Result<_>>()? })
    }
}
impl Module {
    pub fn parse(name: &str, source: &str) -> Result<Self> {
        let tokens = lexer::lex(source).map_err(|mut e| {
            e.module = name.into();
            e
        })?;
        let mut p = Parser { tokens, pos: 0, depth: 0, procedure: String::new() };
        p.module(name).map_err(|mut e| {
            e.module = name.into();
            e.line = p.line();
            e.procedure = p.procedure.clone();
            e
        })
    }
}
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
    procedure: String,
}
impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
    fn at(&self, s: &str) -> bool {
        self.peek().is_some_and(|t| t.is(s))
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.at(s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn need(&mut self, s: &str) -> Result<()> {
        if self.eat(s) { Ok(()) } else { Err(self.error(&format!("Expected {s}"))) }
    }
    fn line(&self) -> usize {
        self.peek().map_or(1, |t| t.line)
    }
    fn error(&self, msg: &str) -> Error {
        let mut e = Error::runtime(1002, msg);
        e.line = self.line();
        e
    }
    fn word(&mut self) -> Result<String> {
        match self.peek().map(|t| &t.kind) {
            Some(Kind::Word(s)) => {
                let s = s.clone();
                self.pos += 1;
                Ok(s)
            }
            _ => Err(self.error("Expected identifier")),
        }
    }
    fn end(&self) -> bool {
        matches!(self.peek().map(|t| &t.kind), None | Some(Kind::Eof))
    }
    fn eol(&self) -> bool {
        matches!(self.peek().map(|t| &t.kind), None | Some(Kind::Eol | Kind::Eof))
    }
    fn sep(&self) -> bool {
        self.eol() || self.at(":")
    }
    fn skip(&mut self) {
        while !self.end() && self.sep() {
            self.pos += 1;
        }
    }
    fn finish(&mut self) -> Result<()> {
        if !self.sep() {
            return Err(self.error("Expected end of statement"));
        }
        self.skip();
        Ok(())
    }
    fn enter(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(Error::limit("syntax nesting"));
        }
        Ok(())
    }
    fn module(&mut self, name: &str) -> Result<Module> {
        let mut m = Module {
            name: name.into(),
            option_explicit: false,
            option_base: 0,
            compare_text: false,
            declarations: Vec::new(),
            procedures: BTreeMap::new(),
        };
        self.skip();
        while !self.end() {
            if self.eat("attribute") {
                while !self.eol() {
                    self.pos += 1;
                }
                self.skip();
                continue;
            }
            if self.eat("option") {
                if self.eat("explicit") {
                    m.option_explicit = true;
                } else if self.eat("base") {
                    m.option_base = if self.eat_number(1.0) {
                        1
                    } else {
                        if !self.eat_number(0.0) {
                            return Err(self.error("Option Base must be 0 or 1"));
                        }
                        0
                    };
                } else if self.eat("compare") {
                    m.compare_text = self.eat("text");
                    if !m.compare_text {
                        self.need("binary")?;
                    }
                } else {
                    return Err(self.error("Unsupported Option"));
                }
                self.finish()?;
                continue;
            }
            let public = self.eat("public");
            let private = self.eat("private");
            self.eat("friend");
            if self.at("declare") {
                return Err(Error::unsupported("Declare"));
            }
            let persistent = self.eat("static");
            if self.at("sub") || self.at("function") || self.at("property") {
                let kind = self.word()?;
                let property = if kind == "property" { self.word()? } else { String::new() };
                let proc_name = self.word()?;
                let mut params = Vec::new();
                if self.eat("(") {
                    if !self.at(")") {
                        loop {
                            let optional = self.eat("optional");
                            let by_val = self.eat("byval");
                            self.eat("byref");
                            let param_array = self.eat("paramarray");
                            let mut decl = self.decl(false, false)?;
                            let default = decl.initial.take();
                            params.push(Param { decl, by_val, optional, param_array, default });
                            if !self.eat(",") {
                                break;
                            }
                        }
                    }
                    self.need(")")?;
                }
                let ty = if self.eat("as") { self.type_name()? } else { suffix_type(&proc_name) };
                self.finish()?;
                self.procedure = proc_name.clone();
                let mut body = self.block(&["end"])?;
                if persistent {
                    persist_declarations(&mut body);
                }
                self.need("end")?;
                self.need(&kind)?;
                self.finish()?;
                self.procedure.clear();
                let key = if property.is_empty() || property == "get" { proc_name.clone() } else { format!("{proc_name}.{property}") };
                if m.procedures.insert(key, Procedure { public: !private, name: proc_name, params, ty, body }).is_some() {
                    return Err(self.error("Duplicate procedure"));
                }
            } else {
                self.eat("dim");
                let constant = self.eat("const");
                let mut declarations = self.decls(constant, persistent)?;
                for declaration in &mut declarations {
                    declaration.public = public;
                }
                m.declarations.extend(declarations);
                self.finish()?;
            }
        }
        Ok(m)
    }
    fn eat_number(&mut self, n: f64) -> bool {
        if matches!(self.peek().map(|t| &t.kind), Some(Kind::Number(v)) if *v == n) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn type_name(&mut self) -> Result<String> {
        if self.eat("new") {
            return Err(Error::unsupported("As New"));
        }
        let mut ty = self.word()?;
        while self.eat(".") {
            ty.push('.');
            ty.push_str(&self.word()?);
        }
        Ok(ty)
    }
    fn decl(&mut self, constant: bool, persistent: bool) -> Result<Decl> {
        let line = self.line();
        let name = self.word()?;
        let bounds = if self.eat("(") {
            let mut b = Vec::new();
            if !self.at(")") {
                loop {
                    let first = self.expr(0)?;
                    let bound = if self.eat("to") { (Some(first), self.expr(0)?) } else { (None, first) };
                    b.push(bound);
                    if !self.eat(",") {
                        break;
                    }
                }
            }
            self.need(")")?;
            Some(b)
        } else {
            None
        };
        let ty = if self.eat("as") { self.type_name()? } else { suffix_type(&name) };
        let initial = if self.eat("=") { Some(self.expr(0)?) } else { None };
        Ok(Decl { public: false, line, name, ty, bounds, initial, constant, persistent })
    }
    fn decls(&mut self, constant: bool, persistent: bool) -> Result<Vec<Decl>> {
        let mut v = vec![self.decl(constant, persistent)?];
        while self.eat(",") {
            v.push(self.decl(constant, persistent)?);
        }
        Ok(v)
    }
    fn block(&mut self, ends: &[&str]) -> Result<Vec<Stmt>> {
        self.enter()?;
        let mut stmts = Vec::new();
        self.skip();
        while !self.end() && !ends.iter().any(|s| self.at(s)) {
            stmts.push(self.statement()?);
            self.skip();
        }
        self.depth = self.depth.saturating_sub(1);
        Ok(stmts)
    }
    fn label(&mut self) -> Result<String> {
        if let Some(Kind::Number(n)) = self.peek().map(|t| &t.kind) {
            let s = n.to_string();
            self.pos += 1;
            Ok(s)
        } else {
            self.word()
        }
    }
    fn statement(&mut self) -> Result<Stmt> {
        self.enter()?;
        let result = self.statement_inner();
        self.depth = self.depth.saturating_sub(1);
        result
    }
    fn statement_inner(&mut self) -> Result<Stmt> {
        let line = self.line();
        let kind = if (self.tokens.get(self.pos + 1).is_some_and(|t| t.is(":"))
            && !["do", "loop", "stop", "else", "end", "wend", "next"].iter().any(|word| self.at(word)))
            || matches!(self.peek().map(|t| &t.kind), Some(Kind::Number(_)))
        {
            let label = self.label()?;
            self.eat(":");
            return Ok(Stmt { line, kind: Statement::Label(label) });
        } else if self.eat("if") {
            let test = self.expr(0)?;
            self.need("then")?;
            let mut branches = Vec::new();
            let mut other = Vec::new();
            if !self.eol() {
                let mut first = Vec::new();
                while !self.eol() && !self.at("else") {
                    if self.eat(":") {
                        continue;
                    }
                    first.push(self.statement()?);
                }
                branches.push((test, first));
                if self.eat("else") {
                    while !self.eol() {
                        if self.eat(":") {
                            continue;
                        }
                        other.push(self.statement()?);
                    }
                }
            } else {
                branches.push((test, self.block(&["elseif", "else", "end"])?));
                while self.eat("elseif") {
                    let e = self.expr(0)?;
                    self.need("then")?;
                    self.finish()?;
                    branches.push((e, self.block(&["elseif", "else", "end"])?));
                }
                if self.eat("else") {
                    self.finish()?;
                    other = self.block(&["end"])?;
                }
                self.need("end")?;
                self.need("if")?;
            }
            Statement::If(branches, other)
        } else if self.eat("select") {
            self.need("case")?;
            let e = self.expr(0)?;
            self.finish()?;
            let mut cases = Vec::new();
            let mut other = Vec::new();
            while self.eat("case") {
                if self.eat("else") {
                    self.finish()?;
                    other = self.block(&["end"])?;
                    break;
                }
                let mut tests = Vec::new();
                loop {
                    tests.push(if self.eat("is") {
                        let op = self.symbol()?;
                        Case::Is(op, self.expr(0)?)
                    } else {
                        let a = self.expr(0)?;
                        if self.eat("to") { Case::Range(a, self.expr(0)?) } else { Case::Value(a) }
                    });
                    if tests.len() > 64 {
                        return Err(Error::limit("Case complexity"));
                    }
                    if !self.eat(",") {
                        break;
                    }
                }
                self.finish()?;
                cases.push((tests, self.block(&["case", "end"])?));
            }
            self.need("end")?;
            self.need("select")?;
            Statement::Select(e, cases, other)
        } else if self.eat("for") {
            let each = self.eat("each");
            let name = self.word()?;
            if each {
                self.need("in")?;
                let e = self.expr(0)?;
                self.finish()?;
                let body = self.block(&["next"])?;
                self.need("next")?;
                if !self.sep() {
                    self.word()?;
                }
                Statement::Each(name, e, body)
            } else {
                self.need("=")?;
                let start = self.expr(0)?;
                self.need("to")?;
                let end = self.expr(0)?;
                let step = if self.eat("step") { self.expr(0)? } else { Expr::Literal(Value::Integer(1)) };
                self.finish()?;
                let body = self.block(&["next"])?;
                self.need("next")?;
                if !self.sep() {
                    self.word()?;
                }
                Statement::For(name, start, end, step, body)
            }
        } else if self.eat("do") {
            let pre = self.condition()?;
            self.finish()?;
            let body = self.block(&["loop"])?;
            self.need("loop")?;
            Statement::Do(pre, body, self.condition()?)
        } else if self.eat("while") {
            let e = self.expr(0)?;
            self.finish()?;
            let body = self.block(&["wend"])?;
            self.need("wend")?;
            Statement::Do(Some((false, e)), body, None)
        } else if self.eat("with") {
            let e = self.expr(0)?;
            self.finish()?;
            let body = self.block(&["end"])?;
            self.need("end")?;
            self.need("with")?;
            Statement::With(e, body)
        } else if self.eat("exit") {
            Statement::Exit(self.word()?)
        } else if self.eat("goto") {
            Statement::Goto(self.label()?)
        } else if self.eat("on") {
            self.need("error")?;
            let mode = if self.eat("resume") {
                self.need("next")?;
                "next".into()
            } else {
                self.need("goto")?;
                self.label()?
            };
            Statement::OnError(mode)
        } else if self.eat("resume") {
            Statement::Resume(if self.sep() { String::new() } else { self.label()? })
        } else if self.eat("redim") {
            let preserve = self.eat("preserve");
            Statement::Redim(preserve, self.decls(false, false)?)
        } else if self.eat("erase") {
            Statement::Erase(self.expr_list()?)
        } else if self.eat("open") {
            while !self.sep() {
                self.pos += 1;
            }
            Statement::Unsupported("Open".into())
        } else if self.eat("stop") {
            Statement::Stop
        } else if self.eat("randomize") {
            Statement::Randomize(if self.sep() { None } else { Some(self.expr(0)?) })
        } else if self.eat("debug") {
            self.need(".")?;
            self.need("print")?;
            let mut es = Vec::new();
            while !self.sep() && !self.at("else") {
                es.push(self.expr(0)?);
                if !self.eat(",") && !self.eat(";") {
                    break;
                }
            }
            Statement::Print(es)
        } else if self.at("dim") || self.at("private") || self.at("public") || self.at("const") || self.at("static") {
            let kw = self.word()?;
            Statement::Decl(self.decls(kw == "const", kw == "static")?)
        } else if self.at("declare") {
            return Err(Error::unsupported("Declare"));
        } else if self.at("mid") || self.at("mid$") {
            self.word()?;
            self.need("(")?;
            let dest = self.expr(0)?;
            self.need(",")?;
            let start = self.expr(0)?;
            let len = if self.eat(",") { Some(self.expr(0)?) } else { None };
            self.need(")")?;
            self.need("=")?;
            Statement::Mid(dest, start, len, self.expr(0)?)
        } else {
            let explicit_call = self.eat("call");
            let set = self.eat("set");
            self.eat("let");
            let target = self.expr(7)?;
            if !explicit_call && self.eat("=") {
                if set { Statement::SetAssign(target, self.expr(0)?) } else { Statement::Assign(target, self.expr(0)?) }
            } else if !self.sep() && !self.at("else") {
                let args = self.args(false)?;
                Statement::Call(Expr::Invoke(Box::new(target), args))
            } else {
                Statement::Call(target)
            }
        };
        Ok(Stmt { line, kind })
    }
    fn expr_list(&mut self) -> Result<Vec<Expr>> {
        let mut v = vec![self.expr(0)?];
        while self.eat(",") {
            v.push(self.expr(0)?);
        }
        Ok(v)
    }
    fn condition(&mut self) -> Result<Option<(bool, Expr)>> {
        if self.eat("while") {
            Ok(Some((false, self.expr(0)?)))
        } else if self.eat("until") {
            Ok(Some((true, self.expr(0)?)))
        } else {
            Ok(None)
        }
    }
    fn symbol(&mut self) -> Result<String> {
        match self.peek().map(|t| &t.kind) {
            Some(Kind::Symbol(s)) => {
                let s = s.clone();
                self.pos += 1;
                Ok(s)
            }
            _ => Err(self.error("Expected operator")),
        }
    }
    fn args(&mut self, parens: bool) -> Result<Vec<Arg>> {
        let mut args = Vec::new();
        if parens && self.eat(")") {
            return Ok(args);
        }
        loop {
            let name = if self.tokens.get(self.pos + 1).is_some_and(|t| t.is(":=")) {
                let n = self.word()?;
                self.need(":=")?;
                Some(n)
            } else {
                None
            };
            let expr = if self.at(",") || self.at(")") || self.sep() { Expr::Literal(Value::Missing) } else { self.expr(0)? };
            args.push(Arg { name, expr });
            if !self.eat(",") {
                break;
            }
        }
        if parens {
            self.need(")")?;
        }
        Ok(args)
    }
    fn expr(&mut self, min: u8) -> Result<Expr> {
        self.enter()?;
        let mut left = if self.eat("not") {
            Expr::Unary("not".into(), Box::new(self.expr(5)?))
        } else if self.eat("-") {
            Expr::Unary("-".into(), Box::new(self.expr(13)?))
        } else if self.eat("+") {
            Expr::Unary("+".into(), Box::new(self.expr(13)?))
        } else if self.eat("(") {
            let e = self.expr(0)?;
            self.need(")")?;
            e
        } else if self.eat(".") {
            Expr::Member(Box::new(Expr::With), self.word()?)
        } else {
            let t = self.peek().cloned().ok_or_else(|| self.error("Expected expression"))?;
            self.pos += 1;
            match t.kind {
                Kind::Number(n) => Expr::Literal(if n.fract() == 0.0 && n >= i16::MIN as f64 && n <= i16::MAX as f64 {
                    Value::Integer(n as i16)
                } else if n.fract() == 0.0 && n >= i32::MIN as f64 && n <= i32::MAX as f64 {
                    Value::Long(n as i32)
                } else {
                    Value::Double(n)
                }),
                Kind::TypedNumber(n, ty) => Expr::Literal(Value::Double(n).coerce(&ty)?),
                Kind::Text(s) => Expr::Literal(Value::String(s)),
                Kind::Date(s) => Expr::Literal(Value::String(s).coerce("date")?),
                Kind::Word(s) => match s.as_str() {
                    "true" => Expr::Literal(Value::Boolean(true)),
                    "false" => Expr::Literal(Value::Boolean(false)),
                    "empty" => Expr::Literal(Value::Empty),
                    "null" => Expr::Literal(Value::Null),
                    "nothing" => Expr::Literal(Value::Object(None)),
                    _ => Expr::Name(s),
                },
                _ => return Err(self.error("Expected expression")),
            }
        };
        let mut chain = 0usize;
        loop {
            chain += 1;
            if chain > 64 {
                return Err(Error::limit("expression complexity"));
            }
            if self.eat(".") {
                left = Expr::Member(Box::new(left), self.word()?);
                continue;
            }
            if self.eat("(") {
                left = Expr::Invoke(Box::new(left), self.args(true)?);
                continue;
            }
            let Some((op, precedence)) = self.peek().and_then(|t| match &t.kind {
                Kind::Word(s) | Kind::Symbol(s) => precedence(s).map(|p| (s.clone(), p)),
                _ => None,
            }) else {
                break;
            };
            if precedence < min {
                break;
            }
            self.pos += 1;
            let right = self.expr(precedence + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        self.depth = self.depth.saturating_sub(1);
        Ok(left)
    }
}
fn precedence(s: &str) -> Option<u8> {
    Some(match s {
        "imp" => 0,
        "eqv" => 1,
        "xor" => 2,
        "or" => 3,
        "and" => 4,
        "=" | "<>" | "<" | ">" | "<=" | ">=" | "like" | "is" => 6,
        "&" => 7,
        "+" | "-" => 8,
        "mod" => 9,
        "\\" => 10,
        "*" | "/" => 11,
        "^" => 13,
        _ => return None,
    })
}
pub(crate) fn suffix_type(name: &str) -> String {
    match name.chars().last() {
        Some('$') => "string",
        Some('%') => "integer",
        Some('&') => "long",
        Some('!') => "single",
        Some('#') => "double",
        Some('@') => "currency",
        _ => "variant",
    }
    .into()
}

fn persist_declarations(body: &mut [Stmt]) {
    for stmt in body {
        match &mut stmt.kind {
            Statement::Decl(ds) => {
                for d in ds {
                    d.persistent = true;
                }
            }
            Statement::If(branches, other) => {
                for (_, body) in branches {
                    persist_declarations(body);
                }
                persist_declarations(other);
            }
            Statement::Select(_, cases, other) => {
                for (_, body) in cases {
                    persist_declarations(body);
                }
                persist_declarations(other);
            }
            Statement::For(_, _, _, _, body) | Statement::Each(_, _, body) | Statement::Do(_, body, _) | Statement::With(_, body) => {
                persist_declarations(body)
            }
            _ => {}
        }
    }
}
