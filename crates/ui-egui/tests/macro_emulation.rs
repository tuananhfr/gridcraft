#![cfg(not(target_arch = "wasm32"))]
use egui_kittest::kittest::Queryable;
use gridcraft_engine::{
    Session,
    core::{CellRef, Value},
};
use gridcraft_ui_egui::{SheetApp, i18n::Language};
use gridcraft_vba::testing::{FixtureModule, fixture};
use serde_json::json;
use std::sync::{Arc, Mutex};
#[path = "../../mdb/tests/support/mod.rs"]
mod mdb_fixture;

fn harness(body: &str, width: f32, render: bool) -> egui_kittest::Harness<'static, SheetApp> {
    let mut session = Session::new();
    session.new_workbook();
    let source = format!("Sub Main()\n{body}\nEnd Sub");
    Arc::make_mut(&mut session.doc_mut().unwrap().wb).vba_project =
        Some(fixture(&[FixtureModule::new("Module1", "Module", &source).unwrap()], 1252, "", &[]).unwrap());
    session.execute("macros.consent", json!({"allow":true})).unwrap();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.language = Language::Vi;
    app.open_dialog("macros", json!({}));
    let b = egui_kittest::Harness::builder().with_size(egui::vec2(width, 900.0));
    let b = if render { b.wgpu() } else { b };
    let mut h = b.build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(6);
    h.get_by_label("Chạy").click();
    h.run_steps(6);
    h
}
fn value(app: &SheetApp) -> Value {
    app.session.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::new(0, 0))
}
#[test]
fn macro_emulation_mdb_picker_resumes_only_its_own_run() {
    let mut h =
        harness("Set db=OpenDatabase(\"synthetic.mdb\")\nSet rs=db.OpenRecordset(\"Support Reactions\")\nRange(\"A1\")=rs!Joint", 375.0, false);
    assert!(h.query_by_label("Chọn tệp .mdb mà macro cần mở: synthetic.mdb").is_some());
    let picked = Arc::new(Mutex::new(false));
    let report = picked.clone();
    h.state_mut().services.pick_macro_mdb = Some(Box::new(move |title, reply| {
        assert!(title.contains("synthetic.mdb"));
        *report.lock().unwrap() = true;
        *reply.lock().unwrap() = Some(Ok(Some(mdb_fixture::etabs(true))));
    }));
    assert!(!*picked.lock().unwrap());
    h.get_by_label("Chọn tệp…").click();
    h.run_steps(20);
    assert!(*picked.lock().unwrap());
    assert_eq!(value(h.state()), Value::text("J1"));
    assert_eq!(h.state().session.documents().len(), 1);
}
#[test]
fn macro_emulation_picker_cancel_rolls_back_and_dxf_is_delivered_after_success() {
    let mut h = harness("Range(\"A1\")=99\nSet db=OpenDatabase(\"synthetic.mdb\")", 375.0, false);
    h.get_by_label("Hủy").click();
    h.run_steps(10);
    assert_eq!(value(h.state()), Value::Empty);
    assert!(h.query_by_label("Đã hủy chọn tệp DAO; macro đã dừng").is_some());
    let mut h =
        harness("Set acad=CreateObject(\"AutoCAD.Application\")\nSet ent=acad.ActiveDocument.ModelSpace.AddCircle(Array(0,0,0),2)", 1440.0, false);
    assert!(h.query_by_label("Macro đã tạo bản vẽ — Lưu DXF").is_some());
    let out = Arc::new(Mutex::new(Vec::new()));
    let report = out.clone();
    h.state_mut().services.save_macro_dxf = Some(Box::new(move |title, bytes| {
        assert_eq!(title, "Macro đã tạo bản vẽ — Lưu DXF");
        *report.lock().unwrap() = bytes.to_vec();
        Ok(())
    }));
    assert!(out.lock().unwrap().is_empty());
    h.get_by_label("Lưu").click();
    h.run_steps(4);
    assert!(String::from_utf8(out.lock().unwrap().clone()).unwrap().contains("0\r\nCIRCLE\r\n"));
}
#[test]
fn macro_emulation_screenshots_when_requested() {
    let Some(dir) = std::env::var_os("GRIDCRAFT_MACRO_SCREENSHOTS") else { return };
    std::fs::create_dir_all(&dir).unwrap();
    for width in [1440.0, 375.0] {
        for (state, body) in [
            ("mdb", "Set db=OpenDatabase(\"C:\\synthetic\\etabs.mdb\")"),
            ("dxf", "Set acad=CreateObject(\"AutoCAD.Application\")\nSet ent=acad.ActiveDocument.ModelSpace.AddPoint(Array(0,0,0))"),
        ] {
            let mut h = harness(body, width, true);
            h.render().unwrap().save(std::path::Path::new(&dir).join(format!("emulation-{state}-{width:.0}.png"))).unwrap();
        }
    }
}
