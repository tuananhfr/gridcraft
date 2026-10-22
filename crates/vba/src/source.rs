use crate::{Compatibility, Error, MAX_LINES, MAX_PROCEDURES, ModuleKind, Param, ProcKind, Procedure, Project, Scope, classify};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) struct Statement {
    pub line: u32,
    pub tokens: Vec<String>,
    pub token_lines: Vec<u32>,
}

pub(crate) fn lower(s: &str) -> String {
    s.to_lowercase()
}

fn tokenize(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        let mut token = c.to_string();
        if c == ':' && chars.peek() == Some(&'=') {
            token.push(chars.next().unwrap_or('='));
        } else if c == '"' {
            while let Some(c) = chars.next() {
                token.push(c);
                if c == '"' {
                    if chars.peek() == Some(&'"') {
                        token.push(chars.next().unwrap_or('"'));
                    } else {
                        break;
                    }
                }
            }
        } else if c.is_alphanumeric() || c == '_' {
            while let Some(&c) = chars.peek() {
                if !c.is_alphanumeric() && c != '_' {
                    break;
                }
                token.push(chars.next().unwrap_or(c));
            }
        }
        tokens.push(token);
    }
    tokens
}

/// Code part of a physical line and whether a comment was stripped from it.
fn clean_line(s: &str) -> (String, bool) {
    let mut output = String::new();
    let mut quoted = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if quoted && chars.peek() == Some(&'"') {
                output.push(c);
                output.push(chars.next().unwrap_or('"'));
                continue;
            }
            quoted = !quoted;
        }
        if !quoted && (c == 'r' || c == 'R') && output.chars().last().is_none_or(|c| c.is_whitespace() || c == ':') {
            let mut ahead = chars.clone();
            if ahead.next().is_some_and(|c| c.eq_ignore_ascii_case(&'e'))
                && ahead.next().is_some_and(|c| c.eq_ignore_ascii_case(&'m'))
                && ahead.next().is_none_or(char::is_whitespace)
            {
                return (output, true);
            }
        }
        if c == '\'' && !quoted {
            return (output, true);
        }
        output.push(c);
    }
    (output, false)
}

fn statements(source: &str, lines: &mut usize, tokens_left: &mut usize) -> Result<Vec<Statement>, Error> {
    let mut result = Vec::new();
    let mut joined = String::new();
    let mut first = 1;
    let mut spans = Vec::new();
    let mut comment_continues = false;
    for (i, line) in source.lines().enumerate() {
        if line.len() > 256 * 1024 || joined.len().saturating_add(line.len()) > 256 * 1024 {
            return Err(Error::Limit("logical line bytes"));
        }
        *lines = lines.checked_sub(1).ok_or(Error::Limit("source line count"))?;
        let number = u32::try_from(i + 1).map_err(|_| Error::Limit("source line count"))?;
        let raw_continued = line.trim_end().ends_with(" _");
        // MS-VBAL comment bodies include line continuations, so ` _` extends a comment onto the next line.
        if comment_continues {
            comment_continues = raw_continued;
            continue;
        }
        let (clean, commented) = clean_line(line);
        let clean = clean.trim_end();
        comment_continues = commented && raw_continued;
        let continued = !commented && clean.ends_with(" _");
        if joined.is_empty() {
            first = number;
        }
        spans.push((joined.len(), number));
        joined.push_str(if continued { clean.get(..clean.len() - 1).unwrap_or(clean) } else { clean });
        if !continued {
            flush(&mut joined, &mut spans, first, &mut result, tokens_left)?;
        }
    }
    // A dangling continuation at the end of a module still forms a statement.
    flush(&mut joined, &mut spans, first, &mut result, tokens_left)?;
    Ok(result)
}

