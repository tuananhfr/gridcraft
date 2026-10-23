//! Read-only VBA inspection shared by commands and calculation diagnostics.
use gridcraft_model::{Cell, Workbook};
use gridcraft_vba::{Project, find_udf};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

pub type Analysis = Result<Arc<Project>, String>;
thread_local! {
    // One bounded entry per thread; failed projects are cached too.
    static CACHE: RefCell<Option<([u8; 32], Analysis)>> = const { RefCell::new(None) };
}

pub fn project(wb: &Workbook) -> Option<Analysis> {
    let bytes = wb.vba_project.as_deref()?;
    let hash: [u8; 32] = Sha256::digest(bytes).into();
    Some(CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((key, result)) = cache.as_ref()
            && *key == hash
        {
            return result.clone();
        }
        let result = gridcraft_vba::analyze(bytes).map(Arc::new).map_err(|e| e.to_string());
        *cache = Some((hash, result.clone()));
        result
    }))
}

pub const UDF_UNAVAILABLE: &str = "VBA function {name} ({module}, line {line}) — macros are not run in GridCraft, so it cannot be calculated yet.";
pub const UDF_RUNTIME: &str = "VBA function error ({module}, line {line}): {detail}";
pub const UDF_LIMIT: &str = "VBA function exceeded its limits ({module}, line {line}): {detail}";

pub fn udf_runtime_diagnostic(doc: &crate::DocState, sheet: usize, cell: gridcraft_core::CellRef) -> Option<Json> {
    let error = doc.calc.external_errors.get(&(sheet, cell))?;
    Some(json!({"cause":"udfRuntime", "reason": if error.limit { UDF_LIMIT } else { UDF_RUNTIME },
        "module": error.module, "line": error.line, "detail": error.message, "suggestion":"View code"}))
}

pub const UDF_MISSING: &str = "Function {name} was not found in this file's macros.";
pub const READ_ERROR: &str = "Could not read the VBA project";

pub fn function_name(name: &str) -> &str {
    if name.get(..7).is_some_and(|prefix| prefix.eq_ignore_ascii_case("_xludf.")) { name.get(7..).unwrap_or(name) } else { name }
}

/// Unknown calls, excluding workbook names and LET/LAMBDA parameters.
pub fn unknown_calls(wb: &Workbook, sheet: usize, cell: &Cell) -> Vec<String> {
    let Some(expr) = cell.formula.as_ref().and_then(|f| f.expr()) else { return vec![] };
    let mut locals = std::collections::HashSet::new();
    expr.walk(&mut |e| {
        if let gridcraft_formula::Expr::Call(name, args) = e {
            for (i, arg) in args.iter().take(args.len().saturating_sub(1)).enumerate() {
                if (name == "LAMBDA" || (name == "LET" && i % 2 == 0))
                    && let gridcraft_formula::Expr::Name(local) = arg
                {
                    locals.insert(local.to_ascii_uppercase());
                }
            }
        }
    });
    let mut calls = Vec::new();
    expr.walk(&mut |e| {
        if let gridcraft_formula::Expr::Call(name, _) = e
            && !gridcraft_calc::is_known_function(name)
            && !locals.contains(&name.to_ascii_uppercase())
            && wb.name(name, sheet).is_none()
        {
            let name = function_name(name).to_string();
            if !calls.iter().any(|n: &String| n.eq_ignore_ascii_case(&name)) {
                calls.push(name);
            }
        }
    });
    calls
}

fn declared_function<'a>(project: &'a Project, name: &str) -> Option<(&'a gridcraft_vba::Module, &'a gridcraft_vba::Procedure)> {
    find_udf(project, name).or_else(|| {
        project.modules.iter().find_map(|module| {
            module
                .procedures
                .iter()
                .find(|procedure| procedure.kind == gridcraft_vba::ProcKind::Function && procedure.name.to_lowercase() == name.to_lowercase())
                .map(|procedure| (module, procedure))
        })
    })
}

pub fn udf_diagnostic(wb: &Workbook, sheet: usize, cell: &Cell) -> Option<Json> {
    udf_diagnostic_with(project(wb).as_ref(), wb, sheet, cell)
}

pub fn udf_diagnostic_with(analysis: Option<&Analysis>, wb: &Workbook, sheet: usize, cell: &Cell) -> Option<Json> {
    let analysis = analysis?;
    let name = unknown_calls(wb, sheet, cell).into_iter().next()?;
    match analysis {
        Ok(project) => {
            if let Some((module, procedure)) = find_udf(project, &name) {
                Some(json!({"cause": "udfUnavailable", "reason": UDF_UNAVAILABLE, "symbol": procedure.name,
                    "module": module.name, "procedure": procedure.name, "line": procedure.line,
                    "suggestion": "View code"}))
            } else if declared_function(project, &name).is_some() {
                Some(json!({"cause": "unknownFunction", "reason": "Unknown function", "symbol": name,
                    "suggestion": "Check the function name or use a supported function."}))
            } else {
                Some(
                    json!({"cause": "missingUdf", "reason": UDF_MISSING, "symbol": name, "suggestion": "Check the function name or use a supported function."}),
                )
            }
        }
        Err(reason) => Some(json!({"cause": "vbaReadError", "reason": READ_ERROR, "symbol": name,
            "detail": reason, "suggestion": "View code"})),
    }
}

