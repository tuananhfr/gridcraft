use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::{SheetApp, i18n::Language};
use serde_json::json;

fn harness() -> egui_kittest::Harness<'static, SheetApp> {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("cell.set", json!({"cell": "A1", "input": "2"})).unwrap();
    session.execute("cell.set", json!({"cell": "B1", "input": "=A1*2"})).unwrap();
    session.execute("formulas.calculationOptions", json!({"mode": "manual"})).unwrap();
    session.execute("cell.set", json!({"cell": "A1", "input": "3"})).unwrap();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.language = Language::Vi;
    app.ui.ribbon_tab = "Formulas".into();
    egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).wgpu().build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    )
}

fn screenshot(h: &mut egui_kittest::Harness<'_, SheetApp>, name: &str) {
    if let Some(dir) = std::env::var_os("GRIDCRAFT_CALC_SCREENSHOTS") {
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(std::path::Path::new(&dir).join(name)).unwrap();
    }
}

#[test]
fn manual_status_and_selection_button_are_visible_in_vietnamese() {
    let mut h = harness();
    h.run_steps(5);
    assert!(h.query_by_label("Cần tính lại").is_some());
    assert!(h.query_by_label("Tính vùng chọn").is_some());
    screenshot(&mut h, "manual-formulas.png");
    h.state_mut().run("formulas.calculateNow", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Cần tính lại").is_none());
}

#[test]
fn error_checking_is_a_translated_table_with_named_causes() {
    let mut h = harness();
    for (cell, input) in [("D1", "=NOSUCHFN()"), ("D2", "=_xludf.MyRate()"), ("D3", "=MissingName")] {
        h.state_mut().run("cell.set", json!({"cell": cell, "input": input})).unwrap();
    }
    h.state_mut().run("formulas.calculateNow", json!({})).unwrap();
    h.state_mut().open_dialog("errorChecking", json!({}));
    h.run_steps(5);
    for label in ["Nguyên nhân", "Gợi ý", "Sheet1!D1", "Hàm không xác định: NOSUCHFN", "Còn lỗi"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    screenshot(&mut h, "calculation-errors.png");
}

#[test]
fn import_details_show_translated_six_item_list() {
    let mut h = harness();
    h.state_mut().show_open_warnings(&json!({"warnings": [
        "Imported from an Excel 97-2003 (.xls) workbook: values, formulas, fonts, number formats, merged cells, column widths, row heights and defined names were read. Fills, borders, charts, pictures, comments and other features are not imported yet. Save as .xlsx; saving .xls is not supported.",
        "Data validation in .xls files is not imported yet.",
        "Pictures, charts and shapes in .xls files are not imported yet.",
        "Cell comments in .xls files are not imported yet.",
        "Conditional formatting in .xls files is not imported yet.",
        "Some defined names use references to other workbooks (external links) and were skipped.",
    ]}));
    h.state_mut().open_dialog("importWarnings", json!({}));
    h.run_steps(5);
    assert!(h.query_by_label("Chưa nhập được chú thích ô trong tệp .xls.").is_some());
    screenshot(&mut h, "import-details-vi.png");
}
