//! The narrow title bar must expose the same close flow as File > Close.

use std::sync::{Arc, Mutex};

use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::i18n::Language;
use gridcraft_ui_egui::{Delivery, DeliveryReport, Services, SheetApp};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness(services: Services) -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, services);
    app.ui.language = Language::Vi;
    let mut h = Harness::builder().with_size(egui::vec2(390.0, 844.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

#[test]
fn close_button_at_390_is_visible_and_closes_a_clean_workbook() {
    let mut h = harness(Services::default());
    let uid = h.state().session.active().unwrap().uid;
    let button = h.get_by_label("Đóng tài liệu");
    let rect = button.rect();
    assert!(rect.left() >= 0.0 && rect.right() <= 390.0);
    assert!(rect.top() >= 0.0 && rect.bottom() <= 844.0);
    button.click();
    h.run_steps(3);
    assert!(h.state().document_index(Some(uid)).is_none());
}

#[test]
fn close_button_at_390_prompts_and_cancel_keeps_dirty_workbook() {
    let mut h = harness(Services::default());
    let uid = h.state().session.active().unwrap().uid;
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "draft"})).unwrap();
    h.run_steps(2);
    h.get_by_label("Đóng tài liệu").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().unwrap().name, "saveChanges");
    h.get_by_label(Language::Vi.tr("Cancel")).click();
    h.run_steps(3);
    assert!(h.state().document_index(Some(uid)).is_some());
    assert!(h.state().session.active().unwrap().is_dirty());
    h.get_by_label("Đóng tài liệu").click();
    h.run_steps(3);
    h.get_by_label(Language::Vi.tr("Don't Save")).click();
    h.run_steps(3);
    assert!(h.state().document_index(Some(uid)).is_none());
}

#[test]
fn dialog_save_at_390_waits_for_browser_delivery() {
    let reports: Arc<Mutex<Vec<DeliveryReport>>> = Arc::default();
    let writes = reports.clone();
    let mut h = harness(Services {
        download: Some(Box::new(move |_, _, report| writes.lock().unwrap().push(report))),
        ..Default::default()
    });
    let uid = h.state().session.active().unwrap().uid;
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "draft"})).unwrap();
    h.run_steps(2);
    h.get_by_label("Đóng tài liệu").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::Button, Language::Vi.tr("Save")).click();
    h.run_steps(3);
    assert_eq!(reports.lock().unwrap().len(), 1);
    assert!(h.state().document_index(Some(uid)).is_some());
    reports.lock().unwrap()[0].settle(Delivery::Delivered);
    h.run_steps(3);
    assert!(h.state().document_index(Some(uid)).is_none());
}
