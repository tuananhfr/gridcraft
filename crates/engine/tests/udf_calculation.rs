use gridcraft_engine::{
    Session,
    core::{CellError, CellRef, Value},
};
use gridcraft_vba::testing::{FixtureModule, fixture};
use serde_json::json;
use std::sync::Arc;
fn session(source: &str) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).vba_project =
        Some(fixture(&[FixtureModule::new("Module1", "Module", source).unwrap()], 1252, "", &[]).unwrap());
    s
}
fn set(s: &mut Session, cell: &str, input: &str) {
    s.execute("cell.set", json!({"cell":cell,"input":input})).unwrap();
}
fn value(s: &Session, cell: &str) -> Value {
    s.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::parse(cell).unwrap())
}
fn allow(s: &mut Session, allow: bool) {
    s.execute("macros.consent", json!({"allow":allow})).unwrap();
}
#[test]
fn udf_scalar_consent_revoke_and_recalculate() {
    let mut s = session("Public Function Twice(x As Double) As Double\nTwice=x*2\nEnd Function");
    set(&mut s, "A1", "3");
    set(&mut s, "B1", "=_xludf.Twice(A1)");
    set(&mut s, "C1", "=Twice(4)");
    assert_eq!(value(&s, "B1"), Value::Error(CellError::Name));
    allow(&mut s, true);
    assert_eq!(value(&s, "B1"), Value::Number(6.0));
    assert_eq!(value(&s, "C1"), Value::Number(8.0));
    set(&mut s, "A1", "9");
    assert_eq!(value(&s, "B1"), Value::Number(18.0));
    allow(&mut s, false);
    assert_eq!(value(&s, "B1"), Value::Error(CellError::Name));
}
#[test]
fn udf_read_only_range_sum_and_precedents() {
    let mut s = session("Public Function Total(r As Range) As Double\nDim c As Object\nFor Each c In r\nTotal=Total+c.Value\nNext c\nEnd Function");
    allow(&mut s, true);
    set(&mut s, "A1", "2");
    set(&mut s, "A2", "3");
    set(&mut s, "B1", "=Total(A1:A2)");
    assert_eq!(value(&s, "B1"), Value::Number(5.0));
    set(&mut s, "A2", "8");
    assert_eq!(value(&s, "B1"), Value::Number(10.0));
}
#[test]
fn udf_runtime_error_has_module_line_and_detail() {
    let mut s = session("Public Function Bad()\nErr.Raise 5, \"udf\", \"broken udf\"\nEnd Function");
    allow(&mut s, true);
    set(&mut s, "A1", "=Bad()");
    assert_eq!(value(&s, "A1"), Value::Error(CellError::Value));
    let errors = s.execute("formulas.errorChecking", json!({})).unwrap();
    let error = &errors["errors"][0];
    assert_eq!(error["cause"], "udfRuntime");
    assert_eq!(error["module"], "Module1");
    assert_eq!(error["line"], 2);
    assert_eq!(error["detail"], "broken udf");
}
#[test]
fn udf_refuses_write_and_interaction() {
    for body in ["Range(\"B1\").Value=9", "MsgBox \"hello\"", "Dim answer\nanswer=InputBox(\"hello\")"] {
        let mut s = session(&format!("Public Function Bad()\n{body}\nBad=1\nEnd Function"));
        allow(&mut s, true);
        set(&mut s, "B1", "7");
        set(&mut s, "A1", "=Bad()");
        assert_eq!(value(&s, "A1"), Value::Error(CellError::Value), "{body}");
        assert_eq!(value(&s, "B1"), Value::Number(7.0));
        assert!(s.take_ui_requests().is_empty());
    }
}
#[test]
fn udf_step_limit_is_reported() {
    let mut s = session("Public Function Forever()\nDo\nLoop\nEnd Function");
    allow(&mut s, true);
    set(&mut s, "A1", "=Forever()");
    assert_eq!(value(&s, "A1"), Value::Error(CellError::Value));
    assert!(s.doc().unwrap().calc.external_errors.get(&(0, CellRef::new(0, 0))).unwrap().limit);
}
#[test]
fn udf_builtin_collision_requires_xludf_prefix() {
    let mut s = session("Public Function SUM(x As Double) As Double\nSUM=x*10\nEnd Function");
    allow(&mut s, true);
    set(&mut s, "A1", "=SUM(2)");
    set(&mut s, "A2", "=_xludf.SUM(2)");
    assert_eq!(value(&s, "A1"), Value::Number(2.0));
    assert_eq!(value(&s, "A2"), Value::Number(20.0));
}
#[test]
fn udf_volatile_only_when_requested() {
    for (body, expected) in [("", 0), ("Application.Volatile\n", 1)] {
        let mut s = session(&format!("Public Function Constant()\n{body}Constant=4\nEnd Function"));
        allow(&mut s, true);
        set(&mut s, "A1", "=Constant()");
        assert_eq!(value(&s, "A1"), Value::Number(4.0));
        set(&mut s, "B1", "1");
        assert_eq!(s.doc().unwrap().calc.last_recalc_cells, expected, "{body}");
    }
}
#[test]
fn udf_worksheetfunction_and_implicit_range_read() {
    let mut s = session("Public Function Total()\nTotal=Application.WorksheetFunction.Sum(Range(\"A1:A2\"))\nEnd Function");
    allow(&mut s, true);
    set(&mut s, "A1", "2");
    set(&mut s, "A2", "3");
    set(&mut s, "B1", "=Total()");
    assert_eq!(value(&s, "B1"), Value::Number(5.0));
    set(&mut s, "A2", "9");
    assert_eq!(value(&s, "B1"), Value::Number(11.0));
}

