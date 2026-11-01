//! Anchors of Cloud comments (M5) between two co-editing clients: an anchor taken on one client
//! is placed again on either after the other's edits (kept, moved by inserted rows, deleted and
//! found again by its content, orphaned), and on a fresh copy of a later version.

mod live_room;

use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::live::Live;
use gridcraft_engine::live::anchor::{Anchor, CONTENT_MAX, Placed};
use gridcraft_engine::{DocState, Session, io};
use live_room::*;
use serde_json::json;

fn doc(s: &Session) -> u64 {
    s.doc().unwrap().uid
}

/// Client `who` selects `range` on its active sheet and takes the anchor of it.
fn anchor_of(room: &mut Room, who: usize, range: &str) -> Anchor {
    room.clients[who].execute("selection.set", json!({"range": range})).unwrap();
    let s = &room.clients[who];
    s.comment_anchor(doc(s)).expect("an anchor")
}

fn place(s: &Session, anchor: &Anchor) -> Placed {
    s.place_comment_anchor(doc(s), anchor)
}

/// The range an anchor is placed on, on sheet `sheet`.
fn at(s: &Session, anchor: &Anchor) -> Option<(usize, String)> {
    match place(s, anchor) {
        Placed::At { sheet, range, .. } => Some((sheet, range.a1())),
        Placed::Orphaned => None,
    }
}

fn seeded(cells: &[(&str, &str)]) -> Room {
    let mut room = Room::new(book());
    for (a1, text) in cells {
        room.run(0, "cell.set", json!({"cell": a1, "input": text}));
    }
    room.settle();
    room
}

/// A copy of client `who`'s workbook reopened from its saved bytes, as a later version opens.
fn reopened(room: &Room, who: usize, live: bool) -> Session {
    let bytes = io::save_bytes(&room.clients[who].doc().unwrap().wb, "b.xlsx").unwrap();
    let (wb, _) = io::open_bytes("b.xlsx", &bytes).unwrap();
    let mut s = Session::new();
    s.add_document(DocState::new(wb, None, "Book".into())).unwrap();
    if live {
        let uid = doc(&s);
        s.live = Some(Live::new(uid, "cccccc", Some("v2".into())));
    }
    s
}

#[test]
fn an_anchor_names_the_cell_region_its_layout_and_content() {
    let mut room = seeded(&[("B3", "=1+2")]);
    let a = anchor_of(&mut room, 0, "B3");
    assert_eq!(a.region, "cell:f1:B3");
    assert_eq!(a.pos.t.as_deref(), Some("v1"));
    assert_eq!(a.pos.range, None);
    assert_eq!((a.soft.sheet.as_str(), a.soft.a1.as_str(), a.soft.content.as_deref()), ("Sheet1", "B3", Some("=1+2")));
    assert_eq!((a.label.as_str(), a.quote.as_str(), a.order.as_slice()), ("Sheet1!B3", "Sheet1!B3", &[0, 2, 1][..]));
    assert_eq!(Anchor::parse(&a.to_json()), Some(a.clone()));
    // Unchanged content places it where it was, reading the same.
    for s in &room.clients {
        assert_eq!(place(s, &a), Placed::At { sheet: 0, range: RangeRef::parse("B3").unwrap(), anchor: Box::new(a.clone()) });
    }
    let range = anchor_of(&mut room, 1, "B3:C5");
    assert_eq!((range.region.as_str(), range.pos.range.as_deref(), range.label.as_str()), ("cell:f1:B3", Some("B3:C5"), "Sheet1!B3:C5"));
    assert_eq!(at(&room.clients[0], &range), Some((0, "B3:C5".into())));
}

#[test]
fn no_anchor_without_the_document_or_off_its_active_copy() {
    let mut room = seeded(&[]);
    let s = &room.clients[0];
    assert!(s.comment_anchor(doc(s) + 1000).is_none());
    room.clients[0].execute("sheet.activate", json!({"sheet": "Two"})).unwrap();
    let a = anchor_of(&mut room, 0, "D4");
    assert_eq!(a.region, "cell:f2:D4");
    assert_eq!(a.order, [1, 3, 3]);
}

#[test]
fn rows_inserted_by_the_other_client_move_the_anchor_on_both() {
    let mut room = seeded(&[("B3", "keep"), ("C5", "end")]);
    let cell = anchor_of(&mut room, 0, "B3");
    let range = anchor_of(&mut room, 0, "B3:C5");
    room.run(1, "home.insertRows", json!({"rows": "1:2"}));
    room.settle();
    for s in &room.clients {
        assert_eq!(at(s, &cell), Some((0, "B5".into())));
        assert_eq!(at(s, &range), Some((0, "B5:C7".into())));
        let Placed::At { anchor, .. } = place(s, &cell) else { panic!("lost") };
        assert_eq!(anchor.region, "cell:f1:B5");
        assert_ne!(anchor.pos.t.as_deref(), Some("v1"), "the anchor takes the layout it is now at");
        // Placed again from its new form, it stays.
        assert_eq!(at(s, &anchor), Some((0, "B5".into())));
    }
    // A row inserted inside the range grows it.
    room.run(0, "home.insertRows", json!({"rows": "6:6"}));
    room.settle();
    assert_eq!(at(&room.clients[1], &range), Some((0, "B5:C8".into())));
}

