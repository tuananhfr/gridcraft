use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::{
    Session,
    core::{CellRef, Value},
    model::{Cell, Formula, Workbook},
};
use gridcraft_ui_egui::{SheetApp, i18n::Language};
use serde_json::json;

#[test]
fn unparsed_cache_notice_opens_error_checking_and_clears_after_edit() {
    let mut wb = Workbook::new();
    let mut cell = Cell::formula(Formula::new("1+"));
    cell.value = Value::Number(123.0);
    let sh = wb.sheet_mut(0).unwrap();
    sh.cells.set(CellRef::new(0, 0), cell);
    sh.cells.set(CellRef::new(0, 1), Cell::formula(Formula::new("A1*2")));
    let bytes = gridcraft_engine::io::save_bytes(&wb, "bad.xlsx").unwrap();
    let mut session = Session::new();
    session.execute("file.open", json!({"name": "bad.xlsx", "base64": gridcraft_engine::io::base64_encode(&bytes)})).unwrap();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.language = Language::Vi;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).wgpu().build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    let message = "Có 2 ô chưa tính được. Mở Kiểm tra lỗi để xem chi tiết.";
    assert!(h.query_by_label(message).is_some());
    assert!(h.state().message.is_none(), "the notice must not block editing");
    if let Some(dir) = std::env::var_os("GRIDCRAFT_PARSE_SCREENSHOTS") {
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(std::path::Path::new(&dir).join("unparsed-cache.png")).unwrap();
    }
    h.get_by_label(Language::Vi.tr("Error Checking")).click();
    h.run_steps(3);
    assert!(h.query_by_label("Công thức không đọc được — giá trị hiển thị là giá trị cũ trong tệp, chưa được tính lại").is_some());
    assert!(h.query_by_label("Phụ thuộc ô chưa tính được").is_some());
    if let Some(dir) = std::env::var_os("GRIDCRAFT_PARSE_SCREENSHOTS") {
        h.render().unwrap().save(std::path::Path::new(&dir).join("unparsed-errors.png")).unwrap();
    }
    h.state_mut().run("ui.notice.dismiss", json!({})).unwrap();
    h.state_mut().run("formulas.calculateNow", json!({})).unwrap();
    assert!(h.state().notice.is_some());
    h.state_mut().begin_edit(Some("=1+1".into()), true);
    assert!(h.state_mut().commit_edit(0, 0, false, false));
    assert!(h.state().session.doc().unwrap().calc.not_calculated.is_empty());
    assert!(h.state().notice.is_none());
}
