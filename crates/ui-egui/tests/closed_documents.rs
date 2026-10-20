//! Reopen Closed Document (the tab strip's menu, File › Open) and closing several
//! documents at once (Close All / Close Others, quitting) over the real app.

use egui::{Event, Key, Modifiers, PointerButton};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::{MAX_DOCUMENTS, Session};
use gridcraft_ui_egui::closed_docs::{MAX_CLOSED, NOTHING_TO_REOPEN};
use gridcraft_ui_egui::i18n::Language;
use gridcraft_ui_egui::{Delivery, DeliveryReport, Services, SheetApp, backstage, keymap};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

const CTRL_SHIFT: Modifiers = Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: true };

fn xlsx() -> Vec<u8> {
    let mut s = Session::new();
    s.new_workbook();
    s.run("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    gridcraft_ui_egui::drafts::draft_bytes(&s.active().unwrap().wb, "Book1.xlsx").unwrap()
}

fn session(names: &[&str]) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    let b64 = gridcraft_engine::io::base64_encode(&xlsx());
    for name in names {
        s.run("file.open", json!({"name": name, "base64": b64})).unwrap();
    }
    s
}

fn harness_with(session: Session, services: Services) -> Harness {
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

fn harness(names: &[&str]) -> Harness {
    harness_with(session(names), Services::default())
}

fn titles(h: &Harness) -> Vec<String> {
    h.state().session.documents().iter().map(|d| d.display_title()).collect()
}

fn edit(h: &mut Harness, index: usize, text: &str) {
    let back = h.state().session.active_index();
    h.state_mut().run("window.activate", json!({"index": index})).unwrap();
    h.state_mut().run("cell.set", json!({"cell": "B2", "input": text})).unwrap();
    h.state_mut().run("window.activate", json!({"index": back})).unwrap();
}

fn b2(h: &Harness, title: &str) -> Value {
    let d = h.state().session.documents().iter().find(|d| d.display_title() == title).unwrap();
    d.wb.sheet(0).unwrap().value(CellRef::new(1, 1))
}

fn press(h: &mut Harness, k: Key, m: Modifiers) {
    h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m });
    h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: m });
    h.run_steps(3);
}

fn reopen(h: &mut Harness) {
    h.state_mut().run_or_alert("file.reopenClosed", json!({}));
    h.run_steps(3);
}

#[test]
fn reopen_restores_content_name_and_unsaved_state() {
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"]);
    edit(&mut h, 0, "draft only");
    h.get_by_label("Close Alpha.xlsx").click();
    h.run_steps(3);
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert_eq!(titles(&h), ["Beta.xlsx", "Gamma.xlsx"]);
    h.get_by_label("Close Beta.xlsx").click();
    h.run_steps(3);
    assert_eq!(titles(&h), ["Gamma.xlsx"]);

    reopen(&mut h);
    assert_eq!(titles(&h), ["Gamma.xlsx", "Beta.xlsx"], "the most recent first");
    let beta = h.state().session.doc().unwrap();
    assert_eq!(beta.display_title(), "Beta.xlsx");
    assert!(!beta.is_dirty(), "a saved document reopens saved");

    reopen(&mut h);
    let alpha = h.state().session.doc().unwrap();
    assert_eq!(alpha.display_title(), "Alpha.xlsx");
    assert!(alpha.is_dirty(), "closed with Don't Save: its unsaved changes come back unsaved");
    assert_eq!(b2(&h, "Alpha.xlsx"), Value::Text("draft only".into()));

    assert!(!h.state().can_reopen_closed());
    h.state_mut().run_or_alert("file.reopenClosed", json!({}));
    assert_eq!(h.state().message.as_ref().map(|(_, m)| m.as_str()), Some(NOTHING_TO_REOPEN));
    assert_ne!(Language::Vi.tr(NOTHING_TO_REOPEN), NOTHING_TO_REOPEN);
}