#[test]
fn udf_range_limit_is_reported_before_copying() {
    let mut s = session("Public Function Total(r As Range)\nTotal=r.Count\nEnd Function");
    allow(&mut s, true);
    set(&mut s, "B1", "=Total(A1:A5000)");
    assert_eq!(value(&s, "B1"), Value::Error(CellError::Value));
    assert!(s.doc().unwrap().calc.external_errors.get(&(0, CellRef::new(0, 1))).unwrap().limit);
}

#[test]
fn udf_range_value_is_a_two_dimensional_vba_array() {
    let mut s = session("Public Function Pick(r As Object)\nDim a As Variant\na=r.Value2\nPick=a(2,2)\nEnd Function");
    allow(&mut s, true);
    set(&mut s, "B2", "17");
    set(&mut s, "D1", "=Pick(A1:B2)");
    assert_eq!(value(&s, "D1"), Value::Number(17.0));
}

#[test]
fn udf_bracket_evaluation_reads_and_tracks_precedents() {
    let mut s = session("Function BracketTotal()\nBracketTotal = [A1] * 2 + [SUM(A1:A2)] + Application.Evaluate(\"1+2\")\nEnd Function");
    set(&mut s, "A1", "4");
    set(&mut s, "A2", "5");
    set(&mut s, "B1", "=BracketTotal()");
    allow(&mut s, true);
    assert_eq!(value(&s, "B1"), Value::Number(20.0));
    set(&mut s, "A2", "7");
    assert_eq!(value(&s, "B1"), Value::Number(22.0));
}

#[test]
fn udf_bracket_evaluation_cannot_write() {
    let mut s = session("Function BadWrite()\n[A1].Value = 99\nBadWrite = 1\nEnd Function");
    set(&mut s, "A1", "4");
    set(&mut s, "B1", "=BadWrite()");
    allow(&mut s, true);
    assert_eq!(value(&s, "B1"), Value::Error(CellError::Value));
    assert_eq!(value(&s, "A1"), Value::Number(4.0));
}

#[test]
fn udf_compile_failure_in_other_module_does_not_block_evaluation() {
    let mut s = session("Function Healthy()\nHealthy = Worksheets(1).[A1].Value + 1\nEnd Function");
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).vba_project = Some(
        fixture(
            &[
                FixtureModule::new("Module1", "Module", "Function Healthy()\nHealthy = Worksheets(1).[A1].Value + 1\nEnd Function").unwrap(),
                FixtureModule::new("Broken", "Module", "Function Bad()\nDim =\nEnd Function").unwrap(),
            ],
            1252,
            "",
            &[],
        )
        .unwrap(),
    );
    set(&mut s, "A1", "4");
    set(&mut s, "B1", "=Healthy()");
    allow(&mut s, true);
    assert_eq!(value(&s, "B1"), Value::Number(5.0));
}
