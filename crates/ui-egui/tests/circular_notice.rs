use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::{SheetApp, i18n::Language};
use serde_json::json;

#[test]
fn circular_notice_is_visible_dismissible_and_stays_dismissed_during_recalculation() {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.language = Language::Vi;
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(4);
    harness.state_mut().begin_edit(Some("=A1".into()), true);
    assert!(harness.state_mut().commit_edit(0, 0, false, false));
    harness.run_steps(3);
    let message = "Công thức này tạo ra tham chiếu vòng tròn nên ô hiển thị #CIRC!. Hãy sửa công thức để không tự tham chiếu đến chính nó.";
    assert!(harness.query_by_label(message).is_some());
    harness.state_mut().run("ui.notice.dismiss", json!({})).unwrap();
    harness.run_steps(3);
    harness.state_mut().run("formulas.calculateNow", json!({})).unwrap();
    harness.run_steps(3);
    assert!(harness.query_by_label(message).is_none());
    assert!(harness.state().notice.is_none());
    assert!(harness.state().message.is_none());
}
