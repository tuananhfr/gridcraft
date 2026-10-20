//! The status bar's save state: "Saving…" while the browser's save picker is open, "Save failed"
//! with its reason until the next successful save or edit.

use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::i18n::Language;
use gridcraft_ui_egui::save_status::SaveState;
use gridcraft_ui_egui::{Delivery, DeliveryReport, Services, SheetApp};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;
type Reports = std::sync::Arc<std::sync::Mutex<Vec<DeliveryReport>>>;

fn web() -> (Harness, Reports) {
    let reports: Reports = Default::default();
    let started = reports.clone();
    let services = Services {
        download: Some(Box::new(move |_: &str, _: &[u8], report: DeliveryReport| started.lock().unwrap().push(report))),
        ..Default::default()
    };
    let mut s = Session::new();
    s.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(s, services),
    );
    h.run_steps(4);
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    (h, reports)
}

fn uid(h: &Harness) -> u64 {
    h.state().session.doc().unwrap().uid
}

#[test]
fn saving_then_saved() {
    let (mut h, reports) = web();
    h.state_mut().run_or_alert("file.save", json!({}));
    h.run_steps(2);
    assert_eq!(h.state().save_state(uid(&h)), SaveState::Saving);
    assert!(h.query_by_label("Saving…").is_some());
    reports.lock().unwrap()[0].settle(Delivery::Delivered);
    h.run_steps(2);
    assert_eq!(h.state().save_state(uid(&h)), SaveState::Idle);
    assert!(h.query_by_label("Saving…").is_none());
}

#[test]
fn a_failed_save_shows_until_an_edit_or_the_next_save() {
    let (mut h, reports) = web();
    h.state_mut().ui.language = Language::Vi;
    h.state_mut().run_or_alert("file.save", json!({}));
    h.run_steps(2);
    assert!(h.query_by_label("Đang lưu…").is_some());
    reports.lock().unwrap()[0].settle(Delivery::Failed("the disk is full".into()));
    h.run_steps(2);
    assert_eq!(h.state().save_state(uid(&h)), SaveState::Failed("the disk is full".into()));
    let message = h.state_mut().message.take().map(|(_, m)| m).unwrap();
    assert_eq!(Language::Vi.tr_message(&message), "Không lưu được 'Book1.xlsx': the disk is full");
    h.run_steps(2);
    let label = h.get_by_label("Lưu không thành công");
    label.click();
    h.run_steps(2);
    assert_eq!(h.state().message.as_ref().map(|(_, m)| m.as_str()), Some("the disk is full"), "a click shows the reason");
    h.state_mut().message = None;

    h.state_mut().run_or_alert("file.save", json!({}));
    reports.lock().unwrap()[1].settle(Delivery::Delivered);
    h.run_steps(2);
    assert!(h.query_by_label("Lưu không thành công").is_none(), "the next successful save clears it");

    h.state_mut().run("cell.set", json!({"cell": "A2", "input": "2"})).unwrap();
    h.state_mut().run_or_alert("file.save", json!({}));
    reports.lock().unwrap()[2].settle(Delivery::Failed("denied".into()));
    h.run_steps(2);
    h.state_mut().message = None;
    assert!(h.query_by_label("Lưu không thành công").is_some());
    h.state_mut().run("cell.set", json!({"cell": "A3", "input": "3"})).unwrap();
    h.run_steps(2);
    assert!(h.query_by_label("Lưu không thành công").is_none(), "an edit clears it");
}

#[test]
fn a_cancelled_picker_is_not_a_failure() {
    let (mut h, reports) = web();
    h.state_mut().run_or_alert("file.save", json!({}));
    reports.lock().unwrap()[0].settle(Delivery::Cancelled);
    h.run_steps(2);
    assert_eq!(h.state().save_state(uid(&h)), SaveState::Idle);
    assert!(h.state().message.is_none());
}

#[test]
fn a_failed_desktop_write_shows_the_failure() {
    let mut s = Session::new();
    s.new_workbook();
    let mut app = SheetApp::new(s, Services::default());
    app.run("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    let missing = std::env::temp_dir().join(format!("gridcraft-no-such-dir-{}", std::process::id())).join("x.xlsx");
    app.run_or_alert("file.save", json!({"path": missing.to_string_lossy()}));
    let uid = app.session.doc().unwrap().uid;
    assert!(matches!(app.save_state(uid), SaveState::Failed(_)), "{:?}", app.save_state(uid));
    assert!(app.message.is_some(), "the message box still says why");
}
