use gridcraft_engine::Session;
use serde_json::json;

#[test]
fn history_reports_transaction_metadata() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell":"A1", "input":"1"})).unwrap();
    let h = s.execute("history.list", json!({})).unwrap();
    assert_eq!(h["undo"][0]["transactionId"], 1);
    assert!(h["undo"][0]["timestamp"].as_u64().is_some());
}

#[test]
fn large_range_gestures_restore_entire_workbook() {
    use gridcraft_engine::cell;
    let mut s = Session::new();
    s.new_workbook();
    for r in 0..20 {
        for c in 0..50 {
            let input = match (r + c) % 3 {
                0 => format!("={r}+{c}"),
                1 => format!("text {r}:{c}"),
                _ => (r * 50 + c).to_string(),
            };
            s.execute("cell.set", json!({"cell":cell(r,c).a1(), "input":input})).unwrap();
        }
    }
    s.execute("home.bold", json!({"range":"A1:AX10"})).unwrap();
    s.execute("home.italic", json!({"range":"B1:C20"})).unwrap();
    s.execute("edit.copy", json!({"range":"A1:AX20"})).unwrap();
    s.doc_mut().unwrap().undo.clear();
    for (command, params) in [
        ("edit.paste", json!({"at":"A25"})),
        ("data.sortDescending", json!({"range":"A1:AX20", "column":"A"})),
        ("edit.fillDown", json!({"range":"A1:AX20"})),
        ("edit.clearAll", json!({"range":"A1:AX20"})),
    ] {
        let before = serde_json::to_value(&*s.doc().unwrap().wb).unwrap();
        let count = s.doc().unwrap().undo.len();
        s.execute(command, params).unwrap();
        let after = serde_json::to_value(&*s.doc().unwrap().wb).unwrap();
        assert_ne!(before, after);
        assert_eq!(s.doc().unwrap().undo.len(), (count + 1).min(100));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(serde_json::to_value(&*s.doc().unwrap().wb).unwrap(), before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(serde_json::to_value(&*s.doc().unwrap().wb).unwrap(), after);
    }
}

#[test]
fn metadata_survives_undo_redo_and_new_edit_clears_redo() {
    let mut s = Session::new();
    s.history_clock = Some(|| 123456);
    s.new_workbook();
    s.execute("cell.set", json!({"cell":"A1", "input":"1"})).unwrap();
    let first = s.execute("history.list", json!({})).unwrap()["undo"][0].clone();
    assert_eq!(first["timestamp"], 123456);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.execute("history.list", json!({})).unwrap()["redo"][0], first);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.execute("history.list", json!({})).unwrap()["undo"][0], first);
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("cell.set", json!({"cell":"A1", "input":"2"})).unwrap();
    assert!(s.doc().unwrap().redo.is_empty());
    assert!(s.doc().unwrap().undo.last().unwrap().transaction_id > 1);
    assert_eq!(s.journal_records().last().unwrap()["timestamp"], 123456);
}

#[test]
fn nested_repeat_is_one_transaction() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("home.bold", json!({"range":"A1"})).unwrap();
    s.execute("selection.set", json!({"range":"B2"})).unwrap();
    let count = s.doc().unwrap().undo.len();
    let journal_count = s.journal.len();
    let before = serde_json::to_value(&*s.doc().unwrap().wb).unwrap();
    s.execute("edit.repeat", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().undo.len(), count + 1);
    assert_eq!(s.journal.len(), journal_count + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(serde_json::to_value(&*s.doc().unwrap().wb).unwrap(), before);
}

#[test]
fn script_composite_has_one_outer_transaction() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute(
        "automate.newScript",
        json!({"name":"two edits", "commands":[
            {"command":"cell.set", "params":{"cell":"A1", "input":"1"}},
            {"command":"cell.set", "params":{"cell":"B1", "input":"=A1+1"}}
        ]}),
    )
    .unwrap();
    let before = serde_json::to_value(&*s.doc().unwrap().wb).unwrap();
    let count = s.doc().unwrap().undo.len();
    let journal_count = s.journal.len();
    s.execute("automate.runScript", json!({"name":"two edits"})).unwrap();
    let after = serde_json::to_value(&*s.doc().unwrap().wb).unwrap();
    assert_eq!(s.doc().unwrap().undo.len(), count + 1);
    assert_eq!(s.journal.len(), journal_count + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(serde_json::to_value(&*s.doc().unwrap().wb).unwrap(), before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(serde_json::to_value(&*s.doc().unwrap().wb).unwrap(), after);
}

#[test]
fn undo_redo_restores_calculation_staleness() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell":"A1", "input":"2"})).unwrap();
    s.execute("cell.set", json!({"cell":"B1", "input":"=A1*2"})).unwrap();
    s.execute("cell.set", json!({"cell":"A1", "input":"3"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.doc().unwrap().calc.stale.is_empty(), "restored automatic caches are already calculated");
    s.execute("edit.redo", json!({})).unwrap();
    assert!(s.doc().unwrap().calc.stale.is_empty());
    s.execute("formulas.calculationOptions", json!({"mode":"manual"})).unwrap();
    s.execute("cell.set", json!({"cell":"A1", "input":"4"})).unwrap();
    let stale = s.doc().unwrap().calc.stale.clone();
    assert!(!stale.is_empty());
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.doc().unwrap().calc.stale.is_empty());
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().calc.stale, stale);
}
