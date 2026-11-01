//! The `erpHost` comment calls through the app with a fake shell: the anchor of the selection,
//! marks for open threads, focus, and anchors placed again after the other page's edits (the
//! M4d fake room between two pages). Marks never touch the document or what is saved.

use gridcraft_engine::core::CellRef;
use serde_json::{Value as Json, json};

use super::comments::{self, CommentMsg};
use super::protocol::OpenRequest;
use super::tests::{T, setup, xlsx};
use super::tests_live::{exchange, page};

fn target(t: &T) -> Option<u64> {
    t.docs.borrow().live_target().map(|(uid, _)| uid)
}

fn select(t: &mut T, range: &str) -> Json {
    t.app.run("selection.set", json!({"range": range})).unwrap();
    comments::anchor_of_selection(&t.app, target(t))
}

/// The marks on the first sheet after the app placed them again, as A1 ranges.
fn marks(t: &mut T) -> Vec<(String, String)> {
    let doc = target(t).unwrap();
    let app = &mut t.app;
    app.comment_marks.refresh(&app.session);
    app.comment_marks.marks_on(doc, "f1").into_iter().map(|(id, r)| (id.to_string(), r.a1())).collect()
}

fn thread(id: &str, anchor: &Json, status: &str) -> Json {
    json!({"id": id, "anchor": anchor, "status": status, "count": 1})
}

#[test]
fn the_selection_gives_a_cell_anchor_at_the_opened_version() {
    let mut t = page();
    let a = select(&mut t, "B3");
    assert_eq!(a["v"], 1);
    assert_eq!(a["region"], "cell:f1:B3");
    assert_eq!(a["pos"], json!({"t": "v4"}));
    assert_eq!(a["soft"], json!({"sheet": "Sheet1", "a1": "B3", "content": ""}));
    assert_eq!((a["label"].as_str(), a["quote"].as_str()), (Some("Sheet1!B3"), Some("Sheet1!B3")));
    assert_eq!(a["order"], json!([0, 2, 1]));
    let range = select(&mut t, "A1:C4");
    assert_eq!(range["region"], "cell:f1:A1");
    assert_eq!(range["pos"]["range"], "A1:C4");
    assert_eq!(range["soft"]["content"], "1");
    // Unchanged: the shell is told it is still open, with no new anchor to store.
    let updates = comments::resolve(&t.app, target(&t), &json!([thread("t1", &a, "open"), thread("t2", &range, "resolved")]));
    assert_eq!(updates, json!([{"id": "t1", "threadId": "t1", "status": "open"}, {"id": "t2", "threadId": "t2", "status": "open"}]));
}

#[test]
fn open_threads_get_marks_and_a_click_names_the_thread() {
    let mut t = page();
    let b3 = select(&mut t, "B3");
    let range = select(&mut t, "D2:E5");
    let doc = target(&t).unwrap();
    comments::deliver(
        &mut t.app,
        Some(doc),
        vec![CommentMsg::Show(json!([
            thread("t1", &b3, "open"),
            thread("t2", &range, "open"),
            thread("t3", &b3, "resolved"),
            thread("t4", &b3, "orphaned"),
            {"id": "t5", "anchor": {"kind": "document"}, "status": "open"},
            {"anchor": b3},
            null
        ]))],
    );
    assert_eq!(marks(&mut t), [("t1".to_string(), "B3".to_string()), ("t2".into(), "D2:E5".into())]);
    t.app.comment_marks.click(doc, "f1", CellRef::new(3, 4));
    t.app.comment_marks.click(doc, "f1", CellRef::new(9, 9));
    t.app.comment_marks.click(doc, "f2", CellRef::new(2, 1));
    assert_eq!(t.app.comment_marks.take_clicked(), ["t2"]);
    assert!(t.app.comment_marks.take_clicked().is_empty());
    comments::show(&mut t.app, doc, &json!(null));
    assert!(marks(&mut t).is_empty());
}

/// The shell's document open without co-editing (a shell with comments but no live room).
fn alone() -> T {
    let mut t = setup();
    t.open_request(OpenRequest {
        doc_id: "d1".into(),
        version: Some(2),
        title: "Báo cáo".into(),
        format: "xlsx".into(),
        bytes: Some(xlsx("1")),
        access: Default::default(),
        preserve_view: false,
    });
    t
}

