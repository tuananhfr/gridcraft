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
    s
}
fn consent(s: &mut Session) {
    s.execute("macros.consent", json!({"allow":true})).unwrap();
}
fn run(s: &mut Session) -> serde_json::Value {
    s.execute("macros.run", json!({"name":"Main"})).unwrap()
}
fn value(s: &Session, address: &str) -> Value {
    s.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::parse(address).unwrap())
}
#[test]
fn fill_range_loop_cells_and_one_undo() {
    let mut s = session("Sub Main()\nDim i As Long\nRange(\"A1:B2\").Value = 5\nFor i = 1 To 10\nCells(i, 3) = i * 2\nNext i\nEnd Sub");
    s.history_clock = Some(|| 1234);
    let original = s.doc().unwrap().wb.clone();
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(value(&s, "A1"), Value::Number(5.0));
    assert_eq!(value(&s, "C10"), Value::Number(20.0));
    assert_eq!(s.doc().unwrap().undo.len(), 1);
    let history = s.execute("history.list", json!({})).unwrap();
    assert_eq!(history["undo"][0]["transactionId"], 1);
    assert_eq!(history["undo"][0]["timestamp"], 1234);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().wb, original);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(value(&s, "C10"), Value::Number(20.0));
}
#[test]
fn offset_end_sum_array_and_default_member() {
    let mut s = session(
        "Sub Main()\nDim a(1 To 2, 1 To 2) As Variant\nDim b As Variant\nDim r As Object\na(1,1)=1\na(1,2)=2\na(2,1)=3\na(2,2)=4\nRange(\"A1:B2\").Value2=a\nb=Range(\"A1:B2\").Value\nRange(\"D1:E2\").Value=b\nSet r=Range(\"A1\")\nr.Offset(2,0).Value=8\nRange(\"F1\").Value=Cells(100,1).End(xlUp).Row\nRange(\"G1\").Value=Application.WorksheetFunction.Sum(Range(\"A1:B2\"))\nRange(\"H1\")=Range(\"A1\")+2\nEnd Sub",
    );
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    for (cell, n) in [("E2", 4.0), ("A3", 8.0), ("F1", 3.0), ("G1", 10.0), ("H1", 3.0)] {
        assert_eq!(value(&s, cell), Value::Number(n), "{cell}");
    }
}
#[test]
fn headless_dialog_defaults_and_run_log() {
    let mut s = session(
        "Sub Main()\nRange(\"A1\").Value=MsgBox(\"Proceed?\",vbYesNo)\nRange(\"A2\").Value=InputBox(\"Name\",,\"default\")\nRange(\"A3\").Value=Application.GetOpenFilename()\nEnd Sub",
    );
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(value(&s, "A1"), Value::Number(7.0));
    assert_eq!(value(&s, "A2"), Value::text("default"));
    assert_eq!(value(&s, "A3"), Value::Bool(false));
    assert_eq!(s.take_ui_requests().len(), 3);
    let log = s.execute("macros.log", json!({})).unwrap();
    assert_eq!(log["runs"][0]["status"], "completed");
    assert!(log["runs"][0]["steps"].as_u64().unwrap() > 0);
}
#[test]
fn error_line_and_cancellation_discard_all_changes() {
    let mut s = session("Sub Main()\nRange(\"A1\").Value=42\nErr.Raise 5\nEnd Sub");
    consent(&mut s);
    let before = s.doc().unwrap().wb.clone();
    let result = run(&mut s);
    assert_eq!(result["status"], "failed", "{result}");
    assert_eq!(result["error"]["line"], 3);
    assert_eq!(result["rolledBack"], true);
    assert_eq!(s.doc().unwrap().wb, before);
    assert!(s.doc().unwrap().undo.is_empty());
    let mut s = session("Sub Main()\nDim i As Long\nFor i=1 To 10000\nCells(i,1).Value=i\nNext i\nEnd Sub");
    consent(&mut s);
    let before = s.doc().unwrap().wb.clone();
    let mut task = s.start_macro("Main", MacroOptions { instructions_per_frame: 2, ..Default::default() }).unwrap();
    assert!(s.step_macro(&mut task).unwrap().is_none());
    assert_eq!(s.doc().unwrap().wb, before);
    s.execute("macros.stop", json!({})).unwrap();
    let result = loop {
        if let Some(result) = s.step_macro(&mut task).unwrap() {
            break result;
        }
    };
    assert_eq!(result["status"], "cancelled");
    assert_eq!(s.doc().unwrap().wb, before);
    assert!(s.doc().unwrap().undo.is_empty());
}
#[test]
fn consent_is_session_scoped_and_no_auto_run() {
    let mut s = session("Sub Main()\nRange(\"A1\").Value=1\nEnd Sub\nSub Auto_Open()\nRange(\"A1\").Value=99\nEnd Sub");
    assert!(s.execute("macros.run", json!({"name":"Main"})).is_err());
    assert_eq!(value(&s, "A1"), Value::Empty);
    consent(&mut s);
    assert_eq!(run(&mut s)["status"], "completed");
    s.execute("macros.consent", json!({"allow":false})).unwrap();
    assert!(s.execute("macros.run", json!({"name":"Main"})).is_err());
    s.new_workbook();
    assert_eq!(s.execute("macros.consent", json!({})).unwrap()["consent"], false);
}
#[test]
fn level_three_call_and_step_limit_rollback() {
    let mut s =
        session("Sub Main()\nRange(\"A1\").Value=1\nBad\nEnd Sub\nSub Bad()\nDim o As Object\nSet o=CreateObject(\"Synthetic.Test\")\nEnd Sub");
    consent(&mut s);
    let original = s.doc().unwrap().wb.clone();
    let error = s.execute("macros.run", json!({"name":"Main"})).unwrap_err();
    assert!(error.to_string().contains("level"));
    let log = s.execute("macros.log", json!({})).unwrap();
    assert_eq!(log["runs"][0]["status"], "refused");
    assert_eq!(log["runs"][0]["error"]["level"], 3);
    assert_eq!(s.doc().unwrap().wb, original);
    assert!(s.execute("macros.run", json!({"name":"Bad"})).is_err());
    let mut s = session("Sub Main()\nRange(\"A1\").Value=1\nDo\nLoop\nEnd Sub");
    consent(&mut s);
    let result = s.execute("macros.run", json!({"name":"Main","maxSteps":100})).unwrap();
    assert_eq!(result["status"], "failed");
    assert_eq!(value(&s, "A1"), Value::Empty);
}

