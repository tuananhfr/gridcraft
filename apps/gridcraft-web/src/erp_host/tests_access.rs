//! `erpHost` v2 access modes (rules 12–15), `setMode`, `canDownload`, `preserveView` and `setUser`
//! against the real app with a fake shell.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_engine::doc_access::{COMMENTS_LATER, DocAccess, NO_DOWNLOAD, READ_ONLY};
use gridcraft_ui_egui::SheetView;
use gridcraft_ui_egui::i18n::Language;
use gridcraft_ui_egui::save_status::SaveState;
use serde_json::json;

use super::autosave::{CHECKPOINT_MS, IDLE_MS};
use super::docs::HostDocs;
use super::protocol::{Mode, OpenRequest, Reason, Request, SaveResult};
use super::tests::{Call, T, setup, setup_version, xlsx};

/// A two-sheet workbook whose first cell reads `a1`.
fn two_sheets(a1: &str, second: &str) -> Vec<u8> {
    let mut s = Session::new();
    s.new_workbook();
    s.run("cell.set", json!({"cell": "A1", "input": a1})).unwrap();
    s.run("home.insertSheet", json!({})).unwrap();
    s.run("sheet.rename", json!({"name": second})).unwrap();
    gridcraft_engine::io::save_bytes(&s.active().unwrap().wb, "x.xlsx").unwrap()
}

fn request(doc_id: &str, bytes: Vec<u8>, access: DocAccess, preserve_view: bool) -> OpenRequest {
    OpenRequest {
        doc_id: doc_id.into(), version: Some(1), title: "Báo cáo".into(), format: "xlsx".into(), bytes: Some(bytes), access, preserve_view
    }
}

impl T {
    fn open_with(&mut self, doc_id: &str, access: DocAccess) -> u64 {
        self.open_request(request(doc_id, xlsx("1"), access, false));
        self.app.session.active().unwrap().uid
    }

    fn apply(&mut self, request: Request) {
        self.docs.borrow_mut().apply(&mut self.app, request);
    }

    fn set_mode(&mut self, doc_id: &str, mode: Mode, can_download: bool) {
        self.apply(Request::Mode(doc_id.into(), mode.access(can_download)));
    }

    fn refused(&mut self, id: &str, params: serde_json::Value) -> String {
        self.app.run(id, params).expect_err(id)
    }

    fn a1(&self, uid: u64) -> Option<f64> {
        self.doc(uid).wb.sheets[0].value(CellRef::new(0, 0)).as_f64()
    }
}

#[test]
fn the_mode_vocabulary_matches_the_contract() {
    assert_eq!([None, Some("edit"), Some("comment"), Some("view")].map(Mode::parse), [Mode::Edit, Mode::Edit, Mode::Comment, Mode::View]);
    assert_eq!(Mode::parse(Some("suggest")), Mode::View, "an unknown mode can't edit");
    assert_eq!(Mode::Edit.access(true), DocAccess::default());
    assert_eq!(Mode::Comment.access(true), DocAccess { read_only: true, no_download: false, commenter: true }, "comment is view for the content");
    assert_eq!(Mode::View.access(false), DocAccess { read_only: true, no_download: true, commenter: false });
}

