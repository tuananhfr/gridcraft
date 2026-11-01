//! `erpHost` v3 through the app: two pages of the same shared workbook, a fake room between them
//! that grants locks like the server, the in-cell editor asking for its cell's lock, and what
//! the user sees when a lock is someone else's.

use gridcraft_engine::live::{Call, Region};
use serde_json::json;

use super::live::{self, LiveMsg, cursors_of, locks_of};
use super::protocol::OpenRequest;
use super::tests::{T, setup, xlsx};

fn page() -> T {
    let mut t = setup();
    t.open_request(OpenRequest {
        doc_id: "d1".into(),
        version: Some(4),
        title: "Báo cáo".into(),
        format: "xlsx".into(),
        bytes: Some(xlsx("1")),
        access: Default::default(),
        preserve_view: false,
    });
    live::follow(&mut t.app, t.docs.borrow().live_target());
    t.app.session.live.as_mut().unwrap().set_online(true);
    t
}

/// Runs frames on both pages until the room has nothing left to deliver.
fn exchange(pages: &mut [T; 2], locks: &mut Vec<(String, usize)>, now: f64) {
    for _ in 0..20 {
        let mut inbox: [Vec<LiveMsg>; 2] = [vec![], vec![]];
        let mut moved = false;
        for i in 0..2 {
            for call in live::after_frame(&mut pages[i].app, now) {
                match call {
                    Call::WillEdit(r) => {
                        moved = true;
                        let want = Region::parse(&r).unwrap();
                        let free = !locks.iter().any(|(h, by)| *by != i && Region::parse(h).unwrap().clashes(&want));
                        if free {
                            locks.push((r.clone(), i));
                        }
                        inbox[i].push(LiveMsg::Answer(r, free));
                    }
                    Call::Changed(r, p) => {
                        moved = true;
                        inbox[1 - i].push(LiveMsg::Apply(r, p));
                    }
                    Call::DidEndEdit(r) => {
                        moved = true;
                        locks.retain(|(h, by)| !(h == &r && *by == i));
                    }
                    Call::Selection(..) => {}
                }
            }
        }
        for (i, messages) in inbox.into_iter().enumerate() {
            let shown = json!(locks.iter().map(|(r, by)| json!({"region": r, "holder": format!("m{by}"), "name": format!("User {by}"), "colour": "#00aa00", "mine": *by == i})).collect::<Vec<_>>());
            let (others, mine) = locks_of(&shown);
            let mut all = messages;
            all.push(LiveMsg::Locks(others, mine));
            live::deliver(&mut pages[i].app, all);
        }
        if !moved {
            return;
        }
    }
}

fn a1(t: &T) -> String {
    t.app.session.active().unwrap().wb.sheet(0).unwrap().cell(gridcraft_engine::core::CellRef::new(0, 1)).map(|c| c.input_text()).unwrap_or_default()
}

#[test]
fn the_shell_doc_is_followed_with_its_version_as_origin() {
    let t = page();
    let live = t.app.session.live.as_ref().unwrap();
    assert_eq!(live.doc(), t.app.session.active().unwrap().uid);
    assert!(t.app.session.live_serves_active());
}

#[test]
fn typing_in_a_cell_locks_it_first_and_reaches_the_other_page() {
    let mut pages = [page(), page()];
    let mut locks = Vec::new();
    pages[0].app.run("selection.set", json!({"range": "B1"})).unwrap();
    pages[0].app.begin_edit(Some("4".into()), false);
    exchange(&mut pages, &mut locks, 0.0);
    assert_eq!(locks, [("cell:f1:B1".to_string(), 0)]);
    // The other page sees the lock and its editor doesn't open there, with a toast saying who.
    pages[1].app.run("selection.set", json!({"range": "B1"})).unwrap();
    pages[1].app.begin_edit(Some("5".into()), false);
    assert!(pages[1].app.editor.is_none());
    exchange(&mut pages, &mut locks, 0.0);
    assert!(pages[1].app.toast.as_ref().is_some_and(|(text, _)| text.contains("User 0")));
    assert!(pages[0].app.commit_edit(1, 0, false, false));
    exchange(&mut pages, &mut locks, 0.0);
    assert_eq!(a1(&pages[1]), "4");
    // Moving on hands the cell back.
    exchange(&mut pages, &mut locks, 0.0);
    assert!(locks.is_empty(), "{locks:?}");
}