#[test]
fn workbook_sheet_formatting_union_and_manual_calculation() {
    let mut s = session(
        "Sub Main()\nDim ws As Object\nSet ws=ThisWorkbook.Worksheets(1)\nws.Name=\"Numbers\"\nApplication.Calculation=xlCalculationManual\nws.Range(\"A1\").Value=5\nws.Range(\"B1\").FormulaR1C1=\"=RC[-1]*2\"\nws.Calculate\nws.Range(\"A1:B1\").Font.Bold=True\nws.Range(\"A1\").Interior.Color=255\nws.Range(\"A1\").NumberFormat=\"0.00\"\nUnion(ws.Range(\"D1\"),ws.Range(\"F1\")).Value=8\nws.Range(\"A1:B1\").Copy ws.Range(\"A3\")\nws.Range(\"H1:I1\").Merge\nws.Range(\"H1:I1\").UnMerge\nEnd Sub",
    );
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().name, "Numbers");
    assert_eq!(value(&s, "B1"), Value::Number(10.0));
    assert_eq!(value(&s, "D1"), Value::Number(8.0));
    assert_eq!(value(&s, "E1"), Value::Empty);
    assert_eq!(value(&s, "F1"), Value::Number(8.0));
    assert_eq!(value(&s, "B3"), Value::Number(10.0));
    assert_eq!(s.doc().unwrap().undo.len(), 1);
    let doc = s.doc().unwrap();
    let style = doc.wb.styles.get(doc.wb.sheet(0).unwrap().style_id(CellRef::parse("A1").unwrap()));
    assert!(style.font.bold);
    assert_eq!(style.num_fmt.as_str(), "0.00");
    assert_eq!(style.fill.fg.resolve(&doc.wb.theme), Some([255, 0, 0]));
}
#[test]
fn outstanding_macro_never_overwrites_intervening_edits() {
    let mut s = session("Sub Main()\nDim i As Long\nFor i=1 To 50\nCells(i,1).Value=i\nNext i\nEnd Sub");
    consent(&mut s);
    let mut task = s.start_macro("Main", MacroOptions { instructions_per_frame: 1, ..Default::default() }).unwrap();
    assert!(s.step_macro(&mut task).unwrap().is_none());
    s.execute("cell.set", json!({"cell":"Z1","input":"User edit"})).unwrap();
    let result = loop {
        if let Some(result) = s.step_macro(&mut task).unwrap() {
            break result;
        }
    };
    assert_eq!(result["status"], "failed");
    assert_eq!(value(&s, "Z1"), Value::text("User edit"));
    assert_eq!(value(&s, "A1"), Value::Empty);
    assert_eq!(s.doc().unwrap().undo.len(), 1);
}

