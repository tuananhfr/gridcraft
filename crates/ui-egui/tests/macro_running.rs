#![cfg(not(target_arch = "wasm32"))]
use egui_kittest::kittest::Queryable;
use gridcraft_engine::{
    Session,
    core::{CellRef, Value},
};
use gridcraft_ui_egui::{SheetApp, i18n::Language};
use gridcraft_vba::testing::{FixtureModule, fixture};
use serde_json::json;
use std::sync::Arc;

fn app(source: &str) -> SheetApp {
    let mut session = Session::new();
    session.new_workbook();
    Arc::make_mut(&mut session.doc_mut().unwrap().wb).vba_project =
        Some(fixture(&[FixtureModule::new("Module1", "Module", source).unwrap()], 1252, "", &[]).unwrap());
    let mut app = SheetApp::new(session, Default::default());
    app.ui.language = Language::Vi;
    app.open_dialog("macros", json!({}));
    app
}
fn harness(source: &str, width: f32, render: bool) -> egui_kittest::Harness<'static, SheetApp> {
    let builder = egui_kittest::Harness::builder().with_size(egui::vec2(width, 900.0));
    let builder = if render { builder.wgpu() } else { builder };
    let mut h = builder.build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app(source),
    );
    h.run_steps(6);
    h
}
fn value(app: &SheetApp, cell: &str) -> Value {
    app.session.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::parse(cell).unwrap())
}
fn run(h: &mut egui_kittest::Harness<'static, SheetApp>) {
    h.get_by_label("Chạy").click();
    h.run_steps(2);
    h.get_by_label("Cho phép và chạy").click();
    h.run_steps(6);
}
const FILL: &str = "Sub Main()\nRange(\"A1:A5\").Value=42\nEnd Sub";
const LOOP: &str = "Sub Main()\nRange(\"A1\").Value=42\nDo\nLoop\nEnd Sub";

#[test]
fn macro_run_requires_consent_and_has_one_undo() {
    let mut h = harness(FILL, 1440.0, false);
    h.get_by_label("Chạy").click();
    h.run_steps(3);
    assert!(h.query_by_label("Cho phép chạy macro này?").is_some());
    assert_eq!(value(h.state(), "A1"), Value::Empty);
    h.get_by_label("Hủy").click();
    h.run_steps(2);
    assert!(!h.state().session.doc().unwrap().macro_state.consent);
    run(&mut h);
    h.run_steps(5);
    assert_eq!(value(h.state(), "A5"), Value::Number(42.0));
    assert_eq!(h.state().session.doc().unwrap().undo.len(), 1);
    assert_eq!(h.state().session.doc().unwrap().macro_state.log.last().unwrap()["status"], "completed");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(value(h.state(), "A1"), Value::Empty);
    h.get_by_label("Thu hồi quyền chạy macro").click();
    h.run_steps(2);
    assert!(!h.state().session.doc().unwrap().macro_state.consent);
}

#[test]
fn macro_stop_rolls_back_and_does_not_block_frames() {
    let mut h = harness(LOOP, 375.0, false);
    run(&mut h);
    assert!(h.query_by_label("Dừng").is_some());
    assert_eq!(value(h.state(), "A1"), Value::Empty);
    h.get_by_label("Dừng").click();
    h.run_steps(4);
    assert_eq!(value(h.state(), "A1"), Value::Empty);
    assert!(h.state().session.doc().unwrap().undo.is_empty());
    assert_eq!(h.state().session.doc().unwrap().macro_state.log.last().unwrap()["status"], "cancelled");
    assert!(h.query_by_label("Đã hoàn tác thay đổi của macro; sổ tính không thay đổi.").is_some());
}

#[test]
fn macro_input_and_message_dialogs_resume_with_user_answers() {
    let source =
        "Sub Main()\nRange(\"A1\").Value=InputBox(\"Enter a value\",\"Input\",\"seed\")\nRange(\"A2\").Value=MsgBox(\"Continue?\",vbYesNo)\nEnd Sub";
    let mut h = harness(source, 375.0, false);
    run(&mut h);
    assert!(h.query_by_label("Enter a value").is_some());
    assert_eq!(value(h.state(), "A1"), Value::Empty);
    h.get_by_label("OK").click();
    h.run_steps(6);
    h.get_by_label("Có").click();
    h.run_steps(4);
    assert_eq!(value(h.state(), "A1"), Value::Text("seed".into()));
    assert_eq!(value(h.state(), "A2"), Value::Number(6.0));
}

