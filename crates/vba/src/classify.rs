//! Compatibility vocabulary lives here for extension by a future interpreter.
use crate::source::{Statement, lower};
use crate::{Error, MAX_REASONS, ModuleKind, Project, Reason};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const WINDOWS: &[(&str, &str)] = &[
    ("shell", "shell"),
    ("sendkeys", "send_keys"),
    ("environ", "environment"),
    ("environ$", "environment"),
    ("kill", "file_io"),
    ("filecopy", "file_io"),
    ("mkdir", "file_io"),
    ("rmdir", "file_io"),
    ("dir", "file_io"),
    ("filesystemobject", "file_io"),
];
const COM: &[(&str, &str)] = &[
    ("createobject", "create_object"),
    ("getobject", "get_object"),
    ("dao", "dao"),
    ("adodb", "adodb"),
    ("opendatabase", "dao"),
    ("autocad", "autocad"),
    ("etabs", "etabs"),
];
const NATIVE_CLASSES: &[&str] = &[
    "collection",
    "object",
    "variant",
    "string",
    "boolean",
    "byte",
    "integer",
    "long",
    "longlong",
    "longptr",
    "single",
    "double",
    "currency",
    "date",
    "decimal",
    "range",
    "worksheet",
    "worksheets",
    "workbook",
    "workbooks",
    "application",
    "chart",
    "charts",
];
const EXCEL_EVENTS: &[&str] = &["workbook_", "worksheet_", "application_", "app_"];

fn windows_code(word: &str) -> Option<&'static str> {
    WINDOWS.iter().find(|(keyword, _)| word == *keyword).map(|&(_, code)| code)
}

fn com_code(word: &str) -> Option<&'static str> {
    // ETABS automation libraries are versioned (ETABSv1, ETABS2016, ...).
    COM.iter().find(|(keyword, _)| word == *keyword || (*keyword == "etabs" && word.starts_with(keyword))).map(|&(_, code)| code)
}

fn reason(project: &mut Project, mi: usize, pi: usize, level: u8, code: &str, detail: String, line: u32, budget: &mut usize) -> Result<(), Error> {
    *budget = budget.checked_sub(1).ok_or(Error::Limit("compatibility reasons"))?;
    let proc = project.modules.get_mut(mi).and_then(|m| m.procedures.get_mut(pi)).ok_or(Error::Invalid("procedure index"))?;
    proc.compatibility.level = proc.compatibility.level.max(level);
    proc.compatibility.reasons.push(Reason { code: code.into(), detail, line });
    Ok(())
}

