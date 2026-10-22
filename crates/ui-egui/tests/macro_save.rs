//! A macro-enabled workbook keeps its VBA project when saved: Save stays `.xlsm`, Save As offers
//! `.xlsm`, a plain `.xlsx` asks first, and drafts keep the macros for recovery.

use std::sync::{Arc, Mutex};

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::io::{base64_encode, open_bytes, save_bytes};
use gridcraft_ui_egui::i18n::Language;
use gridcraft_ui_egui::save_as::{DROP_MACROS, SAVE_AS_XLSM, SAVE_AS_XLSX};
use gridcraft_ui_egui::{Delivery, DeliveryReport, Services, SheetApp};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;
type Downloads = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// Arbitrary bytes standing in for a VBA project; they must come back unchanged.
const VBA: &[u8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1\x00\xff synthetic vbaProject \x80\x81";

fn xlsm_bytes() -> Vec<u8> {
    let mut wb = gridcraft_engine::model::Workbook::new();
    wb.vba_project = Some(VBA.to_vec());
    save_bytes(&wb, "macro.xlsm").unwrap()
}

fn harness(services: Services) -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, services),
    );
    h.run_steps(4);
    h
}

/// The web build: every download is delivered at once and kept for the test.
fn web() -> (Harness, Downloads) {
    let downloads: Downloads = Default::default();
    let kept = downloads.clone();
    let services = Services {
        download: Some(Box::new(move |name: &str, bytes: &[u8], report: DeliveryReport| {
            kept.lock().unwrap().push((name.to_string(), bytes.to_vec()));
            report.settle(Delivery::Delivered);
        })),
        ..Default::default()
    };
    (harness(services), downloads)
}

/// Opens `macro.xlsm` as the browser does (bytes, no path) and edits it.
fn open_macro_workbook(h: &mut Harness) {
    let r = h.state_mut().run("file.open", json!({"name": "macro.xlsm", "base64": base64_encode(&xlsm_bytes())})).unwrap();
    h.state_mut().after_open(None, &r);
    // The open warnings' message box, which the user would dismiss with OK.
    h.state_mut().message = None;
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "edited"})).unwrap();
    h.run_steps(2);
}

fn vba_of(name: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    open_bytes(name, bytes).unwrap().0.vba_project
}

fn dialog(h: &Harness) -> Option<String> {
    h.state().dialog.as_ref().map(|d| d.name().to_string())
}

fn click(h: &mut Harness, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

#[test]
fn ctrl_s_on_the_web_keeps_an_xlsm_with_its_vba_project() {
    let (mut h, downloads) = web();
    open_macro_workbook(&mut h);
    let ctrl = Modifiers::COMMAND;
    h.input_mut().events.push(Event::Key { key: Key::S, physical_key: None, pressed: true, repeat: false, modifiers: ctrl });
    h.input_mut().events.push(Event::Key { key: Key::S, physical_key: None, pressed: false, repeat: false, modifiers: ctrl });
    h.run_steps(3);
    assert_eq!(dialog(&h), None, "keeping the kind asks nothing");
    let saved = downloads.lock().unwrap().clone();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].0, "macro.xlsm");
    assert_eq!(vba_of(&saved[0].0, &saved[0].1).as_deref(), Some(VBA), "the VBA project is saved byte for byte");
    assert!(!h.state().session.active().unwrap().is_dirty());
}

#[test]
fn saving_as_xlsx_asks_first_on_the_web() {
    let (mut h, downloads) = web();
    open_macro_workbook(&mut h);

    h.state_mut().open_dialog("saveAs", json!({"format": "xlsx"}));
    h.run_steps(3);
    assert_eq!(dialog(&h).as_deref(), Some(DROP_MACROS));
    assert!(downloads.lock().unwrap().is_empty(), "nothing is saved before the answer");
    click(&mut h, "Cancel");
    assert_eq!(dialog(&h), None);
    assert!(downloads.lock().unwrap().is_empty() && h.state().session.active().unwrap().is_dirty());

    h.state_mut().open_dialog("saveAs", json!({"format": "xlsx"}));
    h.run_steps(3);
    click(&mut h, SAVE_AS_XLSM);
    let (name, bytes) = downloads.lock().unwrap().pop().unwrap();
    assert_eq!((name.as_str(), vba_of(&name, &bytes).as_deref()), ("macro.xlsm", Some(VBA)));

    h.state_mut().open_dialog("saveAs", json!({"format": "xlsx"}));
    h.run_steps(3);
    click(&mut h, SAVE_AS_XLSX);
    let (name, bytes) = downloads.lock().unwrap().pop().unwrap();
    assert_eq!(name, "macro.xlsx");
    assert_eq!(vba_of(&name, &bytes), None, "the user chose to leave the macros out");
}