#[test]
fn a_lost_lock_closes_the_editor() {
    let mut pages = [page(), page()];
    let mut locks = Vec::new();
    pages[0].app.begin_edit(Some("x".into()), false);
    exchange(&mut pages, &mut locks, 0.0);
    live::deliver(&mut pages[0].app, vec![LiveMsg::Denied("cell:f1:A1".into())]);
    live::after_frame(&mut pages[0].app, 1.0);
    assert!(pages[0].app.editor.is_none());
    assert!(pages[0].app.toast.is_some());
}

#[test]
fn undo_on_a_page_keeps_the_other_pages_edit() {
    let mut pages = [page(), page()];
    let mut locks = Vec::new();
    pages[0].edit("C1", "mine");
    exchange(&mut pages, &mut locks, 0.0);
    pages[1].edit("D1", "theirs");
    exchange(&mut pages, &mut locks, 100_000.0);
    pages[0].app.run("edit.undo", json!({})).unwrap();
    exchange(&mut pages, &mut locks, 200_000.0);
    for t in &pages {
        let sh = t.app.session.active().unwrap().wb.sheet(0).cloned().unwrap();
        assert!(sh.cell(gridcraft_engine::core::CellRef::new(0, 2)).is_none());
        assert_eq!(sh.cell(gridcraft_engine::core::CellRef::new(0, 3)).unwrap().input_text(), "theirs");
    }
}

#[test]
fn locks_and_cursors_read_from_the_shells_lists() {
    let (others, mine) = locks_of(&json!([
        {"region": "cell:f1:A1", "holder": "m1", "name": "An", "colour": "#ff0000", "mine": false},
        {"region": "sheet:f2", "holder": "m0", "name": "Me", "colour": "#00ff00", "mine": true},
        {"holder": "m3"}
    ]));
    assert_eq!(others.len(), 1);
    assert_eq!(others[0].name, "An");
    assert_eq!(mine, ["sheet:f2"]);
    let cursors = cursors_of(&json!([{"author": "m1", "name": "An", "colour": "#ff0000", "region": "cell:f1:B2", "range": "B2:C3"}]));
    assert_eq!(cursors[0].range, "B2:C3");
    assert!(cursors_of(&json!(null)).is_empty());
    let mut t = page();
    live::deliver(&mut t.app, vec![LiveMsg::Cursors(cursors), LiveMsg::Leader(true), LiveMsg::Presence(false)]);
    let live = t.app.session.live.as_ref().unwrap();
    assert_eq!(live.cursors()[0].name, "An");
    assert!(live.is_leader());
}

#[test]
fn co_editing_texts_are_in_the_vietnamese_catalog() {
    use gridcraft_engine::live::{Feedback, LOCK_DENIED, LOCKED_BY, NOTHING_TO_UNDO};
    use gridcraft_ui_egui::i18n::Language;
    let vi = Language::Vi;
    let locked = Feedback { text: LOCKED_BY, name: "An Nguyễn".into() };
    assert_eq!(locked.fill(&vi.tr_message(LOCKED_BY)), "An Nguyễn đang sửa phần này.");
    for (en, want) in [
        ("Changed while you were away", "Có thay đổi khi bạn vắng mặt"),
        ("Keep mine", "Giữ bản của tôi"),
        ("Use theirs", "Dùng bản mới"),
    ] {
        assert_eq!(vi.tr(en), want);
    }
    for text in [LOCK_DENIED, NOTHING_TO_UNDO, "Someone else changed this part while you were offline. Which version do you keep?"] {
        assert_ne!(vi.tr_message(text), text, "{text}");
    }
}
