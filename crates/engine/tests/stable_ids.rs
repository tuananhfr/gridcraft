//! Stable ids for live editing: sheet uids and cell ids (coordinates at a structure sequence
//! number) survive edits, undo/redo and save/open, and two clients never make the same id.

use gridcraft_engine::model::{CellAt, CellId, Workbook};
use gridcraft_engine::{DocState, Session, core::CellRef, io};
use serde_json::json;

fn open(prefix: &str, wb: Workbook) -> Session {
    let mut s = Session::new();
    assert!(s.set_client_prefix(prefix));
    s.add_document(DocState::new(wb, None, "Book".into())).unwrap();
    s
}

fn reopen(prefix: &str, s: &Session) -> Session {
    let bytes = io::save_bytes(&s.doc().unwrap().wb, "book.xlsx").unwrap();
    open(prefix, io::open_bytes("book.xlsx", &bytes).unwrap().0)
}

fn uids(s: &Session) -> Vec<(String, String)> {
    s.doc().unwrap().wb.sheets.iter().map(|sh| (sh.name.clone(), sh.uid.clone())).collect()
}

fn cell(a1: &str) -> CellRef {
    CellRef::parse(a1).unwrap()
}

fn id(s: &Session, sheet: usize, a1: &str) -> CellId {
    s.doc().unwrap().wb.cell_id(sheet, cell(a1)).unwrap()
}

fn now(s: &Session, id: &CellId) -> Option<CellRef> {
    match s.doc().unwrap().wb.locate(id) {
        CellAt::At(at) => Some(at.cell),
        CellAt::Deleted => None,
        CellAt::UnknownSeq => panic!("unknown seq {}", id.seq),
    }
}

fn run(s: &mut Session, command: &str, params: serde_json::Value) {
    s.execute(command, params).unwrap();
}

#[test]
fn opened_sheets_get_position_uids_and_new_ones_carry_the_prefix() {
    let mut s = open("0a0b0c", Workbook::new());
    assert_eq!(uids(&s), [("Sheet1".into(), "f1".into())]);
    run(&mut s, "home.insertSheet", json!({}));
    run(&mut s, "sheet.move", json!({"sheet": "Sheet1", "to": 0, "copy": true}));
    run(&mut s, "sheet.rename", json!({"sheet": "Sheet1", "name": "Data"}));
    let mut got = uids(&s);
    got.sort();
    assert_eq!(got.iter().filter(|(_, u)| u == "f1").map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Data"], "{got:?}");
    assert!(got.iter().filter(|(_, u)| u != "f1").all(|(_, u)| u.starts_with("0a0b0c-")), "{got:?}");
    assert_eq!(got.iter().map(|(_, u)| u).collect::<std::collections::HashSet<_>>().len(), 3);
}

#[test]
fn sheet_uids_survive_save_open_edits_and_undo() {
    let mut s = open("111111", Workbook::new());
    run(&mut s, "home.insertSheet", json!({}));
    let before = uids(&s);
    // Another client opens the saved file: same uids, whatever its own prefix.
    let mut t = reopen("222222", &s);
    assert_eq!(uids(&t), before);
    run(&mut t, "home.insertRows", json!({"rows": "1:2"}));
    run(&mut t, "cell.set", json!({"cell": "A1", "input": "x"}));
    run(&mut t, "edit.undo", json!({}));
    run(&mut t, "edit.redo", json!({}));
    assert_eq!(uids(&t), before, "edits never rewrite existing uids");
    assert_eq!(uids(&reopen("333333", &t)), before);
}

#[test]
fn two_clients_adding_sheets_at_once_never_collide() {
    let base = io::save_bytes(&open("abcdef", Workbook::new()).doc().unwrap().wb, "b.xlsx").unwrap();
    let mut a = open("aaaaaa", io::open_bytes("b.xlsx", &base).unwrap().0);
    let mut b = open("aaaaab", io::open_bytes("b.xlsx", &base).unwrap().0);
    let mut all = std::collections::HashSet::new();
    for _ in 0..20 {
        run(&mut a, "home.insertSheet", json!({}));
        run(&mut b, "home.insertSheet", json!({}));
    }
    for (_, uid) in uids(&a).into_iter().chain(uids(&b)).filter(|(_, u)| u != "f1") {
        assert!(all.insert(uid.clone()), "{uid} made twice");
    }
    assert_eq!(all.len(), 40);
}

#[test]
fn cell_ids_follow_row_and_column_inserts_and_deletes() {
    let mut s = open("c0ffee", Workbook::new());
    let c5 = id(&s, 0, "C5");
    let a1 = id(&s, 0, "A1");
    assert_eq!(c5.seq, 0);
    run(&mut s, "home.insertRows", json!({"rows": "2:3"}));
    assert_eq!(now(&s, &c5), Some(cell("C7")));
    assert_eq!(now(&s, &a1), Some(cell("A1")));
    run(&mut s, "home.insertColumns", json!({"cols": "A:A"}));
    assert_eq!(now(&s, &c5), Some(cell("D7")));
    let seq = s.doc().unwrap().wb.structure_seq();
    assert_eq!(seq, 2, "each structural edit bumps the number");
    run(&mut s, "cell.set", json!({"cell": "D7", "input": "5"}));
    assert_eq!(s.doc().unwrap().wb.structure_seq(), seq, "content edits don't");
    run(&mut s, "home.deleteRows", json!({"rows": "7:7"}));
    assert_eq!(now(&s, &c5), None);
    assert_eq!(now(&s, &a1), Some(cell("B1")));
    run(&mut s, "home.deleteCells", json!({"range": "B1", "shift": "left"}));
    assert_eq!(now(&s, &a1), None);
}

#[test]
fn cell_ids_follow_undo_and_redo() {
    let mut s = open("c0ffee", Workbook::new());
    let b2 = id(&s, 0, "B2");
    run(&mut s, "home.insertRows", json!({"rows": "1:1"}));
    let b3 = id(&s, 0, "B3");
    let new_row = id(&s, 0, "B1");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().wb.structure_seq(), 0);
    assert_eq!(now(&s, &b3), Some(cell("B2")));
    assert_eq!(now(&s, &new_row), None, "the undone row is gone");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(now(&s, &b2), Some(cell("B3")));
    assert_eq!(now(&s, &new_row), Some(cell("B1")));
    // A different edit after an undo is a new branch with its own number.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "home.insertColumns", json!({"cols": "A:B"}));
    assert_eq!(s.doc().unwrap().wb.structure_seq(), 2);
    assert_eq!(now(&s, &b3), Some(cell("D2")));
    assert_eq!(now(&s, &new_row), None);
}

#[test]
fn cut_and_paste_carries_cell_ids_to_the_destination() {
    let mut s = open("c0ffee", Workbook::new());
    run(&mut s, "home.insertSheet", json!({"name": "Dest"}));
    run(&mut s, "sheet.activate", json!({"sheet": "Sheet1"}));
    let b2 = id(&s, 0, "B2");
    let dest = s.doc().unwrap().wb.sheets.iter().find(|sh| sh.name == "Dest").unwrap().uid.clone();
    run(&mut s, "cell.set", json!({"cell": "B2", "input": "7"}));
    run(&mut s, "edit.cut", json!({"range": "A1:B2"}));
    run(&mut s, "sheet.activate", json!({"sheet": "Dest"}));
    run(&mut s, "edit.paste", json!({"at": "D10"}));
    let CellAt::At(moved) = s.doc().unwrap().wb.locate(&b2) else { panic!("lost") };
    assert_eq!((moved.sheet, moved.cell), (dest, cell("E11")));
}