#[test]
fn dormant_platform_declarations_do_not_block_safe_macros() {
    let mut s = session(
        "Private Declare PtrSafe Function ExternalThing Lib \"synthetic.dll\" () As Long\nSub Main()\nRange(\"A1\").Value=42\nEnd Sub\nSub Platform()\nExternalThing\nEnd Sub",
    );
    consent(&mut s);
    assert_eq!(run(&mut s)["status"], "completed");
    assert_eq!(value(&s, "A1"), Value::Number(42.0));
    assert!(s.execute("macros.run", json!({"name":"Platform"})).is_err());
}
#[test]
fn bulk_assignments_suspend_and_cancel_without_publishing() {
    let mut s = session("Sub Main()\nRange(\"A1:A5000\").Value=42\nEnd Sub");
    consent(&mut s);
    let before = s.doc().unwrap().wb.clone();
    let mut task = s.start_macro("Main", MacroOptions::default()).unwrap();
    assert!(s.step_macro(&mut task).unwrap().is_none());
    assert!(s.step_macro(&mut task).unwrap().is_none());
    s.execute("macros.stop", json!({})).unwrap();
    let result = loop {
        if let Some(result) = s.step_macro(&mut task).unwrap() {
            break result;
        }
    };
    assert_eq!(result["status"], "cancelled");
    assert_eq!(s.doc().unwrap().wb, before);
}

#[test]
fn unqualified_collections_letter_columns_and_format_reads() {
    let mut s = session(
        "Sub Main()\nCells(1,\"B\").Value=9\nRange(\"A1\").Value=Worksheets.Count\nRange(\"A2\").Value=Rows.Count\nRange(\"A3\").Value=Columns.Count\nRange(\"A4\").Value=Range(\"B1:C2\").Columns(\"B\").Column\nRange(\"B1\").Interior.ColorIndex=56\nRange(\"C1\").Value=Range(\"B1\").Interior.ColorIndex\nRange(\"D1\").FormulaLocal=\"=SUM(1;2)\"\nEnd Sub",
    );
    s.region = gridcraft_engine::core::Region::ViVn;
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    for (cell, n) in [("A1", 1.0), ("A2", 1048576.0), ("A3", 16384.0), ("A4", 3.0), ("B1", 9.0), ("C1", 56.0), ("D1", 3.0)] {
        assert_eq!(value(&s, cell), Value::Number(n), "{cell}");
    }
}

#[test]
fn default_range_values_work_in_conditions_and_loop_bounds() {
    let mut s = session(
        "Sub Main()\nDim i As Long\nRange(\"A1\").Value=3\nIf Range(\"A1\") Then\nFor i=1 To Range(\"A1\")\nCells(i,\"B\")=i\nNext i\nEnd If\nRange(\"C1\").Select\nActiveCell=12\nEnd Sub",
    );
    consent(&mut s);
    assert_eq!(run(&mut s)["status"], "completed");
    assert_eq!(value(&s, "B3"), Value::Number(3.0));
    assert_eq!(value(&s, "C1"), Value::Number(12.0));
}