#[test]
fn a_changed_cell_keeps_its_threads_and_the_soft_content_follows() {
    let mut room = seeded(&[("B3", "draft")]);
    let a = anchor_of(&mut room, 0, "B3");
    room.run(1, "cell.set", json!({"cell": "B3", "input": "final"}));
    room.settle();
    let Placed::At { range, anchor, .. } = place(&room.clients[0], &a) else { panic!("lost") };
    assert_eq!(range.a1(), "B3");
    assert_eq!(anchor.soft.content.as_deref(), Some("final"));
}

#[test]
fn a_deleted_cell_is_found_again_by_its_content_or_orphaned() {
    let mut room = seeded(&[("B3", "total"), ("E9", "total"), ("B4", "gone")]);
    let moved = anchor_of(&mut room, 0, "B3");
    let lost = anchor_of(&mut room, 0, "B4");
    let empty = anchor_of(&mut room, 0, "D4");
    room.run(1, "home.deleteRows", json!({"rows": "3:4"}));
    room.settle();
    for s in &room.clients {
        assert_eq!(at(s, &moved), Some((0, "E7".into())), "the same content, nearest the old place");
        assert_eq!(at(s, &lost), None);
        assert_eq!(at(s, &empty), None, "an empty cell can't be searched for");
    }
}

#[test]
fn a_deleted_sheet_orphans_and_a_renamed_one_relabels() {
    let mut room = seeded(&[]);
    room.clients[0].execute("sheet.activate", json!({"sheet": "Two"})).unwrap();
    let a = anchor_of(&mut room, 0, "A1");
    room.run(1, "sheet.rename", json!({"sheet": "Two", "name": "Data"}));
    room.settle();
    let Placed::At { anchor, .. } = place(&room.clients[0], &a) else { panic!("lost") };
    assert_eq!((anchor.label.as_str(), anchor.soft.sheet.as_str()), ("Data!A1", "Data"));
    room.run(1, "home.deleteSheet", json!({"sheet": "Data"}));
    room.settle();
    assert_eq!(at(&room.clients[0], &a), None);
    assert_eq!(at(&room.clients[1], &a), None);
}

#[test]
fn a_later_version_places_anchors_by_their_soft_part() {
    let long = "y".repeat(CONTENT_MAX + 10);
    let mut room = seeded(&[("B3", "same"), ("C3", "moves"), ("D3", "changes"), ("E3", long.as_str())]);
    let same = anchor_of(&mut room, 0, "B3");
    let moves = anchor_of(&mut room, 0, "C3");
    let changes = anchor_of(&mut room, 0, "D3");
    let empty = anchor_of(&mut room, 0, "F3");
    let hashed = anchor_of(&mut room, 0, "E3");
    assert!(hashed.soft.content.is_none() && hashed.soft.hash.is_some());
    room.run(1, "cell.set", json!({"cell": "C3", "input": ""}));
    room.run(1, "cell.set", json!({"cell": "C8", "input": "moves"}));
    room.run(1, "cell.set", json!({"cell": "D3", "input": "other"}));
    room.run(1, "cell.set", json!({"cell": "F3", "input": "filled"}));
    room.settle();
    for live in [true, false] {
        let s = reopened(&room, 1, live);
        assert_eq!(at(&s, &same), Some((0, "B3".into())));
        assert_eq!(at(&s, &moves), Some((0, "C8".into())));
        assert_eq!(at(&s, &changes), None);
        assert_eq!(at(&s, &empty), Some((0, "F3".into())), "an empty cell stays where it was");
        assert_eq!(at(&s, &hashed), Some((0, "E3".into())));
        let Placed::At { anchor, .. } = place(&s, &same) else { panic!("lost") };
        assert_eq!(anchor.pos.t.as_deref(), live.then_some("v2"));
    }
}

#[test]
fn a_cell_inside_a_merge_anchors_the_merge_as_one_cell() {
    let mut room = seeded(&[]);
    room.run(0, "home.mergeCells", json!({"range": "B2:C3"}));
    room.settle();
    let a = anchor_of(&mut room, 1, "B2:C3");
    assert_eq!((a.pos.range.as_deref(), a.label.as_str()), (None, "Sheet1!B2"));
    assert_eq!(CellRef::parse(&a.soft.a1), Some(CellRef::new(1, 1)));
}