#[test]
fn view_and_comment_refuse_every_edit_entry_point() {
    for (mode, notice) in [(Mode::View, READ_ONLY), (Mode::Comment, COMMENTS_LATER)] {
        let mut t = setup();
        let uid = t.open_with("d1", mode.access(true));
        let wb = t.doc(uid).wb.clone();
        assert!(t.app.read_only());

        // Typing, F2 and the formula bar open no inline editor; the notice says why.
        t.app.begin_edit(Some("9".into()), false);
        t.app.begin_edit(None, true);
        assert!(t.app.editor.is_none(), "{mode:?}");
        assert_eq!(t.app.toast.as_ref().map(|(m, _)| m.as_str()), Some(notice));
        // Paste of external text, and the commands behind every ribbon and menu family.
        t.app.paste_external("5\t6".into());
        for (id, params) in [
            ("cell.set", json!({"cell": "A1", "input": "2"})),
            ("edit.paste", json!({"text": "7"})),
            ("edit.autoFill", json!({"source": "A1", "target": "A1:A5"})),
            ("edit.clearContents", json!({})),
            ("edit.replace", json!({"what": "1", "with": "2"})),
            ("home.bold", json!({})),
            ("home.insertRows", json!({})),
            ("home.insertSheet", json!({})),
            ("sheet.rename", json!({"name": "X"})),
            ("data.sortAscending", json!({})),
            ("data.filter", json!({})),
            ("insert.chart", json!({"range": "A1"})),
            ("review.newComment", json!({"text": "x"})),
            ("formulas.defineName", json!({"name": "N"})),
            ("pageLayout.orientation", json!({"orientation": "landscape"})),
            ("file.save", json!({})),
        ] {
            assert!(t.refused(id, params).contains(notice), "{mode:?} {id}");
        }
        assert!(Arc::ptr_eq(&t.doc(uid).wb, &wb), "{mode:?}: the document is as it was");
        assert!(t.app.allows("selection.set") && t.app.allows("view.zoom") && t.app.allows("edit.find"));
        assert!(!t.app.offers("home.bold") && !t.app.offers("dialog:formatCells") && t.app.offers("dialog:find"));
        t.app.run("selection.set", json!({"range": "B2:C3"})).unwrap();
        t.app.run("view.zoom", json!({"percent": 150})).unwrap();
        t.app.run("edit.copy", json!({})).unwrap();
        assert!(!t.doc(uid).is_dirty());
    }
}

#[test]
fn a_read_only_document_sends_no_save_traffic() {
    let mut t = setup();
    let uid = t.open_with("d1", Mode::View.access(true));
    t.app.run_or_alert("file.save", json!({}));
    for now in [0.0, IDLE_MS + 1.0, CHECKPOINT_MS + 1.0] {
        t.frame(now);
    }
    let flushed = Rc::new(RefCell::new(false));
    let flag = flushed.clone();
    t.docs.borrow_mut().flush(Box::new(move || *flag.borrow_mut() = true));
    assert!(*flushed.borrow(), "rule 13: flush resolves at once");
    assert!(t.saves().is_empty(), "no AutoSave, Ctrl+S or flush");
    assert_eq!(*t.calls(), [Call::Opened("d1".into(), true, None)], "dirty stays unsent");
    assert_eq!(t.app.toast.as_ref().map(|(m, _)| m.as_str()), Some(READ_ONLY), "Ctrl+S only shows the read-only notice");
    assert!(!t.doc(uid).is_dirty());
}

#[test]
fn ctrl_s_on_a_read_only_document_is_not_a_failed_save() {
    for mode in [Mode::View, Mode::Comment] {
        let mut t = setup();
        let uid = t.open_with("d1", mode.access(true));
        t.app.run_or_alert("file.save", json!({}));
        t.frame(0.0);
        assert_eq!(t.app.save_state(uid), SaveState::Idle, "{mode:?}: no \"Save failed\" in the status bar");
        assert!(t.app.message.is_none(), "{mode:?}: only the notice");
    }
}

#[test]
fn comment_mode_says_comments_come_later() {
    let mut t = setup();
    t.open_with("d1", Mode::Comment.access(true));
    t.app.begin_edit(Some("9".into()), false);
    assert_eq!(t.app.toast.as_ref().map(|(m, _)| m.as_str()), Some(COMMENTS_LATER));
    t.app.toast = None;
    t.app.run_or_alert("file.save", json!({}));
    assert_eq!(t.app.toast.as_ref().map(|(m, _)| m.as_str()), Some(COMMENTS_LATER), "Ctrl+S too");
    assert_eq!(Language::Vi.tr(COMMENTS_LATER), "Nhận xét trên web sẽ có sau, hiện chỉ xem được");
    assert_eq!(Language::Vi.tr(READ_ONLY), "Tài liệu này chỉ được xem, không sửa được.", "view mode keeps its notice");
}

