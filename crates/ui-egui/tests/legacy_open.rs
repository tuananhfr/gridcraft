//! Synthetic OLE2 workbooks exercise both file-picker entry points.

use std::sync::{Arc, Mutex};

use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::{Session, legacy};
use gridcraft_ui_egui::{Inbox, Services, SheetApp, i18n::Language};
use serde_json::json;

fn harness(services: Services) -> egui_kittest::Harness<'static, SheetApp> {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, services);
    app.ui.language = Language::Vi;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn legacy_bytes() -> Vec<u8> {
    let rec = |id: u16, payload: &[u8]| [&id.to_le_bytes()[..], &(payload.len() as u16).to_le_bytes(), payload].concat();
    let bof = |kind: u16| rec(0x0809, &[&0x0600u16.to_le_bytes()[..], &kind.to_le_bytes(), &[0; 12]].concat());
    let sheet = [bof(0x0010), rec(0x0203, &[&[0; 6][..], &42f64.to_le_bytes()].concat()), rec(0x000A, &[])].concat();
    let globals = |offset: u32| {
        [bof(0x0005), rec(0x0085, &[&offset.to_le_bytes()[..], &[0, 0, 3, 0], b"Old"].concat()), rec(0x000A, &[])].concat()
    };
    let stream = [globals(globals(0).len() as u32), sheet].concat();
    gridcraft_xlsx::testing::compound_file(&[("Workbook", &stream)])
}

#[test]
fn desktop_legacy_open_shows_one_dialog_and_save_proposes_xlsx() {
    let dir = std::env::temp_dir().join(format!("gridcraft-legacy-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Old.xls");
    let original = legacy_bytes();
    std::fs::write(&path, &original).unwrap();
    let suggested: Arc<Mutex<Vec<String>>> = Default::default();
    let captured = suggested.clone();
    let mut h = harness(Services {
        pick_save: Some(Box::new(move |name| {
            captured.lock().unwrap().push(name.to_string());
            None
        })),
        ..Default::default()
    });
    h.state_mut().open_path(&path.to_string_lossy());
    h.run_steps(3);
    assert_eq!(h.state().message.as_ref().map(|(_, body)| body.as_str()), Some(legacy::OPENED));
    assert!(h.query_by_label(Language::Vi.tr(legacy::OPENED)).is_some());
    assert!(h.state().session.active().unwrap().path.is_none());
    assert_eq!(h.state().session.active().unwrap().display_title(), "Old (imported).xlsx");
    h.get_by_label("  OK  ").click();
    h.run_steps(3);
    assert!(h.state().message.is_none());
    h.state_mut().run("file.save", json!({})).unwrap();
    h.state_mut().open_dialog("saveAs", json!({}));
    assert_eq!(*suggested.lock().unwrap(), ["Old (imported).xlsx", "Old (imported).xlsx"]);
    assert!(h.state_mut().run("file.save", json!({"path": path.to_string_lossy()})).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn web_ole2_open_ignores_extension_and_downloads_xlsx() {
    let inbox: Inbox = Default::default();
    let mut h = harness(Services { inbox: Some(inbox.clone()), ..Default::default() });
    inbox.lock().unwrap().push(("Disguised.csv".into(), legacy_bytes()));
    h.run_steps(4);
    assert!(h.query_by_label(Language::Vi.tr(legacy::OPENED)).is_some());
    assert!(h.state().session.active().unwrap().path.is_none());
    let saved = h.state_mut().run("file.saveBytes", json!({})).unwrap();
    assert_eq!(saved["name"], "Disguised (imported).xlsx");
    let bytes = gridcraft_engine::io::base64_decode(saved["base64"].as_str().unwrap()).unwrap();
    assert!(bytes.starts_with(b"PK"));
}

#[test]
fn refused_ole2_shows_clear_dialog_instead_of_generic_error() {
    let inbox: Inbox = Default::default();
    let mut h = harness(Services { inbox: Some(inbox.clone()), ..Default::default() });
    inbox.lock().unwrap().push(("Broken.xlsx".into(), vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]));
    h.run_steps(4);
    assert_eq!(h.state().message.as_ref(), Some(&(legacy::FAILED_TITLE.to_string(), legacy::FAILED.to_string())));
    assert!(h.query_by_label(Language::Vi.tr(legacy::FAILED_TITLE)).is_some());
    assert!(h.query_by_label(Language::Vi.tr(legacy::FAILED)).is_some());
    assert_eq!(h.state().session.documents().len(), 1);
    assert!(h.state().dialog.is_none());
    h.get_by_label("  OK  ").click();
    h.run_steps(2);
    assert!(h.state().message.is_none());
}