#[test]
fn the_desktop_picker_offers_xlsm_and_a_picked_xlsx_asks_first() {
    let dir = std::env::temp_dir().join(format!("gridcraft-macro-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let asked = Arc::new(Mutex::new(Vec::<String>::new()));
    let (seen, folder) = (asked.clone(), dir.clone());
    // The user types a plain .xlsx name first; offered a .xlsm name, they keep it.
    let services = Services {
        pick_save: Some(Box::new(move |suggested: &str| {
            seen.lock().unwrap().push(suggested.to_string());
            let name = if suggested.ends_with("Out.xlsm") { "Out.xlsm" } else { "Out.xlsx" };
            Some(folder.join(name).to_string_lossy().into_owned())
        })),
        ..Default::default()
    };
    let mut h = harness(services);
    open_macro_workbook(&mut h);
    h.state_mut().open_dialog("saveAs", json!({}));
    h.run_steps(3);
    assert_eq!(asked.lock().unwrap().first().map(String::as_str), Some("macro.xlsm"), "the picker suggests the workbook's own kind");
    assert_eq!(dialog(&h).as_deref(), Some(DROP_MACROS), "a picked .xlsx asks first");
    assert!(!dir.join("Out.xlsx").exists());
    click(&mut h, SAVE_AS_XLSM);
    assert!(asked.lock().unwrap().last().is_some_and(|s| s.ends_with("Out.xlsm")), "{:?}", asked.lock().unwrap());
    let written = std::fs::read(dir.join("Out.xlsm")).unwrap();
    assert_eq!(vba_of("Out.xlsm", &written).as_deref(), Some(VBA));
    let d = h.state().session.active().unwrap();
    assert!(d.path.as_deref().is_some_and(|p| p.ends_with("Out.xlsm")) && !d.is_dirty());

    h.state_mut().open_dialog("saveAs", json!({"format": "xlsx"}));
    h.run_steps(3);
    click(&mut h, SAVE_AS_XLSX);
    let written = std::fs::read(dir.join("Out.xlsx")).unwrap();
    assert_eq!(vba_of("Out.xlsx", &written), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_question_and_the_notice_are_in_vietnamese() {
    let (mut h, _) = web();
    h.state_mut().ui.language = Language::Vi;
    open_macro_workbook(&mut h);
    let notice = h.state().notice.clone().unwrap();
    assert_eq!(notice.text, gridcraft_ui_egui::VBA_NOTICE);
    let shown = Language::Vi.tr(&notice.text);
    let expected = if cfg!(target_arch = "wasm32") { "Bảng tính không chạy macro" } else { "cho phép chạy các macro được hỗ trợ" };
    assert!(shown.contains(expected) && shown.contains(".xlsm"), "{shown}");
    assert!(h.query_by_label(shown).is_some(), "the info bar shows the Vietnamese text");
    h.state_mut().open_dialog("saveAs", json!({"format": "xlsx"}));
    h.run_steps(3);
    for label in ["Lưu dạng .xlsm", "Vẫn lưu dạng .xlsx", "Hủy"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    let kept = Language::Vi.tr_message("macros (VBA project) were kept but not executed");
    assert_eq!(kept, "Đã giữ lại macro (dự án VBA) nhưng không chạy");
}

#[test]
fn save_as_starts_on_xlsm_only_for_a_workbook_with_macros() {
    let (mut h, _) = web();
    h.state_mut().open_backstage(gridcraft_ui_egui::backstage::Page::SaveAs);
    assert_eq!(h.state().backstage.as_ref().unwrap().save_format, "xlsx");
    h.state_mut().backstage = None;
    open_macro_workbook(&mut h);
    h.state_mut().open_backstage(gridcraft_ui_egui::backstage::Page::SaveAs);
    h.run_steps(3);
    assert_eq!(h.state().backstage.as_ref().unwrap().save_format, "xlsm");
}

#[test]
fn a_draft_of_a_macro_workbook_recovers_its_vba_project() {
    let (mut h, _) = web();
    open_macro_workbook(&mut h);
    let docs = h.state().draft_docs();
    assert_eq!(docs[0].name, "macro.xlsm");
    assert_eq!(gridcraft_ui_egui::drafts::draft_format(&docs[0].name), "xlsm");
    let bytes = gridcraft_ui_egui::drafts::draft_bytes(&docs[0].wb, &docs[0].name).unwrap();

    let (mut other, downloads) = web();
    let uid = other.state_mut().open_draft(&docs[0].name, &bytes).unwrap();
    let d = other.state().session.active().unwrap();
    assert_eq!((d.uid, d.display_title(), d.wb.vba_project.as_deref()), (uid, "macro.xlsm".to_string(), Some(VBA)));
    other.state_mut().run_or_alert("file.save", json!({}));
    other.run_steps(2);
    let (name, saved) = downloads.lock().unwrap().pop().unwrap();
    assert_eq!((name.as_str(), vba_of(&name, &saved).as_deref()), ("macro.xlsm", Some(VBA)));
}