#[test]
fn the_shells_share_replaces_the_title_bars_for_its_documents_from_v2() {
    let mut t = setup_version(2);
    let hosted = t.open("d1");
    assert!(t.app.host_shares(hosted));
    let i = t.app.session.new_workbook().unwrap();
    let local = t.app.session.documents()[i].uid;
    assert!(!t.app.host_shares(local), "a local workbook keeps its own Share");

    let mut t = setup_version(1);
    let hosted = t.open("d1");
    assert!(!t.app.host_shares(hosted), "a v1 shell has no Share of its own");
}

#[test]
fn set_mode_both_ways_keeps_the_document_and_its_undo_history() {
    let mut t = setup();
    let uid = t.open("d1");
    t.edit("A1", "2");
    t.edit("A2", "3");
    t.frame(0.0);
    let (wb, undo) = (t.doc(uid).wb.clone(), t.doc(uid).undo.len());

    t.set_mode("d1", Mode::View, true);
    assert!(t.app.read_only());
    assert!(t.refused("edit.undo", json!({})).contains(READ_ONLY));
    t.frame(IDLE_MS + 1.0);
    assert!(t.saves().is_empty(), "no AutoSave while read-only");
    assert!(Arc::ptr_eq(&t.doc(uid).wb, &wb));
    assert_eq!(t.doc(uid).undo.len(), undo);

    t.set_mode("d1", Mode::Edit, true);
    assert!(!t.app.read_only());
    assert_eq!(t.doc(uid).undo.len(), undo, "the history is intact");
    t.frame(IDLE_MS + 2.0);
    assert_eq!(t.saves().len(), 1, "the edits made before are saved once editable: nothing was discarded");
    t.app.run("edit.undo", json!({})).unwrap();
    assert!(t.doc(uid).wb.sheets[0].value(CellRef::new(1, 0)).is_empty());
    t.set_mode("unknown", Mode::View, true);
    assert!(!t.app.read_only(), "an unknown docId changes nothing");
}

#[test]
fn a_downgrade_commits_the_open_cell_editor_and_flush_saves_it_once() {
    let mut t = setup();
    let uid = t.open("d1");
    t.frame(0.0);
    let undo = t.doc(uid).undo.len();
    t.app.begin_edit(Some("7".into()), false);
    assert!(t.app.editor.is_some() && !t.doc(uid).is_dirty(), "typed, not committed");

    t.set_mode("d1", Mode::View, false);
    assert!(t.app.editor.is_none() && t.app.read_only());
    assert_eq!(t.a1(uid), Some(7.0), "the typed value is in the document");
    assert_eq!(t.doc(uid).undo.len(), undo + 1, "as one undo step");
    t.frame(10.0);
    assert!(t.doc(uid).is_dirty());
    assert_eq!(t.calls().last(), Some(&Call::Dirty("d1".into(), true)), "the shell sees it dirty and flushes again");

    let done = Rc::new(RefCell::new(false));
    let flag = done.clone();
    t.docs.borrow_mut().flush(Box::new(move || *flag.borrow_mut() = true));
    assert_eq!(t.saves(), [("d1".to_string(), t.doc(uid).revision, Reason::Flush)], "one save, reason flush");
    assert!(*done.borrow());
    t.frame(20.0);
    t.resolve(0, SaveResult::Saved { version: Some(2) });
    t.frame(30.0);
    assert!(!t.doc(uid).is_dirty(), "`saved` clears dirty");
    assert_eq!(t.calls().last(), Some(&Call::Dirty("d1".into(), false)));

    t.docs.borrow_mut().flush(Box::new(|| {}));
    t.app.run_or_alert("file.save", json!({}));
    for now in [IDLE_MS + 40.0, CHECKPOINT_MS + 40.0] {
        t.frame(now);
    }
    assert_eq!(t.saves().len(), 1, "no further saves");
    assert_eq!(t.a1(uid), Some(7.0));
}

