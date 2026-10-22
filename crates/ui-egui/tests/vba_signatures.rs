use gridcraft_engine::{Session, io};
use gridcraft_ui_egui::{Services, SheetApp, i18n::Language};
use serde_json::json;
use std::sync::Arc;

fn app() -> SheetApp {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Services::default());
    app.ui.language = Language::Vi;
    let wb = Arc::make_mut(&mut app.session.active_mut().unwrap().wb);
    wb.vba_project = Some(b"synthetic".to_vec());
    wb.legacy_vba_signature = true;
    app
}

#[test]
fn vba_signature_open_and_save_notices_include_natural_vietnamese() {
    let mut app = app();
    let warning = gridcraft_xlsx::LEGACY_SIGNATURE;
    let translated = "Chữ ký số của macro trong tệp .xls không được giữ khi lưu .xlsm; macro sẽ chưa được ký";
    app.show_open_warnings(&json!({"warnings": [warning, gridcraft_engine::legacy::BUTTONS]}));
    assert!(app.notice.as_ref().unwrap().text.contains(translated));
    let result = app.run("file.saveBytes", json!({"format": "xlsx"})).unwrap();
    assert!(result["warnings"].as_array().unwrap().iter().any(|w| w == io::REMOVED_SIGNED_MACROS));
    let text = &app.notice.as_ref().unwrap().text;
    assert!(text.contains(translated));
    assert!(text.contains(Language::Vi.tr(io::REMOVED_SIGNED_MACROS)));
    if let Ok(path) = std::env::var("GRID03_SCREENSHOT_PATH") {
        app.message = None;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).wgpu().build_ui_state(
            |ui, app: &mut SheetApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(8);
        h.render().unwrap().save(path).unwrap();
    }
}

#[test]
fn vba_signature_draft_warnings_survive_recovery_and_show_once() {
    let mut app = app();
    let doc = app.session.active().unwrap();
    let (uid, wb) = (doc.uid, doc.wb.clone());
    let warnings = io::save_warnings(&wb, "macro.xlsm");
    app.show_draft_warnings(uid, &warnings);
    assert!(app.notice.is_some());
    app.notice = None;
    app.show_draft_warnings(uid, &warnings);
    assert!(app.notice.is_none(), "a dismissed draft warning must not reappear every autosave");
    let bytes = gridcraft_ui_egui::drafts::draft_bytes(&wb, "macro.xlsm").unwrap();
    let recovered = app.open_draft("macro.xlsm", &bytes).unwrap();
    assert_ne!(recovered, uid);
    assert!(app.session.active().unwrap().wb.legacy_vba_signature);
    assert!(app.notice.as_ref().unwrap().text.contains(Language::Vi.tr(gridcraft_xlsx::LEGACY_SIGNATURE)));
}

#[test]
fn vba_signature_multiline_legacy_dialog_is_translated() {
    let message = format!("{}\n{}", gridcraft_engine::legacy::OPENED_MACROS, gridcraft_xlsx::LEGACY_SIGNATURE);
    let translated = Language::Vi.tr_message(&message);
    assert!(translated.contains(Language::Vi.tr(gridcraft_xlsx::LEGACY_SIGNATURE)));
    assert!(!translated.contains("legacy .xls"));
}

#[test]
fn vba_signature_browser_save_and_exports_show_shared_warnings() {
    let mut app = app();
    let downloads = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = downloads.clone();
    app.services.download = Some(Box::new(move |name, _bytes, report| {
        captured.lock().unwrap().push(name.to_string());
        report.settle(gridcraft_ui_egui::Delivery::Delivered);
    }));
    app.open_dialog("saveAs", json!({"format": "xlsm"}));
    assert!(downloads.lock().unwrap().iter().any(|n| n.ends_with(".xlsm")));
    assert!(app.notice.as_ref().unwrap().text.contains(Language::Vi.tr(gridcraft_xlsx::LEGACY_SIGNATURE)));
    app.notice = None;
    assert!(gridcraft_ui_egui::backstage::export_as(&mut app, "csv"));
    assert!(downloads.lock().unwrap().iter().any(|n| n.ends_with(".csv")));
    assert!(app.notice.as_ref().unwrap().text.contains(Language::Vi.tr(io::REMOVED_SIGNED_MACROS)));
}