#[test]
fn reopening_respects_the_open_document_limit() {
    let names: Vec<String> = (1..=MAX_DOCUMENTS).map(|i| format!("Book {i}.xlsx")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut h = harness(&names);
    h.state_mut().run("file.close", json!({"index": 0})).unwrap();
    h.state_mut().run("file.new", json!({})).unwrap();
    assert_eq!(h.state().session.documents().len(), MAX_DOCUMENTS);
    assert!(h.state_mut().run("file.reopenClosed", json!({})).is_err(), "ten open: refused");
    assert!(h.state().can_reopen_closed(), "and kept for later");
    h.state_mut().run("file.close", json!({})).unwrap();
    h.state_mut().run("file.reopenClosed", json!({})).unwrap();
    assert_eq!(h.state().session.doc().unwrap().display_title(), "Book1");
}

#[test]
fn at_most_ten_closed_documents_are_kept() {
    let names: Vec<String> = (1..=MAX_DOCUMENTS).map(|i| format!("Book {i}.xlsx")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut h = harness(&names);
    for _ in 0..MAX_DOCUMENTS {
        h.state_mut().run("file.close", json!({"index": 0})).unwrap();
    }
    h.run_steps(2);
    h.state_mut().run("file.close", json!({})).unwrap();
    assert_eq!(h.state().closed_docs.len(), MAX_CLOSED);
    assert_eq!(h.state().last_closed_title(), Some("Book1"), "the blank workbook made after the last one closed");
}

#[test]
fn a_workbook_opened_with_its_password_is_not_kept() {
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx"]);
    let plain = gridcraft_engine::io::save_bytes(&Session::new().blank_workbook().wb, "book.xlsx").unwrap();
    let bytes = gridcraft_xlsx::testing::encrypt_workbook(&plain, "secret");
    h.state_mut()
        .run("file.open", json!({"name": "Secret.xlsx", "base64": gridcraft_engine::io::base64_encode(&bytes), "password": "secret"}))
        .unwrap();
    h.run_steps(2);
    h.state_mut().run("file.close", json!({})).unwrap();
    assert!(!h.state().can_reopen_closed(), "never kept decrypted");
}

#[test]
fn the_tab_menu_reopens_and_closes_others_or_all() {
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"]);
    h.get_by_label("Beta.xlsx").click_button(PointerButton::Secondary);
    h.run_steps(3);
    assert!(h.get_by_label("Reopen Closed Document").accesskit_node().is_disabled(), "nothing closed yet: disabled");
    h.get_by_label("Close Others").click();
    h.run_steps(3);
    assert_eq!(titles(&h), ["Beta.xlsx"], "all clean: closed without asking");
    h.state_mut().run("file.new", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("Beta.xlsx").click_button(PointerButton::Secondary);
    h.run_steps(3);
    h.get_by_label("Reopen Closed Document").click();
    h.run_steps(3);
    assert_eq!(titles(&h), ["Beta.xlsx", "Book1", "Gamma.xlsx"]);
    h.get_by_label("Book1").click_button(PointerButton::Secondary);
    h.run_steps(3);
    h.get_by_label("Close All").click();
    h.run_steps(4);
    assert_eq!(titles(&h), ["Book1"], "a fresh blank workbook replaces the closed ones");
    assert_eq!(h.state().closed_docs.len(), 4, "Alpha left after Gamma reopened, then Beta, Book1 and Gamma");
}

#[test]
fn file_open_offers_reopen_and_returns_to_the_sheet() {
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx"]);
    h.state_mut().open_backstage(backstage::Page::Open);
    h.run_steps(3);
    assert!(h.get_by_label("Reopen Closed Document").accesskit_node().is_disabled(), "disabled when nothing was closed");
    h.state_mut().backstage = None;
    h.state_mut().run("file.close", json!({"index": 0})).unwrap();
    h.state_mut().open_backstage(backstage::Page::Open);
    h.run_steps(3);
    h.get_by_label("Reopen Closed Document").click();
    h.run_steps(3);
    assert!(h.state().backstage.is_none());
    assert_eq!(titles(&h), ["Beta.xlsx", "Alpha.xlsx"]);
}

#[test]
fn ctrl_shift_t_stays_autosum_and_reopen_has_no_key() {
    let e = keymap::reference().iter().find(|e| e.keys() == "Ctrl+Shift+T").unwrap();
    assert_eq!(e.label, "AutoSum");
    assert!(!keymap::reference().iter().any(|e| e.label == "Reopen Closed Document"));
    assert_eq!(Language::Vi.tr("Reopen Closed Document"), "Mở lại tài liệu vừa đóng");
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx"]);
    h.state_mut().run("file.close", json!({"index": 0})).unwrap();
    h.run_steps(3);
    press(&mut h, Key::T, CTRL_SHIFT);
    assert_eq!(titles(&h), ["Beta.xlsx"], "the key is Excel's AutoSum, not a browser tab's");
    assert!(h.state().can_reopen_closed());
}

// ---------------------------------------------------------------- closing several at once

#[test]
fn one_dialog_lists_every_unsaved_document_and_cancel_closes_nothing() {
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"]);
    edit(&mut h, 0, "a");
    edit(&mut h, 2, "c");
    h.run_steps(2);
    h.state_mut().close_all();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("closeMany"));
    assert!(h.query_by_label_contains("these 2 documents").is_some());
    assert!(h.query_by_label("• Alpha.xlsx").is_some() && h.query_by_label("• Gamma.xlsx").is_some());
    assert!(h.query_by_label("• Beta.xlsx").is_none(), "only the unsaved ones are listed");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert_eq!(titles(&h), ["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"], "Cancel closes nothing, not even the saved ones");
}

#[test]
fn dont_save_closes_them_all_and_keeps_them_for_reopening() {
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"]);
    edit(&mut h, 0, "a");
    edit(&mut h, 1, "b");
    let gamma = h.state().session.documents()[2].uid;
    h.state_mut().close_others(gamma);
    h.run_steps(3);
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert_eq!(titles(&h), ["Gamma.xlsx"]);
    assert_eq!(h.state().closed_docs.len(), 2);
    assert!(h.state().closed_docs.latest().unwrap().unsaved);
}

#[test]
fn save_all_saves_each_then_closes_on_the_desktop() {
    let dir = std::env::temp_dir().join(format!("gridcraft-close-many-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let picks = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = picks.clone();
    let folder = dir.clone();
    let services = Services {
        pick_save: Some(Box::new(move |name: &str| {
            seen.lock().unwrap().push(name.to_string());
            Some(folder.join(name).to_string_lossy().to_string())
        })),
        ..Default::default()
    };
    let mut h = harness_with(session(&["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"]), services);
    edit(&mut h, 0, "a");
    edit(&mut h, 2, "c");
    h.state_mut().close_all();
    h.run_steps(3);
    h.get_by_label("Save All").click();
    h.run_steps(4);
    assert_eq!(*picks.lock().unwrap(), ["Alpha.xlsx", "Gamma.xlsx"], "one picker per unsaved document, in order");
    assert_eq!(titles(&h), ["Book1"], "all closed");
    let mut check = Session::new();
    check.run("file.open", json!({"path": dir.join("Gamma.xlsx").to_string_lossy()})).unwrap();
    assert_eq!(check.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::new(1, 1)), Value::Text("c".into()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_cancelled_picker_stops_save_all_and_closes_nothing() {
    let dir = std::env::temp_dir().join(format!("gridcraft-close-cancel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let folder = dir.clone();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    // Saves the first, cancels the second: the third is never asked about.
    let services = Services {
        pick_save: Some(Box::new(move |name: &str| {
            let n = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (n == 0).then(|| folder.join(name).to_string_lossy().to_string())
        })),
        ..Default::default()
    };
    let mut h = harness_with(session(&["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx", "Delta.xlsx"]), services);
    edit(&mut h, 0, "a");
    edit(&mut h, 1, "b");
    edit(&mut h, 2, "c");
    h.state_mut().close_all();
    h.run_steps(3);
    h.get_by_label("Save All").click();
    h.run_steps(4);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(titles(&h), ["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx", "Delta.xlsx"], "nothing closes");
    let dirty: Vec<bool> = h.state().session.documents().iter().map(|d| d.is_dirty()).collect();
    assert_eq!(dirty, [false, true, true, false], "Alpha was saved");
    assert!(!h.state().saving_all());
    assert!(h.state().dialog.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

type Reports = std::sync::Arc<std::sync::Mutex<Vec<(String, DeliveryReport)>>>;

/// The web with a save picker: each download is answered later, when the test settles it.
fn web_with_pickers(names: &[&str]) -> (Harness, Reports) {
    let reports: Reports = Default::default();
    let started = reports.clone();
    let services = Services {
        download: Some(Box::new(move |name: &str, _bytes: &[u8], report: DeliveryReport| started.lock().unwrap().push((name.to_string(), report)))),
        ..Default::default()
    };
    let mut h = harness_with(session(names), services);
    h.state_mut().ui.language = Language::Vi;
    h.run_steps(2);
    (h, reports)
}

fn picked(r: &Reports) -> Vec<String> {
    r.lock().unwrap().iter().map(|(n, _)| n.clone()).collect()
}

fn enabled(h: &Harness, label: &str) -> bool {
    !h.get_by_label(label).accesskit_node().is_disabled()
}

/// The web: a save picker needs a click of its own, so after each one the dialog stays up and
/// the next click saves the next document; they close once all are saved.
#[test]
fn save_all_on_the_web_takes_one_click_per_picker() {
    let (mut h, reports) = web_with_pickers(&["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"]);
    edit(&mut h, 0, "a");
    edit(&mut h, 1, "b");
    edit(&mut h, 2, "c");
    h.state_mut().close_all();
    h.run_steps(3);
    h.get_by_label("Lưu tất cả").click();
    h.run_steps(3);
    assert_eq!(picked(&reports), ["Alpha.xlsx"], "one picker at a time");
    assert!(h.query_by_label("• Alpha.xlsx — Đang lưu…").is_some());
    assert!(!enabled(&h, "Lưu “Beta.xlsx”") && !enabled(&h, "Không Lưu"), "nothing else starts while a picker is open");
    assert!(enabled(&h, "Hủy"));
    assert!(h.query_by_label("Đang lưu…").is_some(), "the status bar says so");

    reports.lock().unwrap()[0].1.settle(Delivery::Delivered);
    h.run_steps(3);
    assert_eq!(picked(&reports), ["Alpha.xlsx"], "the next picker waits for a click");
    assert!(h.query_by_label("• Alpha.xlsx — Đã lưu").is_some());
    assert!(enabled(&h, "Lưu “Beta.xlsx”"));
    assert_eq!(titles(&h), ["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"], "nothing closes before all are saved");
    h.get_by_label("Lưu “Beta.xlsx”").click();
    h.run_steps(3);
    assert_eq!(picked(&reports), ["Alpha.xlsx", "Beta.xlsx"], "the click opened the next picker");
    reports.lock().unwrap()[1].1.settle(Delivery::Delivered);
    h.run_steps(3);
    h.get_by_label("Lưu “Gamma.xlsx”").click();
    h.run_steps(3);
    reports.lock().unwrap()[2].1.settle(Delivery::Delivered);
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert!(!h.state().saving_all());
    assert_eq!(titles(&h), ["Book1"], "all closed once saved");
}

/// The web: Cancel in the second picker stops there and closes nothing.
#[test]
fn cancel_in_the_second_picker_closes_nothing() {
    let (mut h, reports) = web_with_pickers(&["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"]);
    edit(&mut h, 0, "a");
    edit(&mut h, 1, "b");
    edit(&mut h, 2, "c");
    h.state_mut().close_all();
    h.run_steps(3);
    h.get_by_label("Lưu tất cả").click();
    h.run_steps(3);
    reports.lock().unwrap()[0].1.settle(Delivery::Delivered);
    h.run_steps(3);
    h.get_by_label("Lưu “Beta.xlsx”").click();
    h.run_steps(3);
    reports.lock().unwrap()[1].1.settle(Delivery::Cancelled);
    h.run_steps(3);
    assert!(h.state().dialog.is_none(), "the dialog goes");
    assert!(!h.state().saving_all());
    assert_eq!(picked(&reports).len(), 2, "no further picker");
    assert_eq!(titles(&h), ["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"], "nothing closed");
    let dirty: Vec<bool> = h.state().session.documents().iter().map(|d| d.is_dirty()).collect();
    assert_eq!(dirty, [false, true, true]);

    // A failed write in the first picker: the same.
    h.state_mut().close_all();
    h.run_steps(3);
    h.get_by_label("Lưu tất cả").click();
    h.run_steps(3);
    assert_eq!(picked(&reports).len(), 3);
    reports.lock().unwrap()[2].1.settle(Delivery::Failed("disk full".into()));
    h.run_steps(3);
    assert!(h.state().dialog.is_none() && !h.state().saving_all());
    assert_eq!(picked(&reports).len(), 3, "no further picker");
    assert_eq!(titles(&h).len(), 3, "nothing closed");
}

/// The web: Cancel in the dialog while a picker is open stops the rest; that picker's answer
/// still saves its document but closes nothing.
#[test]
fn cancel_in_the_dialog_while_a_picker_is_open_stops_the_rest() {
    let (mut h, reports) = web_with_pickers(&["Alpha.xlsx", "Beta.xlsx"]);
    edit(&mut h, 0, "a");
    edit(&mut h, 1, "b");
    h.state_mut().close_all();
    h.run_steps(3);
    h.get_by_label("Lưu tất cả").click();
    h.run_steps(3);
    h.get_by_label("Hủy").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none() && !h.state().saving_all());
    reports.lock().unwrap()[0].1.settle(Delivery::Delivered);
    h.run_steps(3);
    assert_eq!(picked(&reports), ["Alpha.xlsx"], "no further picker");
    assert_eq!(titles(&h), ["Alpha.xlsx", "Beta.xlsx"]);
    assert!(!h.state().session.documents()[0].is_dirty(), "the open picker still saved Alpha");
}

/// Browsers without a save picker download at once: one click saves them all.
#[test]
fn plain_downloads_save_all_in_one_click() {
    let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = started.clone();
    let services = Services {
        download: Some(Box::new(move |_: &str, _: &[u8], report: DeliveryReport| {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            report.settle(Delivery::Delivered);
        })),
        ..Default::default()
    };
    let mut h = harness_with(session(&["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"]), services);
    edit(&mut h, 0, "a");
    edit(&mut h, 2, "c");
    h.state_mut().close_all();
    h.run_steps(3);
    h.get_by_label("Save All").click();
    h.run_steps(3);
    assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(h.state().dialog.is_none());
    assert_eq!(titles(&h), ["Book1"]);
}

#[test]
fn quitting_asks_once_and_dont_save_lets_the_window_close() {
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx"]);
    assert!(h.state_mut().confirm_quit(), "nothing unsaved: quits at once");
    h.state_mut().quit_confirmed = false;
    edit(&mut h, 0, "a");
    edit(&mut h, 1, "b");
    assert!(!h.state_mut().confirm_quit());
    h.run_steps(3);
    assert!(h.query_by_label_contains("these 2 documents").is_some());
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert!(h.state_mut().confirm_quit());
    assert_eq!(titles(&h).len(), 2, "quitting doesn't close the documents one by one");
}
