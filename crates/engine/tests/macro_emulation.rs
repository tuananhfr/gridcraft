use gridcraft_engine::{
    Session,
    core::{CellRef, Value},
    macro_host::{MacroOptions, MacroTask},
};
use gridcraft_vba::testing::{FixtureModule, fixture};
use serde_json::{Value as Json, json};
use std::sync::Arc;
#[path = "../../mdb/tests/support/mod.rs"]
mod mdb_fixture;

fn session(body: &str) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    let source = format!("Sub Main()\n{body}\nEnd Sub");
    Arc::make_mut(&mut s.doc_mut().unwrap().wb).vba_project =
        Some(fixture(&[FixtureModule::new("Module1", "Module", &source).unwrap()], 1252, "", &[]).unwrap());
    s.execute("macros.consent", json!({"allow":true})).unwrap();
    s
}
fn value(s: &Session, at: &str) -> Value {
    s.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::parse(at).unwrap())
}
fn start(s: &mut Session) -> MacroTask {
    s.start_macro_interactive("Main", MacroOptions { wall_ms: 300_000, ..Default::default() }).unwrap()
}
fn finish(s: &mut Session, task: &mut MacroTask, bytes: Option<&[u8]>) -> (Json, usize) {
    let mut picks = 0;
    for _ in 0..10000 {
        if let Some(result) = s.step_macro(task).unwrap() {
            return (result, picks);
        }
        if let Some((name, params)) = task.take_prompt() {
            assert_eq!(name, "macro.openMdb", "{params}");
            assert_eq!(params["prompt"], "C:\\synthetic\\etabs.mdb");
            picks += 1;
            task.answer_prompt(bytes.map(|b| json!({"base64":gridcraft_engine::io::base64_encode(b)})).unwrap_or(Json::Null));
        }
    }
    panic!("macro did not finish")
}
#[test]
fn macro_dao_jet4_query_navigation_bang_fields_and_cache() {
    let mut s = session(
        r#"
Dim engine As Object, db As DAO.Database, again As Object, rs As DAO.Recordset
Set engine = CreateObject("DAO.DBEngine.120")
Set db = engine.Workspaces(0).OpenDatabase("C:\synthetic\etabs.mdb")
Set again = OpenDatabase("C:\synthetic\etabs.mdb")
Set rs = db.OpenRecordset("SELECT Joint, Fz, Memo FROM [Support Reactions] WHERE Fz < 0 AND Joint = 'J1' OR Fz > 999 ORDER BY Fz DESC, Joint ASC")
Range("A1") = rs!Joint
Range("B1") = rs.Fields("Fz").Value
Range("C1") = rs("Memo")
Range("A2") = rs.RecordCount
Range("B2") = rs.Fields.Count
Range("C2") = rs.Fields(1).Name
rs.MoveLast
rs.MoveNext
Range("A3") = rs.EOF
rs.MovePrevious
Range("B3") = rs.EOF
rs.MoveFirst
rs.MovePrevious
Range("C3") = rs.BOF
rs.MoveFirst
Range("D3") = rs.BOF
Range("A4") = db.TableDefs(0).Name
Range("E1") = rs.RecordCount
Range("F1").CopyFromRecordset rs
Range("E2") = rs.EOF
rs.Close
again.Close
db.Close
"#,
    );
    let mut task = start(&mut s);
    let (result, picks) = finish(&mut s, &mut task, Some(&mdb_fixture::etabs(true)));
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(picks, 1);
    for (at, expected) in [
        ("A1", Value::text("J1")),
        ("B1", Value::Number(-123.5)),
        ("C1", Value::text("Phản lực")),
        ("A2", Value::Number(3.0)),
        ("B2", Value::Number(3.0)),
        ("C2", Value::text("Fz")),
        ("A3", Value::Bool(true)),
        ("B3", Value::Bool(false)),
        ("C3", Value::Bool(true)),
        ("D3", Value::Bool(false)),
    ] {
        assert_eq!(value(&s, at), expected, "{at}");
    }
    assert_eq!(value(&s, "F1"), Value::text("J1"));
    assert_eq!(value(&s, "G3"), Value::Number(-123.5));
    assert_eq!(value(&s, "E2"), Value::Bool(true));
    assert_eq!(s.doc().unwrap().undo.len(), 1);
}
#[test]
fn macro_dao_cancel_is_fatal_and_rolls_back() {
    let mut s = session("On Error Resume Next\nRange(\"A1\")=42\nSet db = DAO.OpenDatabase(\"C:\\synthetic\\etabs.mdb\")\nRange(\"A2\")=99");
    let original = s.doc().unwrap().wb.clone();
    let mut task = start(&mut s);
    let (result, picks) = finish(&mut s, &mut task, None);
    assert_eq!(picks, 1);
    assert_eq!(result["rolledBack"], true, "{result}");
    assert!(result["error"]["message"].as_str().unwrap().contains("cancelled"));
    assert_eq!(s.doc().unwrap().wb, original);
    assert!(s.doc().unwrap().undo.is_empty());
}
#[test]
fn macro_dao_writes_refused_and_closed_handles_fail() {
    for statement in [
        "rs.Edit",
        "rs.AddNew",
        "rs.Update",
        "db.Execute \"DELETE FROM [Support Reactions]\"",
        "rs.Fields(0).Value = 3",
        "db.Close\nx = rs.RecordCount",
    ] {
        let mut s = session(&format!(
            "Set db = OpenDatabase(\"C:\\synthetic\\etabs.mdb\")\nSet rs = db.OpenRecordset(\"Support Reactions\")\nRange(\"A1\")=1\n{statement}"
        ));
        let original = s.doc().unwrap().wb.clone();
        let mut task = start(&mut s);
        let (result, _) = finish(&mut s, &mut task, Some(&mdb_fixture::etabs(true)));
        assert_eq!(result["rolledBack"], true, "{statement}: {result}");
        assert_eq!(s.doc().unwrap().wb, original);
        let error = result["error"]["message"].as_str().unwrap();
        assert!(error.contains("write") || error.contains("closed"), "{error}");
    }
}
#[test]
fn macro_autocad_dxf_entities_layers_arrays_and_noop_log() {
    let mut s = session(
        r#"
Dim acad As Object, doc As Object, layer As Object, ent As Object
Set acad = GetObject(, "AutoCAD.Application")
acad.Visible = True
Set doc = acad.ActiveDocument
Set layer = doc.Layers.Add("Synthetic")
layer.Color = acGreen
Set layer = doc.Layers.Add("synthetic")
Range("J1") = layer.Color
Set doc.ActiveLayer = layer
Set ent = doc.ModelSpace.AddLine(Array(1,2,3), Array(4,5,6))
ent.Color = acRed
ent.Update
Set ent = doc.ModelSpace.AddLightWeightPolyline(Array(0,0,1,2,3,4))
Set ent = doc.ModelSpace.AddPolyline(Array(0,0,1,2,3,4))
Set ent = doc.ModelSpace.AddCircle(Array(0,0,0), 5)
Set ent = doc.ModelSpace.AddArc(Array(0,0,0), 2, 0, 1.5)
Set ent = doc.ModelSpace.AddText("Synthetic", Array(1,2,0), 3)
Set ent = doc.ModelSpace.AddMText(Array(1,2,0), 10, "Line one" & vbLf & "Line two")
Set ent = doc.ModelSpace.AddPoint(Array(7,8,9))
ent.Layer = "Synthetic"
acad.ZoomExtents
acad.ZoomAll
AppActivate acad.Caption
ChDir CurDir
ChDrive "Z"
"#,
    );
    let mut task = start(&mut s);
    let (result, _) = finish(&mut s, &mut task, None);
    assert_eq!(result["status"], "completed", "{result}");
    let drawing = String::from_utf8(task.take_drawing().unwrap()).unwrap();
    for entity in ["LINE", "LWPOLYLINE", "POLYLINE", "CIRCLE", "ARC", "TEXT", "MTEXT", "POINT"] {
        assert!(drawing.contains(&format!("0\r\n{entity}\r\n")), "{entity}");
    }
    assert!(drawing.contains("8\r\nSynthetic\r\n"));
    assert!(drawing.contains("62\r\n1\r\n"));
    assert_eq!(value(&s, "J1"), Value::Number(3.0));
    assert!(result["messages"].as_array().unwrap().iter().any(|v| v.as_str().unwrap().contains("ignored")));
}
#[test]
fn macro_autocad_unknown_member_named_and_drawing_discarded() {
    let mut s = session(
        "Set acad=CreateObject(\"AutoCAD.Application\")\nSet ent=acad.ActiveDocument.ModelSpace.AddPoint(Array(0,0,0))\nacad.UnsupportedSyntheticMember",
    );
    let mut task = start(&mut s);
    let (result, _) = finish(&mut s, &mut task, None);
    assert_eq!(result["rolledBack"], true, "{result}");
    assert!(result["error"]["message"].as_str().unwrap().to_ascii_lowercase().contains("unsupportedsyntheticmember"));
    assert!(task.take_drawing().is_none());
}