#[test]
fn paste_special_uses_existing_clipboard_values_and_one_transaction() {
    let mut s = session("Sub Main()\nRange(\"D1\").PasteSpecial xlPasteValues\nEnd Sub");
    s.execute("cell.set", json!({"cell":"A1","input":"=1+2"})).unwrap();
    s.execute("edit.copy", json!({"range":"A1"})).unwrap();
    s.doc_mut().unwrap().undo.clear();
    consent(&mut s);
    assert_eq!(run(&mut s)["status"], "completed");
    assert_eq!(value(&s, "D1"), Value::Number(3.0));
    assert!(s.doc().unwrap().wb.sheet(0).unwrap().cell(CellRef::parse("D1").unwrap()).unwrap().formula.is_none());
    assert_eq!(s.doc().unwrap().undo.len(), 1);
}

#[test]
fn r1c1_handles_quoted_sheet_names_and_whole_lines() {
    let mut s = session(
        "Sub Main()\nActiveSheet.Name=\"R1C1 data\"\nRange(\"A1\").Value=5\nRange(\"B1\").FormulaR1C1=\"='R1C1 data'!RC[-1]\"\nRange(\"C1\").FormulaR1C1=\"=SUM(C[-2])\"\nRange(\"D2\").FormulaR1C1=\"=SUM(R[-1])\"\nEnd Sub",
    );
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(value(&s, "B1"), Value::Number(5.0));
    assert_eq!(value(&s, "C1"), Value::Number(5.0));
    assert_eq!(value(&s, "D2"), Value::Number(15.0));
}

#[test]
fn find_searches_sparse_whole_sheet_with_named_arguments() {
    let mut s = session(
        "Sub Main()\nDim found As Object\nRange(\"A1\").Value=\"alpha\"\nRange(\"B2\").Value=\"beta\"\nSet found=Cells.Find(What:=\"BETA\",LookAt:=xlWhole)\nRange(\"C1\").Value=found.Row\nEnd Sub",
    );
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(value(&s, "C1"), Value::Number(2.0));
}

#[test]
fn deadline_includes_time_between_ui_frames() {
    let mut s = session("Sub Main()\nRange(\"A1\").Value=42\nEnd Sub");
    consent(&mut s);
    let original = s.doc().unwrap().wb.clone();
    let mut task = s.start_macro("Main", MacroOptions { wall_ms: 1, ..Default::default() }).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let result = s.step_macro(&mut task).unwrap().unwrap();
    assert_eq!(result["status"], "failed");
    assert!(result["error"]["message"].as_str().unwrap().contains("deadline"));
    assert_eq!(s.doc().unwrap().wb, original);
}

#[test]
fn worksheet_function_aliases_named_and_missing_arguments() {
    let mut s = session(
        "Sub Main()\nRange(\"A1\").Value=Application.WorksheetFunction.Norm_S_Dist(Arg2:=True,Arg1:=0)\nRange(\"A2\").Value=Application.WorksheetFunction.Count(,3)\nRange(\"D1\").Value=1\nRange(\"E1\").Value=5\nRange(\"D2\").Value=3\nRange(\"E2\").Value=9\nRange(\"A3\").Value=Application.WorksheetFunction.VLookup(2,Range(\"D1:E2\"),2,)\nEnd Sub",
    );
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    for (cell, n) in [("A1", 0.5), ("A2", 1.0), ("A3", 5.0)] {
        assert_eq!(value(&s, cell), Value::Number(n), "{cell}");
    }
}

#[test]
fn intersections_keep_matrix_shape_and_union_selection_is_disjoint() {
    let mut s = session(
        "Sub Main()\nDim r As Object\nDim a As Variant\nSet r=Intersect(Range(\"A1:C3\"),Range(\"B2:D4\"))\nr.Value=9\na=r.Value\nRange(\"F1:G2\").Value=a\nRange(\"E1\").Value=r.Rows.Count\nRange(\"E2\").Value=r.Columns.Count\nUnion(Range(\"A1\"),Range(\"E5\")).Select\nEnd Sub",
    );
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    for (cell, n) in [("B2", 9.0), ("C3", 9.0), ("G2", 9.0), ("E1", 2.0), ("E2", 2.0)] {
        assert_eq!(value(&s, cell), Value::Number(n), "{cell}");
    }
    assert_eq!(s.doc().unwrap().selection.ranges.len(), 2);
}