#[test]
fn focus_selects_the_anchor_on_its_sheet_and_highlights_the_thread() {
    let mut t = alone();
    t.app.run("home.insertSheet", json!({})).unwrap();
    let a = select(&mut t, "C7:D8");
    assert_eq!(a["pos"]["t"], Json::Null, "no room, no layout token");
    let sheet = t.app.session.active().unwrap().wb.active_sheet;
    t.app.run("sheet.activate", json!({"sheet": 1 - sheet})).unwrap();
    t.app.run("selection.set", json!({"range": "A1"})).unwrap();
    let doc = target(&t).unwrap();
    t.app.session.new_workbook();
    comments::deliver(&mut t.app, Some(doc), vec![CommentMsg::Focus(a.clone(), Some("t1".into()))]);
    let d = t.app.session.active().unwrap();
    assert_eq!(d.uid, doc, "the shell's document comes to the front");
    assert_eq!(d.wb.active_sheet, sheet);
    assert_eq!(d.selection.current().a1(), "C7:D8");
    assert!(t.app.grid.ensure_visible);
    assert_eq!(t.app.comment_marks.focused.as_deref(), Some("t1"));
    assert!(!comments::focus(&mut t.app, doc, &json!({"kind": "document"}), None));
}

#[test]
fn marks_and_focus_never_change_the_document_or_its_saved_bytes() {
    let mut t = page();
    let a = select(&mut t, "B3");
    let doc = target(&t).unwrap();
    let (wb, revision) = {
        let d = t.app.session.active().unwrap();
        (d.wb.clone(), d.revision)
    };
    let saved = gridcraft_engine::io::save_bytes(&wb, "x.xlsx").unwrap();
    let calls = t.calls().len();
    comments::show(&mut t.app, doc, &json!([thread("t1", &a, "open")]));
    assert_eq!(marks(&mut t).len(), 1);
    t.app.comment_marks.click(doc, "f1", CellRef::new(2, 1));
    let _ = comments::resolve(&t.app, Some(doc), &json!([thread("t1", &a, "open")]));
    t.frame(0.0);
    let d = t.app.session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&d.wb, &wb), "the workbook is the same snapshot");
    assert_eq!(d.revision, revision);
    assert_eq!(gridcraft_engine::io::save_bytes(&d.wb, "x.xlsx").unwrap(), saved);
    assert_eq!(t.calls().len(), calls, "no dirty or save call");
    assert!(t.saves().is_empty());
}

#[test]
fn without_the_shells_document_there_is_no_anchor_and_nothing_resolves() {
    let mut t = setup();
    assert_eq!(comments::anchor_of_selection(&t.app, None), Json::Null);
    assert_eq!(comments::resolve(&t.app, None, &json!([{"id": "t1", "anchor": {"v": 1}}])), json!([]));
    comments::deliver(&mut t.app, None, vec![CommentMsg::Show(json!([]))]);
    assert!(t.app.comment_marks.is_empty());
    // A local workbook in front of the shell's document gives no anchor either.
    let mut t = page();
    let doc = target(&t);
    t.app.session.new_workbook();
    assert_eq!(comments::anchor_of_selection(&t.app, doc), Json::Null);
}

#[test]
fn the_other_pages_row_insert_and_delete_move_and_orphan_threads() {
    let mut pages = [page(), page()];
    let mut locks = Vec::new();
    pages[0].edit("B3", "total");
    pages[0].edit("C4", "gone");
    exchange(&mut pages, &mut locks, 100_000.0);
    let b3 = select(&mut pages[0], "B3");
    let c4 = select(&mut pages[0], "C4");
    let threads = json!([thread("t1", &b3, "open"), thread("t2", &c4, "open")]);
    let doc = target(&pages[0]).unwrap();
    comments::show(&mut pages[0].app, doc, &threads);
    assert_eq!(marks(&mut pages[0]), [("t1".to_string(), "B3".to_string()), ("t2".into(), "C4".into())]);
    // The other page inserts two rows above: the marks follow at once, and the leader's
    // resolveAnchors hands the shell the new anchors.
    pages[1].app.run("home.insertRows", json!({"rows": "1:2"})).unwrap();
    exchange(&mut pages, &mut locks, 200_000.0);
    exchange(&mut pages, &mut locks, 300_000.0);
    assert_eq!(marks(&mut pages[0]), [("t1".to_string(), "B5".to_string()), ("t2".into(), "C6".into())]);
    for t in &pages {
        let updates = comments::resolve(&t.app, target(t), &threads);
        assert_eq!(updates[0]["status"], "open");
        assert_eq!(updates[0]["anchor"]["region"], "cell:f1:B5");
        assert_eq!(updates[1]["anchor"]["region"], "cell:f1:C6");
    }
    // Then deletes C6's row: that thread has nowhere to go.
    pages[1].app.run("selection.set", json!({"range": "A6"})).unwrap();
    pages[1].app.run("home.deleteRows", json!({"rows": "6:6"})).unwrap();
    exchange(&mut pages, &mut locks, 400_000.0);
    exchange(&mut pages, &mut locks, 500_000.0);
    let updates = comments::resolve(&pages[0].app, Some(doc), &threads);
    assert_eq!(updates[0]["anchor"]["region"], "cell:f1:B5");
    assert_eq!(updates[1], json!({"id": "t2", "threadId": "t2", "status": "orphaned"}));
    assert_eq!(marks(&mut pages[0]), [("t1".to_string(), "B5".to_string())]);
}