fn assigned_to(assigned: &str, module: &str, procedure: &str) -> bool {
    let assigned = assigned.rsplit('!').next().unwrap_or(assigned).trim_matches('\'');
    assigned.eq_ignore_ascii_case(procedure) || assigned.eq_ignore_ascii_case(&format!("{module}.{procedure}"))
}

pub fn inspect(wb: &Workbook) -> Json {
    let analysis = project(wb);
    let (project, error) = match analysis {
        Some(Ok(project)) => (Some(project), Json::Null),
        Some(Err(reason)) => (None, json!({"message": READ_ERROR, "reason": reason})),
        None => (None, Json::Null),
    };
    let mut procedures = Vec::new();
    let mut modules = Vec::new();
    if let Some(project) = &project {
        for module in &project.modules {
            modules.push(json!({"name": module.name, "kind": module.kind, "lineCount": module.source.lines().count(), "source": module.source}));
            for procedure in &module.procedures {
                let mut value = serde_json::to_value(procedure).unwrap_or(Json::Null);
                if let Some(value) = value.as_object_mut() {
                    value.insert("module".into(), json!(module.name));
                    value.insert(
                        "runnableShape".into(),
                        json!(
                            !module.private_module
                                && module.kind == gridcraft_vba::ModuleKind::Standard
                                && procedure.scope == gridcraft_vba::Scope::Public
                                && procedure.kind == gridcraft_vba::ProcKind::Sub
                                && procedure.params.iter().all(|p| p.optional || p.param_array)
                        ),
                    );
                }
                procedures.push(value);
            }
        }
    }
    procedures.sort_by_key(|p| {
        (!p.get("runnableShape").and_then(Json::as_bool).unwrap_or(false), p.get("name").and_then(Json::as_str).unwrap_or_default().to_lowercase())
    });
    let buttons: Vec<Json> = wb
        .macro_buttons
        .iter()
        .map(|button| {
            let exists = project
                .as_ref()
                .is_some_and(|p| p.modules.iter().any(|m| m.procedures.iter().any(|f| assigned_to(&button.macro_name, &m.name, &f.name))));
            json!({"sheet": wb.sheet(button.sheet).map(|s| &s.name), "name": button.name, "caption": button.caption,
            "macro": button.macro_name, "kind": button.kind, "exists": exists})
        })
        .collect();
    let mut usage: BTreeMap<String, Json> = BTreeMap::new();
    for (si, sheet) in wb.sheets.iter().enumerate() {
        for (at, cell) in sheet.cells.iter() {
            for name in unknown_calls(wb, si, cell) {
                let found = project.as_ref().and_then(|p| declared_function(p, &name));
                let mut explicit = false;
                if let Some(expr) = cell.formula.as_ref().and_then(|f| f.expr()) {
                    expr.walk(&mut |e| {
                        if let gridcraft_formula::Expr::Call(called, _) = e
                            && called.len() != function_name(called).len()
                            && function_name(called).eq_ignore_ascii_case(&name)
                        {
                            explicit = true;
                        }
                    });
                }
                if !explicit && found.is_none() {
                    continue;
                }
                let entry = usage.entry(name.to_ascii_uppercase()).or_insert_with(|| {
                    json!({"name": name,
                    "exists": found.is_some(), "udf": found.is_some_and(|(_, f)| f.udf), "module": found.map(|(m, _)| &m.name), "line": found.map(|(_, f)| f.line), "count": 0, "cells": []})
                });
                if let Some(obj) = entry.as_object_mut() {
                    let count = obj.get("count").and_then(Json::as_u64).unwrap_or(0).saturating_add(1);
                    obj.insert("count".into(), json!(count));
                    if let Some(cells) = obj.get_mut("cells").and_then(Json::as_array_mut)
                        && cells.len() < 100
                    {
                        cells.push(json!({"sheet": sheet.name, "cell": at.a1()}));
                    }
                }
            }
        }
    }
    json!({"present": wb.vba_project.is_some(), "executed": false, "error": error,
        "name": project.as_ref().map(|p| &p.name), "locked": project.as_ref().map(|p| p.locked),
        "warnings": project.as_ref().map(|p| &p.warnings), "references": project.as_ref().map(|p| &p.references),
        "signed": wb.vba_signatures.as_ref().is_some_and(|s| s.matches(wb.vba_project.as_deref())) || wb.legacy_vba_signature,
        "modules": modules, "procedures": procedures, "buttons": buttons, "udfUsage": usage.into_values().collect::<Vec<_>>()})
}
