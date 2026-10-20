//! The document tab strip over the real app: shown from two open documents, switching, closing
//! (with the unsaved-changes prompt) and the open-document limit.

use egui::PointerButton;
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::{MAX_DOCUMENTS, Session, TOO_MANY_DOCUMENTS};
use gridcraft_ui_egui::i18n::Language;
use gridcraft_ui_egui::{Inbox, Services, SheetApp, doc_tabs};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

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

fn harness_with(session: Session, services: Services, size: egui::Vec2) -> Harness {
    let mut h = egui_kittest::Harness::builder().with_size(size).build_ui_state(
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
    harness_with(session(names), Services::default(), egui::vec2(1200.0, 800.0))
}

fn titles(h: &Harness) -> Vec<String> {
    h.state().session.documents().iter().map(|d| d.display_title()).collect()
}

fn active_title(h: &Harness) -> String {
    h.state().session.doc().unwrap().display_title()
}

#[test]
fn the_strip_shows_from_two_documents() {
    let mut h = harness(&[]);
    assert!(doc_tabs::tabs(&h.state().session).is_empty());
    assert!(h.query_by_label("Close Book1").is_none(), "one document looks as before: no tab strip");
    h.state_mut().run("file.new", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Close Book1").is_some() && h.query_by_label("Close Book2").is_some());
    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Close Book1").is_none(), "back to one document: the strip goes away");
}

#[test]
fn clicking_a_tab_switches_through_window_activate() {
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx"]);
    assert_eq!(titles(&h), ["Alpha.xlsx", "Beta.xlsx"], "the first open replaced the blank Book1");
    assert_eq!(active_title(&h), "Beta.xlsx");
    h.get_by_label("Alpha.xlsx").click();
    h.run_steps(3);
    assert_eq!(active_title(&h), "Alpha.xlsx");
    assert!(h.state().session.journal.iter().any(|(id, p)| id == "window.activate" && p["index"] == 0));
}

#[test]
fn switching_tabs_commits_the_cell_being_edited_to_its_own_workbook() {
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx"]);
    h.state_mut().begin_edit(Some("typed".into()), false);
    h.run_steps(2);
    h.get_by_label("Alpha.xlsx").click();
    h.run_steps(3);
    assert_eq!(active_title(&h), "Alpha.xlsx");
    let beta = &h.state().session.documents()[1];
    let a1 = beta.wb.sheet(0).unwrap().value(gridcraft_engine::core::CellRef::new(0, 0));
    assert_eq!(a1, gridcraft_engine::core::Value::Text("typed".into()));
    assert!(h.state().editor.is_none());
}

#[test]
fn the_close_button_closes_its_own_document_and_keeps_the_active_one() {
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"]);
    h.get_by_label("Beta.xlsx").click();
    h.run_steps(3);
    h.get_by_label("Close Alpha.xlsx").click();
    h.run_steps(3);
    assert_eq!(titles(&h), ["Beta.xlsx", "Gamma.xlsx"]);
    assert_eq!(active_title(&h), "Beta.xlsx", "closing another tab doesn't switch away");
    h.get_by_label("Gamma.xlsx").click_button(PointerButton::Middle);
    h.run_steps(3);
    assert_eq!(titles(&h), ["Beta.xlsx"], "a middle click closes the tab");
    assert!(h.query_by_label("Close Beta.xlsx").is_none());
}

#[test]
fn closing_a_dirty_tab_asks_and_dont_save_closes_that_one() {
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx"]);
    h.state_mut().run("window.activate", json!({"index": 0})).unwrap();
    h.state_mut().run("cell.set", json!({"cell": "B2", "input": "unsaved"})).unwrap();
    h.state_mut().run("window.activate", json!({"index": 1})).unwrap();
    h.run_steps(3);
    h.get_by_label("Close Alpha.xlsx").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("saveChanges"));
    assert_eq!(titles(&h).len(), 2, "nothing closes before the answer");
    assert_eq!(active_title(&h), "Beta.xlsx");
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert_eq!(titles(&h), ["Beta.xlsx"]);
    assert!(!h.state().session.doc().unwrap().is_dirty(), "Beta itself was left alone");
}

#[test]
fn saving_from_the_prompt_saves_and_closes_the_tab_being_closed() {
    let dir = std::env::temp_dir().join(format!("gridcraft-doc-tabs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("Alpha saved.xlsx");
    let picked = target.to_string_lossy().to_string();
    let services = Services { pick_save: Some(Box::new(move |_| Some(picked.clone()))), ..Default::default() };
    let mut h = harness_with(session(&["Alpha.xlsx", "Beta.xlsx"]), services, egui::vec2(1200.0, 800.0));
    h.state_mut().run("window.activate", json!({"index": 0})).unwrap();
    h.state_mut().run("cell.set", json!({"cell": "B2", "input": "keep me"})).unwrap();
    h.state_mut().run("window.activate", json!({"index": 1})).unwrap();
    h.run_steps(3);
    h.get_by_label("Close Alpha.xlsx").click();
    h.run_steps(3);
    h.get_by_label("Save").click();
    h.run_steps(3);
    assert_eq!(titles(&h), ["Beta.xlsx"], "Alpha was saved, then closed");
    assert_eq!(active_title(&h), "Beta.xlsx");
    let mut check = Session::new();
    check.run("file.open", json!({"path": target.to_string_lossy()})).unwrap();
    let wb = &check.doc().unwrap().wb;
    assert_eq!(wb.sheet(0).unwrap().value(gridcraft_engine::core::CellRef::new(1, 1)), gridcraft_engine::core::Value::Text("keep me".into()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_eleventh_document_is_refused_with_a_translated_message() {
    let inbox: Inbox = Default::default();
    let names: Vec<String> = (1..=MAX_DOCUMENTS).map(|i| format!("Book {i}.xlsx")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut h = harness_with(session(&names), Services { inbox: Some(inbox.clone()), ..Default::default() }, egui::vec2(1200.0, 800.0));
    h.state_mut().ui.language = Language::Vi;
    assert_eq!(titles(&h).len(), MAX_DOCUMENTS);
    let vi = Language::Vi.tr(TOO_MANY_DOCUMENTS);
    assert_ne!(vi, TOO_MANY_DOCUMENTS, "the refusal is translated");

    h.state_mut().run_or_alert("file.new", json!({}));
    h.run_steps(2);
    assert_eq!(h.state().message.as_ref().map(|(_, m)| Language::Vi.tr(m)), Some(vi), "File › New");
    h.state_mut().message = None;

    inbox.lock().unwrap().push(("Dropped.xlsx".into(), xlsx()));
    h.run_steps(2);
    assert_eq!(h.state().message.as_ref().map(|(_, m)| m.as_str()), Some(TOO_MANY_DOCUMENTS), "a dropped or picked file");
    h.state_mut().message = None;

    assert_eq!(h.state_mut().open_draft("Recovered.xlsx", &xlsx()), None, "a recovered draft");
    assert_eq!(h.state().message.as_ref().map(|(_, m)| m.as_str()), Some(TOO_MANY_DOCUMENTS));
    h.state_mut().message = None;

    h.state_mut().run_or_alert("view.newWindow", json!({}));
    assert!(h.state().message.is_some(), "View › New Window");
    assert_eq!(titles(&h).len(), MAX_DOCUMENTS);
    assert_eq!(doc_tabs::tabs(&h.state().session).len(), MAX_DOCUMENTS);
}

#[test]
fn a_narrow_window_scrolls_the_active_tab_into_view() {
    let names: Vec<String> = (1..=MAX_DOCUMENTS).map(|i| format!("Book {i}.xlsx")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut h = harness_with(session(&names), Services::default(), egui::vec2(700.0, 500.0));
    h.run_steps(4);
    let last = h.get_by_label("Close Book 10.xlsx").rect();
    assert!(last.right() <= 700.0, "the active (last) tab is scrolled into view: {last:?}");
    h.state_mut().run("window.activate", json!({"index": 0})).unwrap();
    h.run_steps(4);
    let first = h.get_by_label("Close Book 1.xlsx").rect();
    assert!(first.left() >= 0.0 && first.right() <= 700.0, "and back to the first: {first:?}");
}

#[test]
fn several_restored_drafts_each_get_a_tab() {
    let mut h = harness(&[]);
    let a = h.state_mut().open_draft("Budget.xlsx", &xlsx()).unwrap();
    let b = h.state_mut().open_draft("Plan.xlsx", &xlsx()).unwrap();
    h.run_steps(3);
    let tabs = doc_tabs::tabs(&h.state().session);
    assert_eq!(tabs.iter().map(|t| (t.title.as_str(), t.dirty)).collect::<Vec<_>>(), [("Budget.xlsx", true), ("Plan.xlsx", true)]);
    let drafted: Vec<u64> = h.state().draft_docs().iter().filter(|d| d.unsaved).map(|d| d.uid).collect();
    assert_eq!(drafted, [a, b], "every open document is drafted, not only the active one");
}

#[test]
fn a_hidden_window_gets_no_tab() {
    let mut h = harness(&["Alpha.xlsx", "Beta.xlsx", "Gamma.xlsx"]);
    h.state_mut().run("view.hideWindow", json!({})).unwrap();
    h.run_steps(3);
    let shown: Vec<String> = doc_tabs::tabs(&h.state().session).into_iter().map(|t| t.title).collect();
    assert_eq!(shown, ["Alpha.xlsx", "Beta.xlsx"]);
    assert!(h.query_by_label("Close Gamma.xlsx").is_none());
}

/// Long and dirty titles at 1, 2 and 10 documents in Vietnamese. Set `GRIDCRAFT_DOC_TABS_SHOTS`
/// to a folder to save the renders there.
#[test]
fn document_tab_screenshots() {
    let Ok(dir) = std::env::var("GRIDCRAFT_DOC_TABS_SHOTS") else { return };
    let long = "Báo cáo doanh thu quý 3 năm 2026 - chi nhánh Hà Nội.xlsx";
    let mut ten = vec![long.to_string()];
    ten.extend((2..=MAX_DOCUMENTS).map(|i| format!("Sổ làm việc {i}.xlsx")));
    for (file, names) in [("1-document.png", vec![]), ("2-documents.png", vec![long.to_string(), "Dự toán.xlsx".into()]), ("10-documents.png", ten)]
    {
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut s = session(&names);
        // Unsaved changes in the first and the active document show their dot.
        for i in [0, s.documents().len() - 1] {
            s.run("window.activate", json!({"index": i})).unwrap();
            s.run("cell.set", json!({"cell": "C3", "input": "đã sửa"})).unwrap();
        }
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 360.0)).with_pixels_per_point(1.5).wgpu().build_ui_state(
            |ui, app: &mut SheetApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            {
                let mut app = SheetApp::new(s, Services::default());
                app.ui.language = Language::Vi;
                app
            },
        );
        h.run_steps(8);
        if names.len() > 1 {
            h.get_by_label(names[0]).hover();
            h.run_steps(30);
        }
        h.render().unwrap().save(std::path::Path::new(&dir).join(file)).unwrap();
    }
}