#[test]
fn macro_dao_empty_recordsets_and_navigation_boundaries() {
    let mut s = session(
        r#"
Set db = OpenDatabase("C:\synthetic\etabs.mdb")
Set rs = db.OpenRecordset("SELECT * FROM [Support Reactions] WHERE Fz > .5")
Range("A1") = rs.EOF
Range("B1") = rs.BOF
Range("C1") = rs.RecordCount
On Error Resume Next
rs.MoveFirst
Range("D1") = Err.Number
Err.Clear
On Error GoTo 0
Set rs = db.OpenRecordset("Support Reactions")
rs.MoveLast
rs.MoveNext
On Error Resume Next
rs.MoveNext
Range("D2") = Err.Number
"#,
    );
    let mut task = start(&mut s);
    let (result, _) = finish(&mut s, &mut task, Some(&mdb_fixture::etabs(true)));
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(value(&s, "A1"), Value::Bool(true));
    assert_eq!(value(&s, "B1"), Value::Bool(true));
    assert_eq!(value(&s, "C1"), Value::Number(0.0));
    assert_eq!(value(&s, "D1"), Value::Number(3021.0));
    assert_eq!(value(&s, "D2"), Value::Number(3021.0));
}

#[test]
fn macro_dao_picker_wait_does_not_consume_execution_deadline() {
    let mut s = session("Set db=OpenDatabase(\"C:\\synthetic\\etabs.mdb\")\nRange(\"A1\")=1");
    let mut task = s.start_macro_interactive("Main", MacroOptions { wall_ms: 100, ..Default::default() }).unwrap();
    loop {
        assert!(s.step_macro(&mut task).unwrap().is_none());
        if let Some((name, _)) = task.take_prompt() {
            assert_eq!(name, "macro.openMdb");
            break;
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    task.answer_prompt(json!({"base64":gridcraft_engine::io::base64_encode(&mdb_fixture::etabs(true))}));
    let (result, _) = finish(&mut s, &mut task, None);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(value(&s, "A1"), Value::Number(1.0));
}
