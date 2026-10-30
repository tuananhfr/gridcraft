//! Host-imposed access (`DocState::access`): a read-only document refuses every edit at the
//! engine's single entry point and keeps its history; `no_download` refuses getting content out.

use std::sync::Arc;

use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_engine::doc_access::{DocAccess, NO_DOWNLOAD, READ_ONLY};
use serde_json::{Value, json};

const READ: DocAccess = DocAccess { read_only: true, no_download: false, commenter: false, edit_lock: None };
const NO_COPY: DocAccess = DocAccess { read_only: false, no_download: true, commenter: false, edit_lock: None };

/// Two sheets with values, a note, an undo step and a redo step.
fn session(access: DocAccess) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": [["b", 2], ["a", 1], ["c", 3], ["d", 4]]})).unwrap();
    s.execute("review.newNote", json!({"cell": "D1", "text": "note"})).unwrap();
    s.execute("home.insertSheet", json!({})).unwrap();
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    s.execute("cell.set", json!({"cell": "F1", "input": "undone"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("selection.set", json!({"range": "A1:B4"})).unwrap();
    let d = s.active_mut().unwrap();
    d.saved = d.wb.clone();
    d.access = access;
    s.take_ui_requests();
    s
}

/// One edit per command family, each valid on an editable document.
fn edits() -> Vec<(&'static str, Value)> {
    vec![
        ("cell.set", json!({"cell": "C1", "input": "5"})),
        ("range.setValues", json!({"range": "C1", "values": [[1]]})),
        ("edit.clearContents", json!({"range": "A1"})),
        ("edit.paste", json!({"at": "E1", "text": "x\ty"})),
        ("edit.autoFill", json!({"source": "B1:B2", "target": "B1:B8"})),
        ("edit.replace", json!({"what": "a", "with": "z"})),
        ("edit.cut", json!({"range": "A1"})),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
        ("edit.beginEdit", json!({})),
        ("home.bold", json!({"range": "A1"})),
        ("home.numberFormat", json!({"range": "B1", "format": "Percentage"})),
        ("home.insertRows", json!({"rows": "2:2"})),
        ("home.deleteColumns", json!({"cols": "B:B"})),
        ("home.columnWidth", json!({"cols": "A:A", "width": 120})),
        ("home.insertSheet", json!({})),
        ("sheet.rename", json!({"sheet": 0, "name": "Renamed"})),
        ("data.sortAscending", json!({"column": "A"})),
        ("data.filter", json!({"range": "A1:B4"})),
        ("insert.chart", json!({"range": "A1:B4", "type": "column"})),
        ("review.newComment", json!({"cell": "E5", "text": "hi"})),
        ("review.newNote", json!({"cell": "E6", "text": "hi"})),
        ("review.deleteComment", json!({"cell": "D1"})),
        ("formulas.defineName", json!({"name": "First", "refersTo": "=Sheet1!$A$1"})),
        ("pageLayout.orientation", json!({"orientation": "landscape"})),
        ("view.freezePanes", json!({"cell": "B2"})),
        ("formulas.calculateNow", json!({})),
        ("review.protectSheet", json!({})),
    ]
}

#[test]
fn every_command_family_is_refused_on_a_read_only_document() {
    for (id, params) in edits() {
        let mut editable = session(DocAccess::default());
        let r = editable.execute(id, params.clone());
        assert!(r.is_ok(), "{id} must work on an editable document: {r:?}");

        let mut s = session(READ);
        let d = s.active().unwrap();
        let (wb, undo, redo, uid) = (d.wb.clone(), d.undo.len(), d.redo.len(), d.uid);
        let err = s.execute(id, params).unwrap_err().to_string();
        assert!(err.contains(READ_ONLY), "{id}: {err}");
        let d = s.documents().iter().find(|d| d.uid == uid).unwrap();
        assert!(Arc::ptr_eq(&d.wb, &wb), "{id} changed the workbook");
        assert_eq!((d.undo.len(), d.redo.len()), (undo, redo), "{id} changed the history");
        assert_eq!(s.ui_requests, [], "{id} asked the interface for something");
    }
}

#[test]
fn no_command_changes_a_read_only_document_or_its_history() {
    for spec in gridcraft_engine::command_specs() {
        let mut s = session(READ);
        let d = s.active().unwrap();
        let (undo, redo, uid, note) = (d.undo.len(), d.redo.len(), d.uid, d.wb.sheets[0].comments.len());
        let _ = s.execute(spec.id, json!({}));
        let Some(d) = s.documents().iter().find(|d| d.uid == uid) else { continue };
        assert!(!d.is_dirty(), "{} changed a read-only document", spec.id);
        assert_eq!((d.undo.len(), d.redo.len()), (undo, redo), "{} changed the history", spec.id);
        assert_eq!(d.wb.sheets[0].comments.len(), note, "{}", spec.id);
        assert_eq!(d.wb.sheets[0].value(CellRef::new(0, 0)).as_text(), Some("b"), "{}", spec.id);
    }
}

#[test]
fn reading_and_moving_around_still_work_without_dirtying_or_history() {
    let mut s = session(READ);
    let undo = s.active().unwrap().undo.len();
    s.execute("selection.set", json!({"range": "B2:B3"})).unwrap();
    s.execute("sheet.activate", json!({"sheet": 1})).unwrap();
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    s.execute("view.zoom", json!({"percent": 150})).unwrap();
    s.execute("edit.find", json!({"what": "c"})).unwrap();
    s.execute("edit.goTo", json!({"reference": "C9"})).unwrap();
    s.execute("cell.get", json!({"cell": "A1"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap();
    let d = s.active().unwrap();
    assert_eq!(d.wb.sheets[0].zoom, 150);
    assert!(!d.is_dirty(), "view state isn't an edit");
    assert_eq!(d.undo.len(), undo, "a zoom on a read-only document is not an undo step");
    let info = s.commands().into_iter().find(|c| c.id == "home.bold").unwrap();
    assert!(!info.enabled && info.disabled_reason.as_deref() == Some(READ_ONLY), "the command list shows it unavailable");
}

#[test]
fn no_download_refuses_copy_save_as_export_and_print_only() {
    for id in ["edit.copy", "edit.cut", "file.saveAs", "file.saveBytes", "file.exportCsv", "file.exportHtml", "file.exportPdf", "file.print"] {
        let mut s = session(NO_COPY);
        let err = s.execute(id, json!({})).unwrap_err().to_string();
        assert!(err.contains(NO_DOWNLOAD), "{id}: {err}");
        assert!(s.clipboard.is_none(), "{id}");
        // CSV and HTML export need a file path, which a test shouldn't write.
        if !id.starts_with("file.export") || id == "file.exportPdf" {
            assert!(session(DocAccess::default()).execute(id, json!({})).is_ok(), "{id} works with downloads allowed");
        }
    }
    let mut s = session(NO_COPY);
    s.execute("cell.set", json!({"cell": "C1", "input": "5"})).unwrap();
    assert!(s.active().unwrap().is_dirty(), "editing is still allowed");
}

#[test]
fn switching_access_both_ways_keeps_the_document_and_its_history() {
    let mut s = session(DocAccess::default());
    s.execute("cell.set", json!({"cell": "C1", "input": "7"})).unwrap();
    let uid = s.active().unwrap().uid;
    let (wb, undo, redo) = {
        let d = s.active().unwrap();
        (d.wb.clone(), d.undo.len(), d.redo.len())
    };
    s.document_mut(uid).unwrap().access = READ;
    assert!(s.execute("edit.undo", json!({})).is_err());
    s.document_mut(uid).unwrap().access = DocAccess::default();
    let d = s.active().unwrap();
    assert!(Arc::ptr_eq(&d.wb, &wb));
    assert_eq!((d.undo.len(), d.redo.len()), (undo, redo));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().wb.sheets[0].value(CellRef::new(0, 2)).is_empty(), "the edit made before is still undoable");
}

const LOCK: &str = "Sign in to edit";

#[test]
fn the_edit_lock_refuses_every_edit_but_keeps_save_and_downloads() {
    for (id, params) in edits() {
        let mut s = session(DocAccess::default());
        s.set_edit_lock(Some(LOCK));
        let d = s.active().unwrap();
        let (wb, undo, uid) = (d.wb.clone(), d.undo.len(), d.uid);
        let err = s.execute(id, params).unwrap_err().to_string();
        assert!(err.contains(LOCK), "{id}: {err}");
        let d = s.documents().iter().find(|d| d.uid == uid).unwrap();
        assert!(Arc::ptr_eq(&d.wb, &wb) && d.undo.len() == undo, "{id} changed the document");
    }
    let mut s = session(DocAccess::default());
    s.execute("cell.set", json!({"cell": "C1", "input": "typed before"})).unwrap();
    s.set_edit_lock(Some(LOCK));
    let path = std::env::temp_dir().join(format!("gridcraft-edit-lock-{}.xlsx", std::process::id()));
    let saved = s.execute("file.save", json!({"path": path.to_string_lossy()}));
    let _ = std::fs::remove_file(&path);
    saved.expect("Save writes what was typed before the lock");
    assert!(!s.active().unwrap().is_dirty());
    for id in ["edit.copy", "file.saveBytes", "file.exportPdf", "file.print"] {
        s.execute(id, json!({})).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let info = s.commands().into_iter().find(|c| c.id == "file.save").unwrap();
    assert!(info.enabled, "Save stays offered under the lock");
    // The host's read-only still refuses Save, and its notice wins.
    s.active_mut().unwrap().access.read_only = true;
    assert!(s.execute("file.save", json!({})).unwrap_err().to_string().contains(READ_ONLY));
}

#[test]
fn the_edit_lock_covers_documents_opened_later_and_lifts_without_reopening() {
    let mut s = session(DocAccess::default());
    s.set_edit_lock(Some(LOCK));
    let first = s.active().unwrap().uid;
    s.new_workbook().unwrap();
    assert_eq!(s.edit_lock(), Some(LOCK));
    assert!(s.execute("cell.set", json!({"cell": "A1", "input": "1"})).is_err(), "a new workbook is locked too");
    s.set_edit_lock(None);
    s.execute("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    s.set_active(0);
    assert_eq!(s.active().unwrap().uid, first);
    s.execute("cell.set", json!({"cell": "C1", "input": "2"})).unwrap();
    assert!(s.documents().iter().all(|d| d.access.is_full()));
}

#[test]
fn the_host_author_writes_new_comments_and_notes() {
    let mut s = session(DocAccess::default());
    s.author = Some("Nguyễn Văn A".into());
    s.execute("review.newComment", json!({"cell": "E1", "text": "x"})).unwrap();
    s.execute("review.replyComment", json!({"cell": "E1", "text": "y"})).unwrap();
    s.author = None;
    s.execute("review.newNote", json!({"cell": "E2", "text": "z"})).unwrap();
    let sh = &s.active().unwrap().wb.sheets[0];
    let comment = &sh.comments[&CellRef::new(0, 4)];
    assert_eq!(comment.author, "Nguyễn Văn A");
    assert_eq!(comment.replies[0].0, "Nguyễn Văn A");
    assert_eq!(sh.comments[&CellRef::new(1, 4)].author, s.prefs.user_name, "without one, the preference names the author");
}

#[test]
fn moving_a_document_keeps_the_active_one() {
    let mut s = Session::new();
    let uids: Vec<u64> = (0..3)
        .map(|_| {
            let i = s.new_workbook().unwrap();
            s.documents()[i].uid
        })
        .collect();
    s.move_document(2, 0);
    assert_eq!(s.documents().iter().map(|d| d.uid).collect::<Vec<_>>(), [uids[2], uids[0], uids[1]]);
    assert_eq!(s.active().unwrap().uid, uids[2]);
    s.move_document(0, 9);
    assert_eq!(s.active().unwrap().uid, uids[2], "out of range does nothing");
}
