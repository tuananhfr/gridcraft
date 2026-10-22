#[path = "../../mdb/tests/support/mod.rs"]
mod support;

use gridcraft_core::{CellRef, Value};
use gridcraft_engine::{Session, io};
use serde_json::json;

#[test]
fn import_access_lists_and_imports_in_one_undo_step() {
    for jet4 in [false, true] {
        let bytes = support::etabs(jet4);
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        let data = io::base64_encode(&bytes);
        let before = s.doc().unwrap().wb.clone();
        let listed = s.execute("data.importAccess", json!({"data": data})).unwrap();
        assert!(listed["tables"].as_array().unwrap().iter().any(|t| t["name"] == support::SUPPORT));
        assert_eq!(*s.doc().unwrap().wb, *before);
        s.execute("data.importAccess", json!({"data": data, "table": support::SUPPORT, "sheet": "Access data"})).unwrap();
        let wb = &s.doc().unwrap().wb;
        let sh = wb.sheet(wb.sheet_index("Access data").unwrap()).unwrap();
        assert_eq!(sh.value(CellRef::new(0, 0)), Value::Text("Joint".into()));
        assert!(wb.styles.get(sh.cell(CellRef::new(0, 0)).unwrap().style).font.bold);
        assert_eq!(sh.value(CellRef::new(1, 1)), Value::Number(-123.5));
        assert_eq!(sh.value(CellRef::new(1, 2)), Value::Bool(true));
        assert_eq!(sh.value(CellRef::new(1, 3)), Value::Number(45000.5));
        assert_eq!(wb.styles.get(sh.cell(CellRef::new(1, 3)).unwrap().style).num_fmt.as_str(), "yyyy-mm-dd hh:mm:ss");
        assert_eq!(sh.value(CellRef::new(1, 9)), Value::Number(-12.3456));
        assert_eq!(sh.value(CellRef::new(1, 10)), Value::Number(-123.45));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().wb, *before);
        s.execute("edit.redo", json!({})).unwrap();
        assert!(s.doc().unwrap().wb.sheet_index("Access data").is_some());
    }
}

#[test]
fn import_access_bad_input_is_atomic_and_never_opens_dialogs() {
    let mut s = Session::new();
    s.execute("file.new", json!({})).unwrap();
    let before = s.doc().unwrap().wb.clone();
    for p in [json!({}), json!({"data":"!"}), json!({"data":"AA=="}), json!({"data": 4})] {
        assert!(s.execute("data.importAccess", p).is_err());
        assert_eq!(*s.doc().unwrap().wb, *before);
    }
    assert!(s.ui_requests.is_empty());
}

#[test]
fn import_access_real_jet_fixtures_through_session() {
    for bytes in [include_bytes!("../../mdb/tests/fixtures/jet3.mdb").as_slice(), include_bytes!("../../mdb/tests/fixtures/jet4.mdb").as_slice()] {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        let data = io::base64_encode(bytes);
        let list = s.execute("data.importAccess", json!({"data": data})).unwrap();
        assert!(list["tables"].as_array().unwrap().iter().any(|t| t["name"] == "Table1"));
        s.execute("data.importAccess", json!({"data": data, "table": "Table1"})).unwrap();
        let imported = s.doc().unwrap().wb.active().unwrap();
        assert!(!matches!(imported.value(CellRef::new(0, 0)), Value::Empty));
    }
}

#[test]
fn import_access_named_sheet_validation_and_journal_redaction() {
    let mut s = Session::new();
    s.execute("file.new", json!({})).unwrap();
    let data = io::base64_encode(&support::etabs(true));
    let before = s.doc().unwrap().wb.clone();
    for params in [
        json!({"data": data, "table": "not a table"}),
        json!({"data": data, "table": support::SUPPORT, "sheet": "bad/name"}),
        json!({"data": data, "table": 1}),
        json!({"data": data, "sheet": false}),
    ] {
        assert!(s.execute("data.importAccess", params).is_err());
        assert_eq!(*s.doc().unwrap().wb, *before);
    }
    let name = s.doc().unwrap().wb.active().unwrap().name.clone();
    let sheet_count = s.doc().unwrap().wb.sheets.len();
    s.execute("data.importAccess", json!({"data": data, "table": support::SUPPORT, "sheet": name})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheets.len(), sheet_count);
    assert!(s.journal.iter().all(|(_, params)| params.get("data").is_none()));
}

#[test]
fn import_access_dates_follow_the_workbook_epoch() {
    let mut s = Session::new();
    s.execute("file.new", json!({})).unwrap();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).date_system = gridcraft_core::DateSystem::D1904;
    s.execute("data.importAccess", json!({"data": io::base64_encode(&support::etabs(true)), "table": support::SUPPORT})).unwrap();
    assert_eq!(s.doc().unwrap().wb.active().unwrap().value(CellRef::new(1, 3)), Value::Number(43538.5));
}
