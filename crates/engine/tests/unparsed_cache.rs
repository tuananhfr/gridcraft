use gridcraft_engine::{
    Session,
    core::{CellRef, Value},
    model::{Cell, Formula, Workbook},
};
use serde_json::json;

#[test]
fn unparsed_cache_is_reported_with_transitive_dependents_and_clears_on_edit() {
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    let mut a = Cell::formula(Formula::new("1+"));
    a.value = Value::Number(123.0);
    sh.cells.set(CellRef::new(0, 0), a);
    sh.cells.set(CellRef::new(0, 1), Cell::formula(Formula::new("A1*2")));
    sh.cells.set(CellRef::new(0, 2), Cell::formula(Formula::new("B1+1")));
    sh.cells.set(CellRef::new(0, 3), Cell::formula(Formula::new("INDIRECT(\"A1\")*3")));
    let bytes = gridcraft_engine::io::save_bytes(&wb, "bad.xlsx").unwrap();
    let mut s = Session::new();
    s.execute("file.open", json!({"name": "bad.xlsx", "base64": gridcraft_engine::io::base64_encode(&bytes)})).unwrap();
    for _ in 0..2 {
        assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::new(0, 0)), Value::Number(123.0));
        let errors = s.execute("formulas.errorChecking", json!({})).unwrap();
        assert_eq!(errors["errors"].as_array().unwrap().len(), 4, "{errors}");
        assert_eq!(errors["notCalculated"], 4);
        s.execute("formulas.calculateNow", json!({})).unwrap();
    }
    s.execute("cell.set", json!({"cell": "A1", "input": "=1+1"})).unwrap();
    assert!(s.execute("formulas.errorChecking", json!({})).unwrap()["errors"].as_array().unwrap().is_empty());
    assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::new(0, 1)), Value::Number(4.0));
}
