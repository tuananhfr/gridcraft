use gridcraft_engine::{
    Session, macros,
    model::{MacroButton, MacroButtonKind},
};
use gridcraft_vba::testing::{FixtureModule, fixture};
use serde_json::json;
use std::sync::Arc;

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    let wb = Arc::make_mut(&mut s.active_mut().unwrap().wb);
    wb.vba_project = Some(fixture(&[
        FixtureModule::new("Module1", "Module", "Public Sub RunMe()\nEnd Sub\nPublic Function Twice(Optional ByVal x As Double = 2) As Double\nTwice = x * 2\nEnd Function\nPrivate Function Hidden()\nEnd Function").unwrap(),
        FixtureModule::new("Platform", "Module", "Public Sub WindowsCall()\nShell \"cmd\"\nEnd Sub\nPublic Sub UseCom()\nCreateObject(\"ETABSv1.Helper\")\nEnd Sub").unwrap(),
        FixtureModule::new("Form1", "BaseClass", "Private Sub CommandButton1_Click()\nEnd Sub").unwrap(),
        FixtureModule::new("ThisWorkbook", "Document", "Private Sub Workbook_Open()\nEnd Sub").unwrap(),
    ], 1252, "", &[]).unwrap());
    wb.macro_buttons.push(MacroButton {
        sheet: 0,
        name: "Button1".into(),
        caption: "Calculate".into(),
        macro_name: "'book.xlsm'!Module1.RunMe".into(),
        kind: MacroButtonKind::FormControl,
    });
    wb.macro_buttons.push(MacroButton {
        sheet: 0,
        name: "Missing".into(),
        caption: "Missing".into(),
        macro_name: "Gone".into(),
        kind: MacroButtonKind::ActiveX,
    });
    s
}

#[test]
fn macro_analysis_command_is_read_only_and_preserves_old_listing() {
    let mut s = session();
    let before = s.doc().unwrap().wb.vba_project.clone();
    let old = s.run("view.macros", json!({})).unwrap();
    assert_eq!(old[0]["name"], "(VBA)");
    let a = s.run("macros.analyze", json!({})).unwrap();
    assert_eq!(a["name"], "Synthetic");
    assert_eq!(a["executed"], false);
    assert_eq!(a["locked"], false);
    assert_eq!(a["signed"], false);
    assert!(a["modules"][0]["lineCount"].as_u64().unwrap() > 0);
    for (name, level) in [("WindowsCall", 2), ("UseCom", 3), ("CommandButton1_Click", 4)] {
        let procedure = a["procedures"].as_array().unwrap().iter().find(|p| p["name"] == name).unwrap();
        assert_eq!(procedure["compatibility"]["level"], level);
        assert!(procedure["compatibility"]["reasons"].as_array().unwrap().iter().any(|r| r["line"].as_u64().unwrap() > 0));
    }
    assert_eq!(a["procedures"][0]["name"], "RunMe");
    let udf = a["procedures"].as_array().unwrap().iter().find(|p| p["name"] == "Twice").unwrap();
    assert_eq!(udf["udf"], true);
    assert_eq!(udf["params"][0]["type_name"], "Double");
    assert_eq!(udf["params"][0]["optional"], true);
    assert_eq!(udf["params"][0]["default"], "2");
    assert_eq!(a["buttons"][0]["exists"], true);
    assert_eq!(a["buttons"][1]["exists"], false);
    assert!(a["procedures"].as_array().unwrap().iter().any(|p| p["event"] == "Workbook_Open"));
    assert_eq!(s.doc().unwrap().wb.vba_project, before);
    let first = macros::project(&s.doc().unwrap().wb).unwrap().unwrap();
    let second = macros::project(&s.doc().unwrap().wb).unwrap().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn macro_udf_mapping_existing_missing_private_no_project_and_caps() {
    let mut s = session();
    for (cell, formula) in
        [("A1", "=_xludf.Twice(3)"), ("A2", "=Twice(4)"), ("A3", "=_xludf.Gone()"), ("A4", "=Hidden()"), ("A5", "=LET(Twice,LAMBDA(x,x*2),Twice(3))")]
    {
        s.execute("cell.set", json!({"cell": cell, "input": formula})).unwrap();
    }
    let a = s.run("macros.analyze", json!({})).unwrap();
    let usage = a["udfUsage"].as_array().unwrap();
    let twice = usage.iter().find(|u| u["name"].as_str().unwrap().eq_ignore_ascii_case("twice")).unwrap();
    assert_eq!(twice["count"], 2);
    assert_eq!(twice["exists"], true);
    let private = usage.iter().find(|u| u["name"].as_str().unwrap().eq_ignore_ascii_case("hidden")).unwrap();
    assert_eq!(private["exists"], true);
    assert_eq!(private["udf"], false);
    let errors = s.run("formulas.errorChecking", json!({})).unwrap();
    let errors = errors["errors"].as_array().unwrap();
    assert_eq!(errors.iter().find(|e| e["cell"] == "A1").unwrap()["cause"], "udfUnavailable");
    assert_eq!(errors.iter().find(|e| e["cell"] == "A3").unwrap()["cause"], "missingUdf");
    assert_eq!(errors.iter().find(|e| e["cell"] == "A4").unwrap()["cause"], "unknownFunction");
    for i in 6..=110 {
        s.execute("cell.set", json!({"cell": format!("A{i}"), "input": "=Twice(2)"})).unwrap();
    }
    let a = s.run("macros.analyze", json!({})).unwrap();
    let twice = a["udfUsage"].as_array().unwrap().iter().find(|u| u["name"].as_str().unwrap().eq_ignore_ascii_case("twice")).unwrap();
    assert_eq!(twice["count"], 107);
    assert_eq!(twice["cells"].as_array().unwrap().len(), 100);
    Arc::make_mut(&mut s.active_mut().unwrap().wb).vba_project = None;
    let errors = s.run("formulas.errorChecking", json!({})).unwrap();
    assert!(errors["errors"].as_array().unwrap().iter().filter(|e| e["error"] == "#NAME?").all(|e| e["cause"] == "unknownFunction"));
}

#[test]
fn macro_malformed_project_reports_structured_error_and_invalidates_cache() {
    let mut s = session();
    assert!(macros::project(&s.doc().unwrap().wb).unwrap().is_ok());
    Arc::make_mut(&mut s.active_mut().unwrap().wb).vba_project = Some(b"broken".to_vec());
    let a = s.run("macros.analyze", json!({})).unwrap();
    assert_eq!(a["error"]["message"], macros::READ_ERROR);
    assert!(!a["error"]["reason"].as_str().unwrap().is_empty());
    assert!(a["procedures"].as_array().unwrap().is_empty());
}
