use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_ui_egui::{SheetApp, i18n::Language};
use gridcraft_vba::testing::{FixtureModule, fixture};
use serde_json::json;
use std::sync::Arc;

fn app() -> SheetApp {
    let mut s = Session::new();
    s.new_workbook();
    Arc::make_mut(&mut s.active_mut().unwrap().wb).vba_project = Some(fixture(&[
        FixtureModule::new("Module1", "Module", "Public Sub CalculateLoads()\nDim x As Double\nx = 2\nEnd Sub\nPublic Function Twice(Optional ByVal x As Double = 2) As Double\nTwice = x * 2\nEnd Function").unwrap(),
        FixtureModule::new("ThisWorkbook", "Document", "Private Sub Workbook_Open()\nEnd Sub").unwrap(),
    ], 1252, "", &[]).unwrap());
    Arc::make_mut(&mut s.active_mut().unwrap().wb).macro_buttons.push(gridcraft_engine::model::MacroButton {
        sheet: 0,
        name: "Button1".into(),
        caption: "Calculate loads".into(),
        macro_name: "Module1.CalculateLoads".into(),
        kind: gridcraft_engine::model::MacroButtonKind::FormControl,
    });
    let mut app = SheetApp::new(s, Default::default());
    app.ui.language = Language::Vi;
    app.open_dialog("macros", json!({}));
    app
}

fn harness(width: f32) -> egui_kittest::Harness<'static, SheetApp> {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, 900.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app(),
    );
    h.run_steps(6);
    h
}

fn assert_dialog_fits(h: &egui_kittest::Harness<'static, SheetApp>, width: f32) {
    let rect = h.ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(gridcraft_ui_egui::dialogs::MACRO_DIALOG_LAYOUT))).unwrap();
    assert!(rect.left() >= 0.0 && rect.right() <= width && rect.top() >= 0.0 && rect.bottom() <= 900.0, "{width}: {rect:?}");
}

#[test]
fn macro_procedure_viewport_stays_below_summary_when_code_opens() {
    for width in [1440.0, 375.0] {
        let mut h = harness(width);
        for code in [false, true] {
            if code {
                h.get_by_label("Xem mã").click();
                h.run_steps(4);
            }
            let rect = h.ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new("gridcraft.macros.procedures.rect"))).unwrap();
            assert!(rect.height() >= 200.0, "{width}, code={code}: {rect:?}");
            let summary = h.get_by_label("2 mô-đun · 3 thủ tục · 1 UDF · Sự kiện tự chạy: Có · Đã ký: Không · Đã khóa: Không").rect();
            assert!(rect.top() >= summary.bottom(), "{width}, code={code}: {rect:?}, {summary:?}");
            let first = h.get_by_label("CalculateLoads — Module1").rect();
            assert!(rect.contains_rect(first), "{width}, code={code}: {first:?}, {rect:?}");
            assert!(h.query_all_by_label("Mức 1").count() > 0);
        }
    }
}

#[test]
fn macro_dialog_labels_source_and_narrow_layouts() {
    for width in [1440.0, 320.0, 375.0, 390.0] {
        let mut h = harness(width);
        assert_dialog_fits(&h, width);
        for label in [
            "Macro chưa được chạy trong Bảng tính; đây là phân tích chỉ đọc.",
            "CalculateLoads — Module1",
            "Xem mã",
            "1 · Chạy được khi có trình chạy macro",
        ] {
            let rect = h.get_by_label(label).rect();
            assert!(rect.left() >= 0.0 && rect.right() <= width, "{width}: {label}: {rect:?}");
        }
        assert!(h.query_by_label("Sheet1 · Calculate loads · Điều khiển biểu mẫu").is_some());
        h.get_by_label("Xem mã").click();
        h.run_steps(4);
        assert_dialog_fits(&h, width);
        assert!(h.query_by_label("    1  Public Sub CalculateLoads()").is_some());
        let bounds = h.ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(gridcraft_ui_egui::dialogs::MACRO_DIALOG_LAYOUT))).unwrap();
        assert!(bounds.contains_rect(h.get_by_label("    3  x = 2").rect()), "source must be visible at {width}");
        assert!(h.query_by_label("Chạy").is_none());
    }
}

#[test]
fn macro_error_checking_explains_udf_and_opens_its_procedure() {
    let mut h = harness(1440.0);
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "=_xludf.Twice(3)"})).unwrap();
    h.state_mut().open_dialog("errorChecking", json!({}));
    h.run_steps(4);
    assert!(h.query_by_label("Hàm VBA tự định nghĩa `Twice` (Module1, dòng 5) — Bảng tính chưa chạy macro nên chưa tính được").is_some());
    h.get_by_label("Xem mã").click();
    h.run_steps(4);
    let dialog = h.state().dialog.as_ref().unwrap();
    assert_eq!(dialog.name, "macros");
    let selected = &dialog.result.as_ref().unwrap()["procedures"][dialog.list_index];
    assert_eq!(selected["name"], "Twice");
    h.get_by_label("Xem mã").click();
    h.run_steps(4);
    assert!(h.query_by_label("    5  Public Function Twice(Optional ByVal x As Double = 2) As Double").is_some());
}

#[test]
fn macro_view_code_scrolls_to_a_procedure_in_a_long_module() {
    let mut h = harness(375.0);
    let source = format!("{}Public Function Deep() As Double\nDeep = 2\nEnd Function", "' spacer\n".repeat(500));
    Arc::make_mut(&mut h.state_mut().session.active_mut().unwrap().wb).vba_project =
        Some(fixture(&[FixtureModule::new("Module1", "Module", &source).unwrap()], 1252, "", &[]).unwrap());
    h.state_mut().open_dialog("macros", json!({"procedure": "Deep", "module": "Module1"}));
    h.run_steps(6);
    h.get_by_label("Xem mã").click();
    h.run_steps(6);
    assert_dialog_fits(&h, 375.0);
    let line = h.get_by_label("  501  Public Function Deep() As Double").rect();
    assert!(line.top() >= 0.0 && line.bottom() <= 900.0);
    assert!(h.query_by_label("    1  ' spacer").is_none());
}

#[test]
fn macro_dialog_screenshots_when_requested() {
    let Some(dir) = std::env::var_os("GRIDCRAFT_MACRO_SCREENSHOTS") else { return };
    std::fs::create_dir_all(&dir).unwrap();
    for width in [1440.0, 375.0] {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, 900.0)).wgpu().build_ui_state(
            |ui, app: &mut SheetApp| {
                app.logic(&ui.ctx().clone());
                app.ui(ui);
            },
            app(),
        );
        h.run_steps(6);
        h.render().unwrap().save(std::path::Path::new(&dir).join(format!("macro-{width:.0}.png"))).unwrap();
        h.get_by_label("Xem mã").click();
        h.run_steps(4);
        h.render().unwrap().save(std::path::Path::new(&dir).join(format!("macro-code-{width:.0}.png"))).unwrap();
    }
}