#[test]
fn sheet_visibility_preserves_very_hidden_and_is_undoable() {
    let mut s = session("Sub Main()\nWorksheets(\"Extra\").Visible=xlSheetVeryHidden\nRange(\"A1\").Value=Worksheets(\"Extra\").Visible\nEnd Sub");
    s.execute("home.insertSheet", json!({"name":"Extra"})).unwrap();
    s.execute("sheet.hide", json!({"sheet":"Extra"})).unwrap();
    s.doc_mut().unwrap().undo.clear();
    consent(&mut s);
    assert_eq!(run(&mut s)["status"], "completed");
    assert_eq!(value(&s, "A1"), Value::Number(2.0));
    assert_eq!(
        s.doc().unwrap().wb.sheets.iter().find(|sheet| sheet.name == "Extra").unwrap().visibility,
        gridcraft_engine::model::Visibility::VeryHidden
    );
    assert_eq!(s.doc().unwrap().undo.len(), 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(
        s.doc().unwrap().wb.sheets.iter().find(|sheet| sheet.name == "Extra").unwrap().visibility,
        gridcraft_engine::model::Visibility::Hidden
    );
}

#[test]
fn runtime_level_three_setter_refusal_bypasses_resume_next_and_rolls_back() {
    let mut s = session(
        "Public Property Let Bad(ByVal v As Long)\nDim o As Object\nSet o=CreateObject(\"Synthetic.Test\")\nEnd Property\nPublic Property Get Bad() As Long\nBad=0\nEnd Property\nPublic Sub Main()\nOn Error Resume Next\nRange(\"A1\").Value=42\nRange(\"A2\").Value=Bad\nBad=1\nEnd Sub",
    );
    let analysis = s.execute("macros.analyze", json!({})).unwrap();
    let main = analysis["procedures"].as_array().unwrap().iter().find(|p| p["name"] == "Main").unwrap();
    assert_eq!(main["compatibility"]["level"], 1);
    let original = s.doc().unwrap().wb.clone();
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "failed", "{result}");
    assert_eq!(result["attemptedCellsChanged"], 2);
    assert_eq!(result["error"]["module"], "Module1");
    assert_eq!(result["error"]["line"], 3);
    assert!(result["error"]["message"].as_str().unwrap().contains("level 3"));
    assert_eq!(s.doc().unwrap().wb, original);
    assert!(s.doc().unwrap().undo.is_empty());
}

#[test]
fn scalar_cell_writes_share_the_interpreter_frame_budget() {
    let mut s = session("Sub Main()\nDim i As Long\nFor i=1 To 100\nCells(i,1).Value=i\nNext i\nEnd Sub");
    consent(&mut s);
    let mut task = s.start_macro("Main", MacroOptions { instructions_per_frame: 64, ..Default::default() }).unwrap();
    let mut frames = 0;
    let result = loop {
        frames += 1;
        if let Some(result) = s.step_macro(&mut task).unwrap() {
            break result;
        }
        assert!(frames < 10);
    };
    assert_eq!(result["status"], "completed");
    assert_eq!(value(&s, "A100"), Value::Number(100.0));
}

