use gridcraft_engine::{
    Session,
    core::{CellRef, Value},
    model::{Cell, Formula, Sheet, Workbook},
};
use serde_json::json;
use std::sync::Arc;

fn session() -> Session {
    let mut wb = Workbook::new();
    wb.sheets.push(Arc::new(Sheet::new("Other")));
    for si in 0..2 {
        let sh = wb.sheet_mut(si).unwrap();
        sh.cells.set(CellRef::new(0, 0), Cell::value(Value::Number(2.0)));
        sh.cells.set(CellRef::new(0, 1), Cell::formula(Formula::new("A1*2")));
        sh.cells.set(CellRef::new(0, 2), Cell::formula(Formula::new("B1+1")));
    }
    let mut s = Session::new();
    s.add_document(gridcraft_engine::DocState::new(wb, None, "Test".into())).unwrap();
    s.execute("formulas.calculationOptions", json!({"mode": "manual"})).unwrap();
    s
}

fn value(s: &Session, sheet: usize, cell: &str) -> Value {
    s.doc().unwrap().wb.sheet(sheet).unwrap().value(CellRef::parse(cell).unwrap())
}

#[test]
fn calculate_sheet_preserves_other_sheet_cache() {
    let mut s = session();
    s.execute("cell.set", json!({"sheet": 1, "cell": "A1", "input": "10"})).unwrap();
    s.execute("cell.set", json!({"cell": "A1", "input": "3"})).unwrap();
    let result = s.execute("formulas.calculateSheet", json!({})).unwrap();
    assert_eq!(value(&s, 0, "B1"), Value::Number(6.0));
    assert_eq!(value(&s, 1, "B1"), Value::Number(4.0));
    assert_eq!(result["status"], "partial");
    assert_eq!(result["stale"], 2);
    let result = s.execute("formulas.calculateNow", json!({})).unwrap();
    assert_eq!(value(&s, 1, "B1"), Value::Number(20.0));
    assert_eq!(result["status"], "complete");
}

#[test]
fn manual_structural_edits_preserve_caches_until_calculate() {
    for (command, params, at) in [
        ("formulas.defineName", json!({"name": "Rate", "refersTo": "=2"}), "B1"),
        ("data.sortAscending", json!({"range": "A1:C2", "column": "A", "header": false}), "B1"),
        ("home.insertRows", json!({"rows": "1:1"}), "B2"),
        ("sheet.rename", json!({"name": "Renamed"}), "B1"),
    ] {
        let mut s = session();
        s.execute("cell.set", json!({"cell": "A1", "input": "3"})).unwrap();
        s.execute(command, params).unwrap();
        assert_eq!(value(&s, 0, at), Value::Number(4.0), "{command}");
        s.execute("formulas.calculateNow", json!({})).unwrap();
        assert_eq!(value(&s, 0, at), Value::Number(6.0), "{command}");
    }
}

#[test]
fn selection_calculates_cross_sheet_precedents_only() {
    let mut s = session();
    s.execute("cell.set", json!({"cell": "D1", "input": "=Other!B1+1"})).unwrap();
    s.execute("cell.set", json!({"sheet": 1, "cell": "A1", "input": "10"})).unwrap();
    s.execute("selection.set", json!({"range": "D1"})).unwrap();
    let result = s.execute("formulas.calculateSelection", json!({})).unwrap();
    assert_eq!(value(&s, 0, "D1"), Value::Number(21.0));
    assert_eq!(value(&s, 1, "B1"), Value::Number(20.0));
    assert_eq!(value(&s, 1, "C1"), Value::Number(5.0));
    assert_eq!(result["calculated"], 2);
    assert_eq!(result["status"], "partial");
}

#[test]
fn calculation_status_and_diagnostic_causes() {
    for (formula, cause, symbol) in [
        ("=NOSUCHFN()", "unknownFunction", "NOSUCHFN"),
        ("=_xludf.MyRate()", "unknownFunction", "MyRate"),
        ("=MissingName", "undefinedName", "MissingName"),
        ("='[Book.xlsx]Sheet1'!A1", "externalReference", ""),
    ] {
        let mut s = session();
        s.execute("cell.set", json!({"cell": "D1", "input": formula})).unwrap();
        let result = s.execute("formulas.calculateNow", json!({})).unwrap();
        assert_eq!(result["status"], "error", "{formula}: {result}");
        assert_eq!(result["errors"], 1);
        assert_eq!(result["uncalculated"], 0);
        assert_eq!(result["stale"], 0);
        assert_eq!(result["revision"], s.doc().unwrap().revision);
        let diagnostics = s.execute("formulas.errorChecking", json!({})).unwrap();
        let row = diagnostics["errors"].as_array().unwrap().iter().find(|r| r["cell"] == "D1").unwrap();
        assert_eq!(row["cause"], cause, "{row}");
        assert!(row["symbol"].as_str().unwrap_or_default().to_uppercase().contains(&symbol.to_uppercase()));
        assert!(row["suggestion"].as_str().is_some_and(|s| !s.is_empty()));
    }
}