fn flush(joined: &mut String, spans: &mut Vec<(usize, u32)>, first: u32, result: &mut Vec<Statement>, tokens_left: &mut usize) -> Result<(), Error> {
    let tokens = tokenize(joined);
    *tokens_left = tokens_left.checked_sub(tokens.len()).ok_or(Error::Limit("source tokens"))?;
    let mut cursor = 0;
    let token_lines: Vec<_> = tokens
        .iter()
        .map(|token| {
            let start = joined.get(cursor..).and_then(|s| s.find(token.as_str())).map(|n| cursor + n).unwrap_or(cursor);
            cursor = start + token.len();
            spans.get(spans.partition_point(|&(offset, _)| offset <= start).saturating_sub(1)).map(|&(_, line)| line).unwrap_or(first)
        })
        .collect();
    let mut start = 0;
    for part in tokens.split(|t| t == ":") {
        let end = start + part.len();
        let part_lines = token_lines.get(start..end).unwrap_or(&[]).to_vec();
        start = end + 1;
        if part.first().is_some_and(|t| t.eq_ignore_ascii_case("rem")) {
            break;
        }
        if !part.is_empty() && !part.first().is_some_and(|t| t.eq_ignore_ascii_case("attribute")) {
            result.push(Statement { line: first, tokens: part.to_vec(), token_lines: part_lines });
        }
    }
    joined.clear();
    spans.clear();
    Ok(())
}

fn at(tokens: &[String], i: usize, expected: &str) -> bool {
    tokens.get(i).is_some_and(|t| t.eq_ignore_ascii_case(expected))
}

fn identifier(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
}

fn typed(tokens: &[String]) -> Option<String> {
    tokens.iter().position(|s| s.eq_ignore_ascii_case("as")).map(|i| tokens.iter().skip(i + 1).cloned().collect::<Vec<_>>().join(""))
}

fn parameter(tokens: &[String]) -> Option<Param> {
    let equal = tokens.iter().position(|s| s == "=").unwrap_or(tokens.len());
    let head = tokens.get(..equal)?;
    let name = head.iter().find(|s| !["optional", "byval", "byref", "paramarray"].contains(&lower(s).as_str()))?;
    Some(Param {
        name: name.clone(),
        type_name: typed(head),
        by_val: head.iter().any(|s| s.eq_ignore_ascii_case("byval")),
        optional: head.iter().any(|s| s.eq_ignore_ascii_case("optional")),
        param_array: head.iter().any(|s| s.eq_ignore_ascii_case("paramarray")),
        default: tokens.get(equal + 1..).map(|t| t.join("")),
    })
}

fn header(statement: &Statement) -> Option<Procedure> {
    let t = &statement.tokens;
    let mut i = 0;
    let scope = if at(t, i, "private") {
        i += 1;
        Scope::Private
    } else if at(t, i, "friend") {
        i += 1;
        Scope::Friend
    } else {
        if at(t, i, "public") {
            i += 1;
        }
        Scope::Public
    };
    if at(t, i, "static") {
        i += 1;
    }
    let kind = if at(t, i, "sub") {
        ProcKind::Sub
    } else if at(t, i, "function") {
        ProcKind::Function
    } else if at(t, i, "property") {
        i += 1;
        if at(t, i, "get") {
            ProcKind::PropertyGet
        } else if at(t, i, "let") {
            ProcKind::PropertyLet
        } else if at(t, i, "set") {
            ProcKind::PropertySet
        } else {
            return None;
        }
    } else {
        return None;
    };
    i += 1;
    let name = t.get(i)?.clone();
    if !identifier(&name) {
        return None;
    }
    i += 1;
    // VBA type characters are allowed between the name and parameter list.
    if t.get(i).is_some_and(|s| ["$", "%", "&", "!", "#", "@"].contains(&s.as_str())) {
        i += 1;
    }
    let mut params = Vec::new();
    if at(t, i, "(") {
        let start = i + 1;
        let mut depth = 1usize;
        i += 1;
        while i < t.len() && depth != 0 {
            if at(t, i, "(") {
                depth += 1;
            }
            if at(t, i, ")") {
                depth -= 1;
            }
            i += 1;
        }
        if depth != 0 {
            return None;
        }
        let inner = t.get(start..i - 1)?;
        for part in inner.split(|s| s == ",") {
            if let Some(param) = parameter(part) {
                params.push(param);
            }
        }
    }
    Some(Procedure {
        name,
        kind,
        scope,
        params,
        returns: typed(t.get(i..)?),
        line: statement.line,
        event: None,
        udf: false,
        compatibility: Compatibility { level: 1, reasons: Vec::new() },
    })
}