#[test]
fn macro_refusal_reports_level_and_preserves_workbook() {
    let source = "Sub Main()\nDim o As Object\nSet o=CreateObject(\"Synthetic.Test\")\nEnd Sub";
    let mut h = harness(source, 375.0, false);
    let wb = h.state().session.doc().unwrap().wb.clone();
    h.get_by_label("Chạy").click();
    h.run_steps(3);
    assert!(h.query_by_label("Chưa hỗ trợ mức tương thích 3.").is_some());
    assert_eq!(h.state().session.doc().unwrap().wb, wb);
    assert!(h.state().session.doc().unwrap().undo.is_empty());
}

#[test]
fn macro_running_screenshots_when_requested() {
    let Some(dir) = std::env::var_os("GRIDCRAFT_MACRO_SCREENSHOTS") else { return };
    std::fs::create_dir_all(&dir).unwrap();
    for width in [1440.0, 375.0] {
        let save = |h: &mut egui_kittest::Harness<'static, SheetApp>, state: &str| {
            h.render().unwrap().save(std::path::Path::new(&dir).join(format!("macro-{state}-{width:.0}.png"))).unwrap();
        };
        let mut h = harness(LOOP, width, true);
        h.get_by_label("Chạy").click();
        h.run_steps(2);
        save(&mut h, "consent");
        h.get_by_label("Cho phép và chạy").click();
        h.run_steps(2);
        save(&mut h, "running");
        let mut h = harness(FILL, width, true);
        run(&mut h);
        h.run_steps(4);
        h.get_by_label("Nhật ký chạy").click();
        h.run_steps(2);
        save(&mut h, "finished");
        let mut h = harness("Sub Main()\nDim o As Object\nSet o=CreateObject(\"Synthetic.Test\")\nEnd Sub", width, true);
        h.get_by_label("Chạy").click();
        h.run_steps(3);
        save(&mut h, "refusal");
    }
}

#[test]
fn macro_form_control_entry_runs_its_assigned_procedure() {
    let mut h = harness("Sub Main()\nEnd Sub\nSub Other()\nRange(\"A1\").Value=99\nEnd Sub", 375.0, false);
    Arc::make_mut(&mut h.state_mut().session.doc_mut().unwrap().wb).macro_buttons.push(gridcraft_engine::model::MacroButton {
        sheet: 0,
        name: "RunOther".into(),
        caption: "Run assigned procedure".into(),
        macro_name: "Module1.Other".into(),
        kind: gridcraft_engine::model::MacroButtonKind::FormControl,
    });
    h.state_mut().open_dialog("macros", json!({}));
    h.run_steps(6);
    h.get_by_label("Nút macro").click();
    h.run_steps(6);
    h.query_all_by_label("Chạy").nth(1).unwrap().click();
    h.run_steps(4);
    h.get_by_label("Cho phép và chạy").click();
    h.run_steps(6);
    assert_eq!(value(h.state(), "A1"), Value::Number(99.0));
    assert_eq!(h.state().session.doc().unwrap().macro_state.log.last().unwrap()["macro"], "Module1.Other");
}

#[test]
fn macro_file_picker_is_explicit_and_returns_false_without_opening_a_document() {
    let mut h = harness("Sub Main()\nRange(\"A1\").Value=Application.GetOpenFilename()\nEnd Sub", 375.0, false);
    let picked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let report = picked.clone();
    h.state_mut().services.pick_open = Some(Box::new(move || {
        report.store(true, std::sync::atomic::Ordering::Relaxed);
        Some("/synthetic/chosen.mdb".into())
    }));
    run(&mut h);
    assert!(!picked.load(std::sync::atomic::Ordering::Relaxed));
    h.get_by_label("Chọn tệp…").click();
    h.run_steps(6);
    assert!(picked.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(value(h.state(), "A1"), Value::Bool(false));
    assert_eq!(h.state().session.documents().len(), 1);
}