#[test]
fn calculate_never_touches_vba_project() {
    let mut s = session();
    let project = b"synthetic VBA bytes: must never execute or change".to_vec();
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).vba_project = Some(project.clone());
    for command in ["formulas.calculateNow", "formulas.calculateSheet", "formulas.calculateSelection"] {
        s.execute(command, json!({})).unwrap();
        assert_eq!(s.doc().unwrap().wb.vba_project.as_ref(), Some(&project));
    }
}

#[test]
fn calculate_sheet_reads_other_formulas_from_cache() {
    let mut s = session();
    s.execute("cell.set", json!({"cell": "D1", "input": "=Other!B1+1"})).unwrap();
    s.execute("cell.set", json!({"sheet": 1, "cell": "A1", "input": "10"})).unwrap();
    s.execute("formulas.calculateSheet", json!({})).unwrap();
    assert_eq!(value(&s, 0, "D1"), Value::Number(5.0));
    assert_eq!(value(&s, 1, "B1"), Value::Number(4.0));
}

#[test]
fn selection_follows_dynamic_precedents_without_recalculating_dependents() {
    let mut s = session();
    s.execute("cell.set", json!({"cell": "D1", "input": "=INDIRECT(\"Other!B1\")+1"})).unwrap();
    s.execute("cell.set", json!({"sheet": 1, "cell": "A1", "input": "10"})).unwrap();
    s.execute("selection.set", json!({"range": "D1"})).unwrap();
    let result = s.execute("formulas.calculateSelection", json!({})).unwrap();
    assert_eq!(value(&s, 0, "D1"), Value::Number(21.0));
    assert_eq!(value(&s, 1, "B1"), Value::Number(20.0));
    assert_eq!(value(&s, 1, "C1"), Value::Number(5.0));
    assert_eq!(result["calculated"], 2);
}

#[test]
fn manual_stale_and_unparsed_and_circular_cells_are_not_complete() {
    let mut s = session();
    s.execute("cell.set", json!({"cell": "A1", "input": "3"})).unwrap();
    let diagnostics = s.execute("formulas.errorChecking", json!({})).unwrap();
    assert!(diagnostics["errors"].as_array().unwrap().iter().any(|row| row["cause"] == "staleCache"));
    s.execute("selection.set", json!({"range": "Z1"})).unwrap();
    let result = s.execute("formulas.calculateSelection", json!({})).unwrap();
    assert_eq!(result["status"], "partial");
    assert_eq!(result["stale"], 2);
    assert_eq!(result["calculated"], 0);
    s.execute("cell.set", json!({"cell": "D1", "input": "=D1+1"})).unwrap();
    assert_eq!(s.execute("formulas.calculateNow", json!({})).unwrap()["status"], "error");
    let diagnostics = s.execute("formulas.errorChecking", json!({})).unwrap();
    assert!(diagnostics["errors"].as_array().unwrap().iter().any(|row| row["cause"] == "circularReference"));
    let d = s.doc_mut().unwrap();
    let mut cell = Cell::formula(Formula::new("1+"));
    cell.value = Value::Number(123.0);
    Arc::make_mut(&mut d.wb).sheet_mut(0).unwrap().cells.set(CellRef::new(0, 3), cell);
    let result = s.execute("formulas.calculateNow", json!({})).unwrap();
    assert_eq!(result["status"], "partial");
    assert_eq!(result["uncalculated"], 1);
}

#[test]
fn spilled_errors_never_report_complete() {
    let mut s = session();
    s.execute("cell.set", json!({"cell": "D1", "input": "=1/{1,0}"})).unwrap();
    let result = s.execute("formulas.calculateNow", json!({})).unwrap();
    assert_eq!(result["status"], "error");
    assert_eq!(result["errors"], 1);
    let diagnostics = s.execute("formulas.errorChecking", json!({})).unwrap();
    assert!(diagnostics["errors"].as_array().unwrap().iter().any(|row| row["cell"] == "E1"));
}