#[test]
fn the_flush_the_shell_makes_from_its_dirty_handler_saves_what_the_downgrade_committed() {
    let mut t = setup();
    let uid = t.open("d1");
    t.frame(0.0);
    t.app.begin_edit(Some("7".into()), false);
    let entry = HostDocs::flush_entry(&t.docs);
    let done = Rc::new(RefCell::new(0));
    let count = done.clone();
    // The real shell answers dirty(true) after a downgrade with flush(), still inside this side's call.
    *t.shell.on_dirty.borrow_mut() = Some(Box::new(move || {
        let count = count.clone();
        entry.flush(Box::new(move || *count.borrow_mut() += 1));
    }));

    t.set_mode("d1", Mode::View, true);
    t.frame(10.0);
    assert_eq!(t.calls().last(), Some(&Call::Dirty("d1".into(), true)));
    assert_eq!(t.saves(), [("d1".to_string(), t.doc(uid).revision, Reason::Flush)], "handed to save in the same frame");
    assert_eq!(*done.borrow(), 1, "and that flush settles");
    assert_eq!(t.a1(uid), Some(7.0));
}

#[test]
fn an_edit_the_workbook_refuses_survives_a_downgrade_and_reopens_once_editable() {
    let mut t = setup();
    let uid = t.open("d1");
    t.app.run("data.validation", json!({"range": "B2", "type": "list", "formula1": "\"First,Second\""})).unwrap();
    t.app.run("selection.set", json!({"range": "B2"})).unwrap();
    let undo = t.doc(uid).undo.len();
    t.app.begin_edit(Some("Third".into()), false);

    t.set_mode("d1", Mode::View, true);
    assert!(t.app.editor.is_none() && t.app.read_only(), "the editor closes");
    assert!(t.app.message.is_some(), "the usual validation message");
    assert_eq!(t.doc(uid).undo.len(), undo, "nothing was entered");
    assert!(t.doc(uid).wb.sheets[0].value(CellRef::new(1, 1)).is_empty());
    t.app.run("selection.set", json!({"range": "D9"})).unwrap();
    t.app.begin_edit(None, true);
    assert!(t.app.editor.is_none(), "still read-only");

    t.app.message = None;
    t.set_mode("d1", Mode::Edit, true);
    let edit = t.app.editor.as_ref().expect("the typed text is back in the cell editor");
    assert_eq!((edit.cell, edit.text.as_str()), (CellRef::new(1, 1), "Third"));
    assert_eq!(t.doc(uid).selection.a1(), "B2");
    assert!(t.app.held_edits.is_empty());
}

#[test]
fn a_downgraded_document_left_dirty_is_saved_by_flush_until_stopped() {
    let mut t = setup();
    let uid = t.open("d1");
    t.edit("A1", "2");
    t.frame(0.0);
    t.set_mode("d1", Mode::Comment, true);
    t.frame(IDLE_MS + 1.0);
    assert!(t.saves().is_empty(), "no AutoSave while read-only");
    t.docs.borrow_mut().flush(Box::new(|| {}));
    t.resolve(0, SaveResult::Offline);
    t.docs.borrow_mut().flush(Box::new(|| {}));
    assert_eq!(t.saves().len(), 2, "still unsaved: the next flush tries again");
    t.resolve(1, SaveResult::Conflict);
    t.docs.borrow_mut().flush(Box::new(|| {}));
    assert_eq!(t.saves().len(), 2, "a conflict stops it (rule 4)");
    assert!(t.doc(uid).is_dirty(), "the edits are kept");
}

#[test]
fn a_downgrade_commits_a_text_box_being_typed_into() {
    let mut t = setup();
    let uid = t.open("d1");
    let shape = t.app.run("insert.textBox", json!({"text": "old"})).unwrap()["shape"].as_u64().unwrap() as u32;
    let undo = t.doc(uid).undo.len();
    assert!(t.app.begin_text_box_edit(shape));
    t.app.text_box_editor.as_mut().unwrap().text = "typed".into();
    t.set_mode("d1", Mode::View, true);
    assert!(t.app.text_box_editor.is_none());
    assert_eq!(t.doc(uid).undo.len(), undo + 1, "one undo step");
    let sheet = &t.doc(uid).wb.sheets[0];
    assert!(sheet.shapes.iter().any(|s| s.id == shape && s.text == "typed"), "the typed text is in the document");
}