pub(crate) fn analyze(project: &mut Project) -> Result<(), Error> {
    let mut lines_left = MAX_LINES;
    let mut tokens_left = 4_000_000usize;
    let mut remaining = MAX_PROCEDURES;
    let mut bodies = Vec::new();
    let mut declarations = BTreeMap::new();
    for module in &mut project.modules {
        let statements = statements(&module.source, &mut lines_left, &mut tokens_left)?;
        for line in module.source.lines() {
            let t = tokenize(&clean_line(line).0);
            if at(&t, 0, "attribute")
                && at(&t, 1, "vb_name")
                && at(&t, 2, "=")
                && let Some(name) = t.get(3).and_then(|s| s.strip_prefix('"')).and_then(|s| s.strip_suffix('"'))
                && lower(name) != lower(&module.name)
            {
                project.warnings.push(format!("{}: Attribute VB_Name differs from the directory name", module.name));
                break;
            }
        }
        let mut active: Option<(Procedure, Vec<Statement>)> = None;
        let mut module_bodies = Vec::new();
        for statement in statements {
            let t = &statement.tokens;
            if at(t, 0, "option") && at(t, 1, "private") && at(t, 2, "module") {
                module.private_module = true;
            }
            if let Some(i) = t.iter().position(|s| s.eq_ignore_ascii_case("declare")) {
                let name = t
                    .iter()
                    .skip(i + 1)
                    .position(|s| s.eq_ignore_ascii_case("function") || s.eq_ignore_ascii_case("sub"))
                    .and_then(|j| t.get(i + j + 2));
                if let Some(name) = name {
                    declarations.insert(lower(name), name.clone());
                }
            }
            if let Some(proc) = header(&statement) {
                if let Some((open, body)) = active.take() {
                    project.warnings.push(format!("{}.{}: missing procedure terminator", module.name, open.name));
                    module.procedures.push(open);
                    module_bodies.push(body);
                }
                remaining = remaining.checked_sub(1).ok_or(Error::Limit("procedure count"))?;
                active = Some((proc, vec![statement]));
                continue;
            }
            let ending = at(t, 0, "end") && (at(t, 1, "sub") || at(t, 1, "function") || at(t, 1, "property"));
            if ending {
                if let Some((proc, body)) = active.take() {
                    let expected = match proc.kind {
                        ProcKind::Sub => "sub",
                        ProcKind::Function => "function",
                        _ => "property",
                    };
                    if !at(t, 1, expected) {
                        project.warnings.push(format!("{}.{}: mismatched procedure terminator", module.name, proc.name));
                    }
                    module.procedures.push(proc);
                    module_bodies.push(body);
                }
            } else if let Some((_, body)) = &mut active {
                body.push(statement);
            }
        }
        if let Some((proc, body)) = active {
            project.warnings.push(format!("{}.{}: missing procedure terminator", module.name, proc.name));
            module.procedures.push(proc);
            module_bodies.push(body);
        }
        for proc in &mut module.procedures {
            proc.udf =
                proc.kind == ProcKind::Function && proc.scope == Scope::Public && module.kind == ModuleKind::Standard && !module.private_module;
            if (module.kind == ModuleKind::Standard && ["auto_open", "auto_close"].contains(&lower(&proc.name).as_str()))
                || (matches!(module.kind, ModuleKind::Document | ModuleKind::Form) && proc.name.contains('_'))
            {
                proc.event = Some(proc.name.clone());
            }
        }
        bodies.push(module_bodies);
    }
    classify::classify(project, &bodies, &declarations)
}