#[test]
fn optional_parameters_are_runnable_and_required_parameters_are_refused() {
    let mut s = session(
        "Public Sub Main(Optional ByVal n As Long = 7)\nRange(\"A1\").Value=n\nEnd Sub\nPublic Sub NeedsArgument(ByVal n As Long)\nRange(\"A1\").Value=n\nEnd Sub",
    );
    let analysis = s.execute("macros.analyze", json!({})).unwrap();
    let procedures = analysis["procedures"].as_array().unwrap();
    assert_eq!(procedures.iter().find(|p| p["name"] == "Main").unwrap()["runnableShape"], true);
    assert_eq!(procedures.iter().find(|p| p["name"] == "NeedsArgument").unwrap()["runnableShape"], false);
    consent(&mut s);
    assert_eq!(run(&mut s)["status"], "completed");
    assert_eq!(value(&s, "A1"), Value::Number(7.0));
    let original = s.doc().unwrap().wb.clone();
    assert!(s.execute("macros.run", json!({"name":"NeedsArgument"})).is_err());
    assert_eq!(s.doc().unwrap().wb, original);
    assert_eq!(s.doc().unwrap().undo.len(), 1);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interactive_prompts_suspend_then_use_responses_atomically() {
    let mut s =
        session("Sub Main()\nRange(\"A1\").Value=InputBox(\"Value\",\"Title\",\"seed\")\nRange(\"A2\").Value=MsgBox(\"Continue?\",vbYesNo)\nEnd Sub");
    consent(&mut s);
    let mut task = s.start_macro_interactive("Main", MacroOptions::default()).unwrap();
    assert!(s.step_macro(&mut task).unwrap().is_none());
    assert_eq!(task.take_prompt().unwrap().0, "macro.inputBox");
    assert_eq!(value(&s, "A1"), Value::Empty);
    task.answer_prompt(json!("chosen"));
    for _ in 0..10 {
        assert!(s.step_macro(&mut task).unwrap().is_none());
        if let Some((kind, _)) = task.take_prompt() {
            assert_eq!(kind, "macro.msgBox");
            break;
        }
    }
    task.answer_prompt(json!(6));
    let result = loop {
        if let Some(result) = s.step_macro(&mut task).unwrap() {
            break result;
        }
    };
    assert_eq!(result["status"], "completed");
    assert_eq!(value(&s, "A1"), Value::Text("chosen".into()));
    assert_eq!(value(&s, "A2"), Value::Number(6.0));
    assert_eq!(s.doc().unwrap().undo.len(), 1);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cancelling_an_interactive_prompt_rolls_back() {
    let mut s = session("Sub Main()\nRange(\"A1\").Value=9\nMsgBox \"Wait\"\nEnd Sub");
    consent(&mut s);
    let mut task = s.start_macro_interactive("Main", MacroOptions::default()).unwrap();
    for _ in 0..10 {
        assert!(s.step_macro(&mut task).unwrap().is_none());
        if task.take_prompt().is_some() {
            break;
        }
    }
    task.cancel();
    let result = loop {
        if let Some(result) = s.step_macro(&mut task).unwrap() {
            break result;
        }
    };
    assert_eq!(result["status"], "cancelled");
    assert_eq!(value(&s, "A1"), Value::Empty);
    assert!(s.doc().unwrap().undo.is_empty());
}

#[test]
fn refused_feature_location_matches_its_reason_line() {
    let mut s = session("Sub Main()\nDim o As Object\nSet o=CreateObject(\"Synthetic.Test\")\nEnd Sub");
    let before = s.doc().unwrap().wb.clone();
    assert!(s.execute("macros.run", json!({"name":"Main"})).is_err());
    let log = s.execute("macros.log", json!({})).unwrap();
    assert_eq!(log["runs"][0]["error"]["line"], 3);
    assert_eq!(log["runs"][0]["error"]["reasons"][0]["line"], 3);
    assert_eq!(s.doc().unwrap().wb, before);
}

#[test]
fn macro_bracket_evaluation_ranges_names_and_values() {
    let mut s = session(
        "Sub Main()\n[A1:B2] = 3\n[A1].Value = 5\n[B3] = [A1] * 2\n[C1] = [SUM(A1:B2)]\nSet r = [Sheet1!B3]\nr.Value = 12\n[NamedBlock].Value = 7\n[C2] = Application.Evaluate(\"2+4\")\nEnd Sub",
    );
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).names.push(gridcraft_model::DefinedName {
        name: "NamedBlock".into(),
        scope: None,
        formula: "Sheet1!$D$1:$D$2".into(),
        comment: String::new(),
        hidden: false,
    });
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    for (cell, expected) in [("A1", 5.0), ("B2", 3.0), ("B3", 12.0), ("C1", 14.0), ("C2", 6.0), ("D2", 7.0)] {
        assert_eq!(value(&s, cell), Value::Number(expected));
    }
}

#[test]
fn macro_compile_failure_isolated_and_reported_on_call() {
    let mut s = session("Sub Main()\n[A1] = 8\nEnd Sub\nSub Caller()\nBroken.Bad\nEnd Sub");
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).vba_project = Some(
        fixture(
            &[
                FixtureModule::new("Module1", "Module", "Sub Main()\n[A1] = 8\nEnd Sub\nSub Caller()\nBroken.Bad\nEnd Sub").unwrap(),
                FixtureModule::new("Broken", "Module", "Sub Bad()\nDim =\nEnd Sub").unwrap(),
            ],
            1252,
            "",
            &[],
        )
        .unwrap(),
    );
    let analysis = s.execute("macros.analyze", json!({})).unwrap();
    let broken = analysis["procedures"].as_array().unwrap().iter().find(|p| p["module"] == "Broken").unwrap();
    assert_eq!(broken["runnableShape"], false);
    assert_eq!(broken["compileError"]["module"], "Broken");
    assert_eq!(broken["compileError"]["line"], 2);
    consent(&mut s);
    assert_eq!(run(&mut s)["status"], "completed");
    assert_eq!(value(&s, "A1"), Value::Number(8.0));
    let result = s.execute("macros.run", json!({"name":"Caller"})).unwrap();
    assert_eq!(result["status"], "failed", "{result}");
    assert_eq!(result["error"]["module"], "Broken");
    assert_eq!(result["error"]["line"], 2);
}