#[test]
fn can_download_false_disables_save_as_export_print_and_copy() {
    let mut t = setup();
    let uid = t.open_with("d1", Mode::Edit.access(false));
    assert!(!t.app.can_download());
    for id in ["file.saveAs", "file.saveBytes", "file.exportPdf", "file.exportCsv", "file.print", "edit.copy", "edit.cut"] {
        assert!(t.refused(id, json!({})).contains(NO_DOWNLOAD), "{id}");
        assert!(!t.app.offers(id), "{id}");
    }
    assert!(t.downloads.borrow().is_empty());
    assert!(t.app.session.clipboard.is_none());
    t.edit("A1", "2");
    assert_eq!(t.a1(uid), Some(2.0), "editing is still allowed");

    t.set_mode("d1", Mode::View, true);
    t.app.run("edit.copy", json!({})).unwrap();
    t.app.run("file.saveAs", json!({})).unwrap();
    assert_eq!(t.downloads.borrow().len(), 1, "a viewer allowed to download saves a local copy");
}

#[test]
fn preserve_view_keeps_the_sheet_selection_and_top_left_cell_in_the_same_tab() {
    let mut t = setup();
    t.open_request(request("d1", two_sheets("1", "Data"), Mode::View.access(true), false));
    let other = t.open("d2");
    let old = t.app.session.documents()[0].uid;
    t.activate(old);
    t.app.run("sheet.activate", json!({"sheet": 1})).unwrap();
    t.app.run("selection.set", json!({"range": "C5:D7"})).unwrap();
    let sh = &t.doc(old).wb.sheets[1];
    let scroll = egui::vec2(sh.col_left(3) as f32, sh.row_top(10) as f32);
    t.app.views.insert((old, 1), SheetView { scroll });
    let before = t.app.view_snapshot(old).unwrap();
    assert_eq!(before.top_left, CellRef::new(10, 3));

    t.open_request(request("d1", two_sheets("2", "Data"), Mode::View.access(true), true));
    let new = t.app.session.documents()[0].uid;
    assert_ne!(new, old, "the newer version replaced the old one in its tab");
    assert_eq!(t.a1(new), Some(2.0));
    assert_eq!(t.app.session.active().unwrap().uid, new);
    let d = t.doc(new);
    assert_eq!(d.wb.active().unwrap().name, "Data");
    assert_eq!(d.selection.a1(), "C5:D7");
    assert_eq!(t.app.view_snapshot(new), Some(before));
    assert!(!d.is_dirty() && t.app.read_only());

    // Reloaded while the user is on another tab: that tab stays in front.
    t.activate(other);
    t.open_request(request("d1", two_sheets("3", "Renamed"), Mode::View.access(true), true));
    assert_eq!(t.app.session.active().unwrap().uid, other);
    let newest = t.app.session.documents()[0].uid;
    assert_eq!(t.a1(newest), Some(3.0));
    let d = t.doc(newest);
    assert_eq!(d.wb.active().unwrap().name, "Renamed", "the sheet is gone: the version opens as saved");
    assert_eq!(d.selection.a1(), "A1");
    assert_eq!(t.calls().iter().filter(|c| matches!(c, Call::Closed(_))).count(), 0, "a reload is not a close");
}

#[test]
fn set_user_authors_new_comments_and_null_keeps_the_preference() {
    let mut t = setup();
    let uid = t.open("d1");
    t.apply(Request::User(Some(" Nguyễn Văn A ".into())));
    t.app.run("review.newComment", json!({"cell": "B1", "text": "x"})).unwrap();
    t.apply(Request::User(None));
    t.app.run("review.newNote", json!({"cell": "B2", "text": "y"})).unwrap();
    let comments = &t.doc(uid).wb.sheets[0].comments;
    assert_eq!(comments[&CellRef::new(0, 1)].author, "Nguyễn Văn A");
    assert_eq!(comments[&CellRef::new(1, 1)].author, t.app.session.prefs.user_name);
}
