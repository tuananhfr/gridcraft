//! The grid, status bar and number-format previews display values in the regional format.
use egui::Shape;
use gridcraft_engine::Session;
use gridcraft_engine::core::Region;
use gridcraft_ui_egui::{SheetApp, i18n::Language};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness(language: Language) -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    for (cell, input) in [("A1", "1234.5"), ("A2", "1234.5"), ("A3", "0.125"), ("A4", "10/19/2026"), ("A5", "1000000"), ("A6", "-0.5")] {
        s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
    }
    s.execute("home.numberFormat", json!({"range": "A2", "code": "#,##0.00"})).unwrap();
    s.execute("home.numberFormat", json!({"range": "A3", "code": "0.0%"})).unwrap();
    s.execute("home.numberFormat", json!({"range": "A5", "code": "#,##0 [$₫-42A]"})).unwrap();
    s.execute("home.numberFormat", json!({"range": "A6", "code": "0.00;[Red](0.00)"})).unwrap();
    s.execute("home.columnWidth", json!({"cols": "A:A", "chars": 16})).unwrap();
    s.execute("selection.set", json!({"range": "A1:A2"})).unwrap();
    let mut app = SheetApp::new(s, Default::default());
    app.ui.language = language;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn painted(h: &Harness) -> Vec<String> {
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

fn shows(h: &Harness, text: &str) -> bool {
    painted(h).iter().any(|t| t == text)
}

#[test]
fn the_grid_and_status_bar_show_vietnamese_numbers_and_dates() {
    let mut h = harness(Language::Vi);
    assert_eq!(h.state().ui.region(), Region::ViVn);
    for text in ["1234,5", "1.234,50", "12,5%", "19/10/2026", "1.000.000 ₫", "(0,50)"] {
        assert!(shows(&h, text), "{text} in {:?}", painted(&h));
    }
    assert!(painted(&h).iter().any(|t| t.contains("2.469")), "status bar sum: {:?}", painted(&h));
    for text in ["1,234.50", "12.5%", "10/19/2026", "1,000,000 ₫"] {
        assert!(!shows(&h, text), "{text} is en-US");
    }
    // The same workbook in en-US, unchanged from before regions.
    h.state_mut().run("app.region.set", json!({"region": "en-US"})).unwrap();
    h.run_steps(2);
    for text in ["1234.5", "1,234.50", "12.5%", "10/19/2026", "1,000,000 ₫", "(0.50)"] {
        assert!(shows(&h, text), "{text} in {:?}", painted(&h));
    }
    // Spanish keeps the en-US display it had.
    h.state_mut().run("app.region.set", json!({"region": "es-ES"})).unwrap();
    h.run_steps(2);
    assert!(shows(&h, "1,234.50") && shows(&h, "10/19/2026"));
}

#[test]
fn the_format_cells_sample_and_currency_follow_the_region() {
    let mut h = harness(Language::Vi);
    h.state_mut().run("selection.set", json!({"range": "A2"})).unwrap();
    h.state_mut().open_dialog("formatCells", json!({"tab": "Number"}));
    h.run_steps(3);
    assert!(shows(&h, "1.234,50"), "sample in {:?}", painted(&h));
    assert_eq!(gridcraft_engine::cmd::format::format_code_in("Currency", Region::ViVn), "#,##0 [$₫-42A]");
    assert_eq!(gridcraft_engine::cmd::format::format_code_in("Accounting", Region::ViVn), gridcraft_engine::core::region::VND_ACCOUNTING);
    assert_eq!(gridcraft_engine::cmd::format::format_code_in("Currency", Region::EnUs), "\"$\"#,##0.00");
    assert_eq!(gridcraft_engine::cmd::format::format_code_in("Short Date", Region::ViVn), "m/d/yyyy");
}

#[test]
fn copy_follows_the_interface_region() {
    let mut h = harness(Language::Vi);
    let app = h.state_mut();
    let copied = app.run("edit.copy", json!({"range": "A2:A3"})).unwrap();
    assert_eq!(copied["text"], "1.234,50\n12,5%\n");
    app.run("app.region.set", json!({"region": "en-US"})).unwrap();
    let copied = app.run("edit.copy", json!({"range": "A2:A3"})).unwrap();
    assert_eq!(copied["text"], "1,234.50\n12.5%\n");
}