#[test]
fn macro_worksheet_qualified_bracket_evaluation_and_with() {
    let mut s = session(
        "Sub Main()\nWorksheets(1).[A1:B2].Value = 4\nWith Worksheets(1)\n.[C1] = .[A1] + 2\nEnd With\n[D1] = Worksheets(1).Evaluate(\"C1\").Value\nEnd Sub",
    );
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(value(&s, "B2"), Value::Number(4.0));
    assert_eq!(value(&s, "D1"), Value::Number(6.0));
}

#[test]
fn macro_document_code_name_resolves_sheet_members_and_vba_procedures() {
    let mut s = session("Sub Main()\nDataCode.Range(\"A1\").Value = 4\nDataCode.[B1] = DataCode.Range(\"A1\").Value + DataCode.Bonus()\nEnd Sub");
    let wb = Arc::make_mut(&mut s.doc_mut().unwrap().wb);
    Arc::make_mut(&mut wb.sheets[0]).vba_code_name = "DataCode".into();
    wb.vba_project = Some(
        fixture(
            &[
                FixtureModule::new(
                    "Module1",
                    "Module",
                    "Sub Main()\nDataCode.Range(\"A1\").Value = 4\nDataCode.[B1] = DataCode.Range(\"A1\").Value + DataCode.Bonus()\nEnd Sub",
                )
                .unwrap(),
                FixtureModule::new("DataCode", "Document", "Public Function Bonus()\nBonus = 2\nEnd Function").unwrap(),
            ],
            1252,
            "",
            &[],
        )
        .unwrap(),
    );
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(value(&s, "B1"), Value::Number(6.0));
}

#[test]
fn macro_sparse_clear_contents_does_not_expand_blank_ranges() {
    let mut s = session("Sub Main()\nRange(\"A:A\").ClearContents\nEnd Sub");
    s.execute("cell.set", json!({"cell":"A1","input":"5"})).unwrap();
    s.execute("cell.set", json!({"cell":"A900000","input":"7"})).unwrap();
    s.execute("cell.set", json!({"cell":"B1","input":"9"})).unwrap();
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["cellsChanged"], 2);
    assert_eq!(value(&s, "A1"), Value::Empty);
    assert_eq!(value(&s, "A900000"), Value::Empty);
    assert_eq!(value(&s, "B1"), Value::Number(9.0));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(value(&s, "A900000"), Value::Number(7.0));
}

