//! Synthetic macro performance and automatic-calculation regressions.
#![cfg(not(target_arch = "wasm32"))]
use gridcraft_engine::{
    Session,
    core::{CellRef, Value},
    macro_host::MacroOptions,
};
use gridcraft_vba::testing::{FixtureModule, fixture};
use serde_json::json;
use std::sync::Arc;

fn session(source: &str) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).vba_project =
        Some(fixture(&[FixtureModule::new("Module1", "Module", source).unwrap()], 1252, "", &[]).unwrap());
    s.execute("macros.consent", json!({"allow":true})).unwrap();
    s
}

#[test]
#[ignore = "synthetic release timing; run with --ignored --nocapture"]
#[allow(clippy::disallowed_methods)] // Opt-in native benchmark clock.
fn macro_loop_host_benchmark() {
    let mut s = session(
        "Sub Main()\nDim i As Long\nDim x As Double\nDim a(1 To 20000) As Double\nFor i = 1 To 3000\na(i) = i\nCells(i, 1) = a(i)\nx = Application.WorksheetFunction.Max(Cells(i, 1), 2)\nCells(i, 2) = x\nNext i\nEnd Sub",
    );
    let start = std::time::Instant::now();
    let mut task = s.start_macro("Main", MacroOptions { wall_ms: 120_000, ..Default::default() }).unwrap();
    let result = loop {
        if let Some(result) = s.step_macro(&mut task).unwrap() {
            break result;
        }
    };
    let elapsed = start.elapsed();
    println!("synthetic ms={} steps={} cellsChanged={}", elapsed.as_millis(), result["steps"], result["cellsChanged"]);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["cellsChanged"], 6000);
    assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::parse("B3000").unwrap()), Value::Number(3000.0));
    assert_eq!(s.doc().unwrap().undo.len(), 1);
    assert!(elapsed.as_secs() < 60);
}

#[test]
fn macro_array_reads_and_writes_keep_automatic_formula_values_fresh() {
    let mut s = session(
        "Sub Main()\nDim a(1 To 3) As Double\nDim i As Long\nRange(\"B1\").Formula = \"=A1*2\"\nFor i = 1 To 3\na(i) = i\nRange(\"A1\") = a(i)\nCells(i, 3) = Application.WorksheetFunction.Max(Range(\"B1\"), 1)\nNext i\nEnd Sub",
    );
    let result = s.execute("macros.run", json!({"name":"Main"})).unwrap();
    assert_eq!(result["status"], "completed", "{result}");
    for (row, expected) in [(0, 2.0), (1, 4.0), (2, 6.0)] {
        assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::new(row, 2)), Value::Number(expected));
    }
    assert_eq!(s.doc().unwrap().undo.len(), 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::new(0, 0)), Value::Empty);
}
