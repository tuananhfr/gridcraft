//! HTML clipboard transport preserves the internal clipboard's precedence.

use gridcraft_core::{CellRef, Value};
use gridcraft_engine::Session;
use serde_json::json;

fn session() -> Session {
    let mut session = Session::new();
    session.new_workbook();
    session
}

fn value(session: &Session, at: &str) -> Value {
    session.active().unwrap().wb.active().unwrap().value(CellRef::parse(at).unwrap())
}

#[test]
fn external_html_table_supplies_two_by_two_values() {
    let mut s = session();
    let html = "<table><tr><td>Alpha</td><td>2</td></tr><tr><td>Beta</td><td>4</td></tr></table>";
    let result = s.execute("edit.paste", json!({"at": "C3", "text": "plain fallback", "html": html})).unwrap();
    assert_eq!(result["rows"], 2);
    assert_eq!(result["cols"], 2);
    assert_eq!(value(&s, "C3"), Value::text("Alpha"));
    assert_eq!(value(&s, "D3"), Value::number(2.0));
    assert_eq!(value(&s, "C4"), Value::text("Beta"));
    assert_eq!(value(&s, "D4"), Value::number(4.0));
    s.execute("edit.undo", json!({})).unwrap();
    for at in ["C3", "D3", "C4", "D4"] {
        assert_eq!(value(&s, at), Value::Empty);
    }
}

#[test]
fn matching_internal_copy_keeps_formulas_and_styles_despite_stale_html() {
    let mut s = session();
    s.execute("cell.set", json!({"cell": "A1", "input": "=21*2"})).unwrap();
    s.execute("home.bold", json!({"range": "A1"})).unwrap();
    let copied = s.execute("edit.copy", json!({"range": "A1"})).unwrap();
    s.execute("edit.paste", json!({"at": "C3", "text": copied["text"], "html": "<table><tr><td>stale</td></tr></table>"})).unwrap();
    let sheet = s.active().unwrap().wb.active().unwrap();
    let original = sheet.cell(CellRef::parse("A1").unwrap()).unwrap();
    let pasted = sheet.cell(CellRef::parse("C3").unwrap()).unwrap();
    assert_eq!(pasted.value, Value::number(42.0));
    assert_eq!(pasted.formula, original.formula);
    assert_eq!(pasted.style, original.style);
}

#[test]
fn matching_internal_copy_accepts_browser_crlf_line_endings() {
    let mut s = session();
    s.execute("range.setValues", json!({"range": "A1", "values": [["=1+1"], ["=2+2"]]})).unwrap();
    let copied = s.execute("edit.copy", json!({"range": "A1:A2"})).unwrap();
    let text = copied["text"].as_str().unwrap().replace('\n', "\r\n");
    s.execute("edit.paste", json!({"at": "D1", "text": text, "html": "<table><tr><td>stale</td></tr></table>"})).unwrap();
    let sheet = s.active().unwrap().wb.active().unwrap();
    assert!(sheet.cell(CellRef::parse("D1").unwrap()).unwrap().formula.is_some());
    assert!(sheet.cell(CellRef::parse("D2").unwrap()).unwrap().formula.is_some());
    assert_eq!(value(&s, "D2"), Value::number(4.0));
}

#[test]
fn html_only_paste_and_non_table_text_fallback() {
    let mut s = session();
    s.execute("edit.paste", json!({"at": "A1", "html": "<table><tr><td>HTML only</td></tr></table>"})).unwrap();
    assert_eq!(value(&s, "A1"), Value::text("HTML only"));
    s.execute("edit.paste", json!({"at": "B1", "text": "fallback\t7", "html": "<p>not a table</p>"})).unwrap();
    assert_eq!(value(&s, "B1"), Value::text("fallback"));
    assert_eq!(value(&s, "C1"), Value::number(7.0));
}

#[test]
fn oversized_html_falls_back_to_text() {
    let mut s = session();
    let html = format!("<table><tr><td>wrong</td></tr></table>{}", " ".repeat(8 * 1024 * 1024));
    s.execute("edit.paste", json!({"at": "A1", "text": "fallback", "html": html})).unwrap();
    assert_eq!(value(&s, "A1"), Value::text("fallback"));
}

#[test]
fn oversized_html_table_allocation_falls_back_to_text() {
    let mut s = session();
    for html in ["<table><tr><td colspan='16384'>x</td></tr><tr><td colspan='16384'>y</td></tr></table>".into(), "<div>".repeat(65)] {
        s.execute("edit.paste", json!({"at": "A1", "text": "fallback", "html": html})).unwrap();
        assert_eq!(value(&s, "A1"), Value::text("fallback"));
        assert_eq!(value(&s, "B1"), Value::Empty);
    }
}