#[test]
fn macro_bracket_name_cycles_are_bounded() {
    let mut s = session("Sub Main()\nx = [CycleName]\nEnd Sub");
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).names.push(gridcraft_model::DefinedName {
        name: "CycleName".into(),
        scope: None,
        formula: "CycleName".into(),
        comment: String::new(),
        hidden: false,
    });
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "failed", "{result}");
    assert_eq!(result["error"]["message"], "VBA sandbox limit: Evaluate name nesting");
}

#[test]
fn macro_range_comparison_to_empty_string_stops_at_blank() {
    let mut s = session("Sub Main()\n[A1] = 4\n[A2] = 6\ni = 1\nDo While Cells(i, 1) <> \"\"\ni = i + 1\nLoop\n[B1] = i - 1\nEnd Sub");
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(value(&s, "B1"), Value::Number(2.0));
}

#[test]
fn macro_autofill_uses_engine_series_formula_translation_and_undo() {
    let mut s = session(
        "Sub Main()\n[A1] = 2\n[A2] = 5\n[A1:A2].AutoFill Destination:=[A1:A5]\n[B1].Formula = \"=A1*2\"\n[B1].AutoFill Destination:=[B1:B5], Type:=xlFillDefault\n[C1] = 9\n[C1].AutoFill Destination:=[C1:C3], Type:=xlFillCopy\nEnd Sub",
    );
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["cellsChanged"], 13);
    assert_eq!(value(&s, "A5"), Value::Number(14.0));
    assert_eq!(value(&s, "B5"), Value::Number(28.0));
    assert_eq!(value(&s, "C3"), Value::Number(9.0));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(value(&s, "A1"), Value::Empty);
    assert_eq!(value(&s, "B5"), Value::Empty);
}

#[test]
fn macro_autofill_rejects_invalid_destination_and_unsupported_type() {
    for statement in ["[A1].AutoFill Destination:=[B1:B4]", "[A1].AutoFill Destination:=[A1:A4], Type:=xlFlashFill"] {
        let mut s = session(&format!("Sub Main()\n[A1] = 2\n{statement}\nEnd Sub"));
        consent(&mut s);
        let result = run(&mut s);
        assert_eq!(result["status"], "failed", "{result}");
        assert_eq!(value(&s, "A1"), Value::Empty);
    }
}

#[test]
fn macro_range_resolves_defined_names_in_scalar_context() {
    let mut s = session("Sub Main()\nRange(\"RecordCount\").Value = 3\nFor i = 1 To Range(\"RecordCount\")\nCells(i, 2) = i\nNext\nEnd Sub");
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).names.push(gridcraft_model::DefinedName {
        name: "RecordCount".into(),
        scope: None,
        formula: "Sheet1!$A$1".into(),
        comment: String::new(),
        hidden: false,
    });
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(value(&s, "B3"), Value::Number(3.0));
}

#[test]
fn macro_page_setup_print_area_roundtrip_clear_and_undo() {
    let mut s = session(
        "Sub Main()\nWith Worksheets(1).PageSetup\n.PrintArea = \"A1:D8\"\n[A1] = .PrintArea\n.PrintArea = \"\"\n[B1] = .PrintArea\n.PrintArea = \"B2:C5\"\nEnd With\nEnd Sub",
    );
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(value(&s, "A1"), Value::from("$A$1:$D$8"));
    assert_eq!(value(&s, "B1"), Value::from(""));
    assert_eq!(s.doc().unwrap().wb.sheets[0].print.print_area, gridcraft_core::RangeRef::parse("B2:C5"));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheets[0].print.print_area, None);
}

#[test]
fn macro_excel_objects_expose_application_for_worksheet_functions() {
    let mut s = session(
        "Sub Main()\n[A1] = 4\n[A2] = 8\n[B1] = Worksheets(1).Application.WorksheetFunction.Max([A1:A2])\n[C1] = [A1].Application.WorksheetFunction.Sum([A1:A2])\nEnd Sub",
    );
    consent(&mut s);
    let result = run(&mut s);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(value(&s, "B1"), Value::Number(8.0));
    assert_eq!(value(&s, "C1"), Value::Number(12.0));
}
