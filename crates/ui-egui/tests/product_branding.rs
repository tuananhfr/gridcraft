use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::{Services, SheetApp, i18n::Language};
use serde_json::json;

#[test]
fn product_branding_about_and_start_keep_web_default() {
    for product in ["GridCraft", "Bảng tính – ERPCons"] {
        for dialog in ["about", "start"] {
            let mut app = SheetApp::new(Session::new(), Services::default());
            assert_eq!(app.product_name, "GridCraft");
            app.product_name = product;
            app.ui.language = if product == "GridCraft" { Language::En } else { Language::Vi };
            app.open_dialog(dialog, json!({}));
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
                |ui, app: &mut SheetApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                app,
            );
            h.run_steps(4);
            assert!(h.query_all_by_label_contains(product).count() > 0);
            if dialog == "about" && product != "GridCraft" {
                assert!(h.query_by_label("ERPCons").is_some());
                assert!(h.query_by_label("https://office.erpcons.vn").is_some());
            }
            if product != "GridCraft"
                && let Ok(dir) = std::env::var("GRIDCRAFT_BRANDING_SHOTS")
            {
                h.render().unwrap().save(std::path::Path::new(&dir).join(format!("{dialog}.png"))).unwrap();
            }
        }
    }
}

#[test]
fn product_branding_system_messages_preserve_workbook_names() {
    let mut app = SheetApp::new(Session::new(), Services::default());
    let message = "'GridCraft.xlsx' uses encryption, which GridCraft can't open yet.";
    assert_eq!(app.product_message(message), message);
    app.product_name = "Bảng tính – ERPCons";
    assert_eq!(app.product_message(message), "'GridCraft.xlsx' uses encryption, which Bảng tính – ERPCons can't open yet.");
    app.ui.language = Language::Vi;
    let translated =
        app.product_message("'GridCraft.xlsx' uses AES encryption, which GridCraft can't open yet. Remove the password in Excel and try again.");
    assert!(translated.contains("'GridCraft.xlsx'"));
    assert!(translated.contains("Bảng tính – ERPCons chưa mở được"));
}