pub(crate) fn classify(project: &mut Project, bodies: &[Vec<Vec<Statement>>], declarations: &BTreeMap<String, String>) -> Result<(), Error> {
    let mut budget = MAX_REASONS;
    let mut names: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut qualified = BTreeMap::new();
    let mut local = BTreeMap::new();
    let forms: BTreeSet<_> = project.modules.iter().filter(|m| m.kind == ModuleKind::Form).map(|m| lower(&m.name)).collect();
    let mut indices = Vec::new();
    let classes: BTreeSet<_> = project.modules.iter().filter(|m| m.kind == ModuleKind::Class).map(|m| lower(&m.name)).collect();
    for (mi, module) in project.modules.iter().enumerate() {
        for (pi, proc) in module.procedures.iter().enumerate() {
            let id = indices.len();
            local.insert((mi, lower(&proc.name)), id);
            names.entry(lower(&proc.name)).or_default().push(id);
            qualified.insert(format!("{}.{}", lower(&module.name), lower(&proc.name)), id);
            indices.push((mi, pi));
        }
    }
    let mut edges = Vec::new();
    for (id, &(mi, pi)) in indices.iter().enumerate() {
        let proc = project.modules.get(mi).and_then(|m| m.procedures.get(pi)).ok_or(Error::Invalid("procedure index"))?;
        let name = lower(&proc.name);
        let display = proc.name.clone();
        let event_line = proc.line;
        let module_kind = project.modules.get(mi).map(|m| m.kind).ok_or(Error::Invalid("module index"))?;
        let active_event = proc.event.is_some()
            && (module_kind == ModuleKind::Form
                || (module_kind == ModuleKind::Document && !EXCEL_EVENTS.iter().any(|prefix| name.starts_with(prefix))));
        if active_event {
            reason(project, mi, pi, 4, "activex_event", display, event_line, &mut budget)?;
        }
        let body = bodies.get(mi).and_then(|b| b.get(pi)).ok_or(Error::Invalid("procedure body index"))?;
        for (statement_index, statement) in body.iter().enumerate() {
            let tokens = &statement.tokens;
            let words: Vec<_> = tokens.iter().map(|s| lower(s)).collect();
            for (i, word) in words.iter().enumerate() {
                let line = statement.token_lines.get(i).copied().unwrap_or(statement.line);
                if word.starts_with('"') {
                    continue;
                }
                let original = tokens.get(i).cloned().unwrap_or_default();
                let after_dot = i.checked_sub(1).and_then(|j| words.get(j)).is_some_and(|s| s == ".");
                if let Some(code) = windows_code(word) {
                    reason(project, mi, pi, 2, code, original.clone(), line, &mut budget)?;
                }
                // `Workbooks.Open` is the Excel method, not the file statement.
                if word == "open" && !after_dot && words.iter().skip(i + 1).any(|s| s == "for") {
                    reason(project, mi, pi, 2, "file_io", "Open ... For".into(), line, &mut budget)?;
                }
                if let Some(code) = com_code(word) {
                    let factory = matches!(code, "create_object" | "get_object");
                    let literal_index = if code == "get_object" { i + 3 } else { i + 2 };
                    let prog_id = if factory && words.get(i + 1).is_some_and(|s| s == "(") {
                        tokens.get(literal_index).filter(|s| s.starts_with('"')).map(|s| s.trim_matches('"').to_ascii_lowercase())
                    } else {
                        None
                    };
                    let covered_factory = prog_id.as_deref().is_some_and(|p| {
                        p == "autocad.application"
                            || (code == "create_object"
                                && (p == "dao.dbengine"
                                    || p.strip_prefix("dao.dbengine.").is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))))
                    }) && (code != "get_object" || words.get(i + 2).is_some_and(|s| s == ","));
                    let known_dao_type = word == "dao"
                        && words.get(i + 1).is_some_and(|s| s == ".")
                        && words.get(i + 2).is_some_and(|s| {
                            matches!(
                                s.as_str(),
                                "database"
                                    | "recordset"
                                    | "dbengine"
                                    | "workspace"
                                    | "workspaces"
                                    | "field"
                                    | "fields"
                                    | "tabledef"
                                    | "tabledefs"
                                    | "opendatabase"
                            )
                        });
                    let covered = if factory {
                        covered_factory && words.get(literal_index + 1).is_some_and(|s| s == ")")
                    } else {
                        word == "opendatabase" || known_dao_type
                    };
                    let detail = prog_id.as_ref().map_or_else(|| original.clone(), |p| format!("{original}(\"{p}\")"));
                    reason(project, mi, pi, if covered { 1 } else { 3 }, if covered { "emulated_com" } else { code }, detail, line, &mut budget)?;
                    // The ProgID names the automated library, e.g. CreateObject("Scripting.FileSystemObject").
                    if let Some(id) = prog_id.filter(|_| !covered) {
                        for part in id.split('.') {
                            if let Some(code) = com_code(part).filter(|c| !matches!(*c, "create_object" | "get_object")) {
                                reason(project, mi, pi, 3, code, id.clone(), line, &mut budget)?;
                            } else if let Some(code) = windows_code(part) {
                                reason(project, mi, pi, 2, code, id.clone(), line, &mut budget)?;
                            }
                        }
                    }
                }
                if word == "new"
                    && let Some(class) = words.get(i + 1)
                {
                    let excel = class == "excel" || class == "vba";
                    if !excel && !NATIVE_CLASSES.contains(&class.as_str()) && !classes.contains(class) && !forms.contains(class) {
                        reason(project, mi, pi, 3, "new_object", class.clone(), line, &mut budget)?;
                    }
                }
                let is_form = forms.contains(word);
                if word == "msforms" || word == "userform" || word.starts_with("userform") || is_form {
                    reason(project, mi, pi, 4, "userform", original.clone(), line, &mut budget)?;
                }
                if let Some(declare) = declarations.get(word) {
                    reason(project, mi, pi, 2, "declare", declare.clone(), line, &mut budget)?;
                }
                // Procedure headers are classified for types, but do not call themselves.
                if statement_index == 0 {
                    continue;
                }
                let previous = i.checked_sub(1).and_then(|j| words.get(j));
                let target = if previous.is_some_and(|s| s == ".") {
                    i.checked_sub(2).and_then(|j| words.get(j)).and_then(|module| qualified.get(&format!("{module}.{word}")).copied())
                } else {
                    local
                        .get(&(mi, word.clone()))
                        .copied()
                        .or_else(|| names.get(word).and_then(|candidates| if candidates.len() == 1 { candidates.first().copied() } else { None }))
                };
                // Recursion (and a Function assigning its own return value) adds nothing to its own level.
                if let Some(target) = target.filter(|&target| target != id) {
                    if edges.len() >= MAX_REASONS {
                        return Err(Error::Limit("call graph edges"));
                    }
                    edges.push((id, target, line));
                }
            }
        }
    }
    // Monotone levels converge even when the call graph contains recursion cycles.
    let mut levels: Vec<_> =
        indices.iter().filter_map(|&(mi, pi)| project.modules.get(mi)?.procedures.get(pi).map(|p| p.compatibility.level)).collect();
    let mut reverse = vec![Vec::new(); indices.len()];
    for &(caller, target, _) in &edges {
        if let Some(callers) = reverse.get_mut(target) {
            callers.push(caller);
        }
    }
    let mut queue: VecDeque<_> = (0..indices.len()).collect();
    while let Some(target) = queue.pop_front() {
        let level = levels.get(target).copied().ok_or(Error::Invalid("call target"))?;
        if let Some(callers) = reverse.get(target) {
            for &caller in callers {
                let current = levels.get_mut(caller).ok_or(Error::Invalid("caller"))?;
                if *current < level {
                    *current = level;
                    queue.push_back(caller);
                }
            }
        }
    }
    for (caller, target, line) in edges {
        let level = levels.get(target).copied().ok_or(Error::Invalid("call level"))?;
        if level > 1 {
            let (mi, pi) = indices.get(caller).copied().ok_or(Error::Invalid("caller index"))?;
            let (tm, tp) = indices.get(target).copied().ok_or(Error::Invalid("target index"))?;
            let name = project.modules.get(tm).and_then(|m| m.procedures.get(tp)).map(|p| p.name.clone()).ok_or(Error::Invalid("call name"))?;
            reason(project, mi, pi, level, "calls", format!("calls {name} (level {level})"), line, &mut budget)?;
        }
    }
    Ok(())
}
