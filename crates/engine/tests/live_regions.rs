//! Region co-editing (`erpHost` v3) between two simulated clients and a fake room that grants
//! locks like the server (`*`, `sheet:<uid>` over `cell:<uid>:…`) and delivers every update in
//! one order: edits of different regions converge, a held region refuses others, a payload
//! changes exactly its region, structural edits carry cell ids, and undo is per user.

use std::collections::BTreeMap;
use std::sync::Arc;

use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::live::{Call, Live, Lock, Region};
use gridcraft_engine::model::{Sheet, Workbook};
use gridcraft_engine::{DocState, Session, io};
use serde_json::json;

fn book() -> Workbook {
    let mut wb = Workbook::new();
    wb.sheets.push(Arc::new(Sheet::new("Two")));
    wb.fill_missing_sheet_uids();
    wb
}

fn client(prefix: &str, wb: Workbook) -> Session {
    let mut s = Session::new();
    assert!(s.set_client_prefix(prefix));
    s.add_document(DocState::new(wb, None, "Book".into())).unwrap();
    let uid = s.doc().unwrap().uid;
    let mut live = Live::new(uid, prefix, Some("v1".into()));
    live.set_online(true);
    s.live = Some(live);
    s
}

/// The fake room: who holds which lock, and whether a client is connected.
struct Room {
    clients: Vec<Session>,
    locks: BTreeMap<String, usize>,
    offline: Vec<bool>,
    /// Calls of offline clients, delivered when they come back.
    held: Vec<Vec<Call>>,
    log: Vec<(usize, String)>,
    clock: f64,
}

impl Room {
    fn new(wb: Workbook) -> Room {
        Room {
            clients: vec![client("aaaaaa", wb.clone()), client("bbbbbb", wb)],
            locks: BTreeMap::new(),
            offline: vec![false; 2],
            held: vec![vec![]; 2],
            log: vec![],
            clock: 0.0,
        }
    }

    fn clash(&self, who: usize, region: &str) -> bool {
        let r = Region::parse(region).unwrap();
        self.locks.iter().any(|(held, by)| *by != who && Region::parse(held).unwrap().clashes(&r))
    }

    fn covered(&self, who: usize, region: &str) -> bool {
        let r = Region::parse(region).unwrap();
        self.locks.iter().any(|(held, by)| *by == who && Region::parse(held).unwrap().covers(&r))
    }

    /// Delivers everything the clients asked for, until nothing moves.
    fn pump(&mut self) {
        for _ in 0..50 {
            let mut moved = false;
            for i in 0..self.clients.len() {
                let mut calls = self.clients[i].live.as_mut().unwrap().take_calls();
                if self.offline[i] {
                    // Offline the shell grants every edit without a lock and keeps the updates.
                    for c in &calls {
                        if let Call::WillEdit(r) = c {
                            self.clients[i].live_answer(r, true);
                        }
                    }
                    self.held[i].extend(calls.drain(..).filter(|c| matches!(c, Call::Changed(..))));
                    continue;
                }
                for c in calls {
                    moved = true;
                    match c {
                        Call::WillEdit(r) => {
                            let granted = !self.clash(i, &r);
                            if granted {
                                self.locks.insert(r.clone(), i);
                            }
                            self.clients[i].live_answer(&r, granted);
                        }
                        Call::Changed(r, payload) => self.update(i, &r, &payload),
                        Call::DidEndEdit(r) => {
                            if self.locks.get(&r) == Some(&i) {
                                self.locks.remove(&r);
                            }
                        }
                        Call::Selection(..) => {}
                    }
                }
            }
            self.show_locks();
            if !moved {
                return;
            }
        }
        panic!("the room never settled");
    }

    fn update(&mut self, from: usize, region: &str, payload: &str) {
        assert!(self.covered(from, region) || self.offline_resent(from, region), "{region} sent without its lock");
        self.log.push((from, region.to_string()));
        for j in 0..self.clients.len() {
            if j != from {
                self.clients[j].live_apply_remote(region, payload).unwrap();
            }
        }
    }

    fn offline_resent(&mut self, from: usize, region: &str) -> bool {
        // A reconnected client locks first, as the shell's pump does.
        if self.clash(from, region) {
            return false;
        }
        self.locks.insert(region.to_string(), from);
        true
    }

    fn show_locks(&mut self) {
        for i in 0..self.clients.len() {
            let others = self
                .locks
                .iter()
                .filter(|(_, by)| **by != i)
                .map(|(r, by)| Lock { region: r.clone(), holder: format!("m{by}"), name: format!("User {by}"), colour: "#3366ff".into() })
                .collect();
            let mine: Vec<String> = self.locks.iter().filter(|(_, by)| **by == i).map(|(r, _)| r.clone()).collect();
            self.clients[i].live.as_mut().unwrap().set_locks(others, &mine);
        }
    }

    fn reconnect(&mut self, who: usize) {
        self.offline[who] = false;
        for c in std::mem::take(&mut self.held[who]) {
            if let Call::Changed(r, p) = c {
                self.update(who, &r, &p);
            }
        }
        self.pump();
    }

    /// Hands back every lock as the clients would after moving on.
    fn settle(&mut self) {
        self.clock += 1_000_000.0;
        for s in &mut self.clients {
            s.live_tick(self.clock);
        }
        self.pump();
    }

    fn run(&mut self, who: usize, command: &str, params: serde_json::Value) {
        self.clients[who].execute(command, params).unwrap();
        self.pump();
    }
}

fn cell(a1: &str) -> CellRef {
    CellRef::parse(a1).unwrap()
}

fn input(s: &Session, sheet: usize, a1: &str) -> String {
    s.doc().unwrap().wb.sheet(sheet).and_then(|sh| sh.cell(cell(a1))).map(|c| c.input_text()).unwrap_or_default()
}

fn value(s: &Session, sheet: usize, a1: &str) -> Value {
    s.doc().unwrap().wb.sheet(sheet).and_then(|sh| sh.cell(cell(a1))).map(|c| c.value.clone()).unwrap_or(Value::Empty)
}

/// Both clients' cells (inputs and styles by value) and sheet lists.
fn same(room: &Room) {
    let dump = |s: &Session| {
        let wb = &s.doc().unwrap().wb;
        wb.sheets
            .iter()
            .map(|sh| {
                let cells: Vec<_> = sh.cells.iter().map(|(at, c)| (at, c.input_text(), wb.styles.get(c.style).clone())).collect();
                (sh.uid.clone(), sh.name.clone(), cells, sh.merges.clone())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(dump(&room.clients[0]), dump(&room.clients[1]));
}

#[test]
fn edits_of_different_regions_converge() {
    let mut room = Room::new(book());
    room.run(0, "cell.set", json!({"cell": "A1", "input": "5"}));
    room.run(1, "cell.set", json!({"cell": "B1", "input": "=A1*2"}));
    room.run(1, "sheet.activate", json!({"sheet": "Two"}));
    room.run(1, "cell.set", json!({"cell": "C3", "input": "hello"}));
    room.settle();
    same(&room);
    assert_eq!(input(&room.clients[1], 0, "A1"), "5");
    // Recalculation runs where the payload lands.
    assert_eq!(value(&room.clients[0], 0, "B1"), Value::Number(10.0));
    assert_eq!(input(&room.clients[0], 1, "C3"), "hello");
    assert_eq!(room.log, [(0, "cell:f1:A1".to_string()), (1, "cell:f1:B1".into()), (1, "cell:f2:C3".into())]);
}

#[test]
fn a_region_someone_holds_refuses_the_others() {
    let mut room = Room::new(book());
    // A opens the in-cell editor on B2: its lock is taken before the first keystroke lands.
    room.clients[0].live_begin_cell_edit(cell("B2")).unwrap();
    room.pump();
    assert!(room.clients[0].live.as_ref().unwrap().holds("cell:f1:B2"));
    // B sees the lock and can't even start editing there.
    assert_eq!(room.clients[1].live_begin_cell_edit(cell("B2")), Err("User 0".into()));
    // A command on that cell is taken back at once, with feedback.
    room.run(1, "cell.set", json!({"cell": "B2", "input": "theirs"}));
    assert_eq!(input(&room.clients[1], 0, "B2"), "");
    assert!(room.clients[1].live.as_mut().unwrap().take_feedback().unwrap().contains("User 0"));
    // A whole-sheet edit clashes with the held cell too.
    room.run(1, "home.insertRows", json!({"rows": "1:1"}));
    assert_eq!(room.clients[1].doc().unwrap().wb.structure_seq(), 0);
    room.run(0, "cell.set", json!({"cell": "B2", "input": "mine"}));
    room.clients[0].live_end_cell_edit();
    room.settle();
    same(&room);
    assert_eq!(input(&room.clients[1], 0, "B2"), "mine");
}

#[test]
fn a_denied_lock_takes_the_edit_back() {
    let mut room = Room::new(book());
    // B's view of the locks is stale: the room denies and the edit is rolled back.
    room.locks.insert("cell:f1:D4".into(), 0);
    room.run(1, "cell.set", json!({"cell": "D4", "input": "late"}));
    assert_eq!(input(&room.clients[1], 0, "D4"), "");
    assert!(room.clients[1].live.as_mut().unwrap().take_feedback().is_some());
    assert!(room.log.is_empty());
}

#[test]
fn a_payload_changes_exactly_its_region() {
    let mut wb = book();
    for (a1, v) in [("A1", 1.0), ("A2", 2.0), ("C5", 3.0)] {
        Arc::make_mut(&mut wb.sheets[0]).set_value(cell(a1), Value::Number(v));
    }
    let mut room = Room::new(wb);
    // B's own region in progress stays as it is while A's payload lands.
    room.clients[1].live_begin_cell_edit(cell("C5")).unwrap();
    room.pump();
    room.run(0, "cell.set", json!({"cell": "A2", "input": "20"}));
    room.run(0, "home.bold", json!({"range": "A1"}));
    let b = &room.clients[1].doc().unwrap().wb;
    assert_eq!(value(&room.clients[1], 0, "A2"), Value::Number(20.0));
    assert!(b.styles.get(b.sheet(0).unwrap().cell(cell("A1")).unwrap().style).font.bold);
    assert_eq!(value(&room.clients[1], 0, "C5"), Value::Number(3.0));
    assert_eq!(b.sheet(0).unwrap().cells.len(), 3);
    same(&room);
}

#[test]
fn structural_edits_take_the_sheet_and_carry_formulas_elsewhere() {
    let mut wb = book();
    Arc::make_mut(&mut wb.sheets[0]).set_value(cell("A5"), Value::Number(7.0));
    let mut room = Room::new(wb);
    room.run(1, "sheet.activate", json!({"sheet": "Two"}));
    room.run(1, "cell.set", json!({"cell": "B1", "input": "=Sheet1!A5"}));
    room.settle();
    // A inserts rows above A5 while B keeps editing Two: the formula follows on both sides.
    room.run(0, "home.insertRows", json!({"rows": "2:3"}));
    room.run(1, "cell.set", json!({"cell": "B2", "input": "elsewhere"}));
    room.settle();
    same(&room);
    assert!(room.log.iter().any(|(who, r)| *who == 0 && r == "sheet:f1"));
    assert_eq!(input(&room.clients[1], 0, "A7"), "7");
    assert_eq!(input(&room.clients[1], 1, "B1"), "=Sheet1!A7");
    assert_eq!(value(&room.clients[1], 1, "B1"), Value::Number(7.0));
    // B then edits the moved cell by its new address, and deletes rows: both follow.
    room.run(1, "sheet.activate", json!({"sheet": "Sheet1"}));
    room.run(1, "cell.set", json!({"cell": "A7", "input": "8"}));
    room.run(1, "home.deleteRows", json!({"rows": "1:2"}));
    room.settle();
    same(&room);
    assert_eq!(input(&room.clients[0], 0, "A5"), "8");
    assert_eq!(input(&room.clients[0], 1, "B1"), "=Sheet1!A5");
}

#[test]
fn an_offline_cell_edit_lands_where_a_remote_row_insert_moved_it() {
    let mut wb = book();
    Arc::make_mut(&mut wb.sheets[0]).set_value(cell("D10"), Value::Number(1.0));
    let mut room = Room::new(wb);
    room.offline[1] = true;
    room.clients[1].live.as_mut().unwrap().set_online(false);
    room.run(1, "cell.set", json!({"cell": "D10", "input": "offline"}));
    room.run(0, "home.insertRows", json!({"rows": "1:2"}));
    room.settle();
    // Back online: the room replays A's insert first, then B's held edit goes out.
    room.clients[1].live.as_mut().unwrap().set_online(true);
    room.reconnect(1);
    room.settle();
    assert_eq!(input(&room.clients[0], 0, "D12"), "offline");
    assert_eq!(input(&room.clients[1], 0, "D12"), "offline");
    same(&room);
}

#[test]
fn whole_workbook_edits_take_the_star_lock() {
    let mut room = Room::new(book());
    room.run(0, "sheet.rename", json!({"sheet": "Two", "name": "Data"}));
    room.run(0, "home.insertSheet", json!({}));
    room.settle();
    assert!(room.log.iter().all(|(_, r)| r == "*"));
    same(&room);
    assert_eq!(room.clients[1].doc().unwrap().wb.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().len(), 3);
    // While someone edits a cell, nobody gets `*`.
    room.clients[1].live_begin_cell_edit(cell("A1")).unwrap();
    room.pump();
    room.run(0, "sheet.rename", json!({"sheet": "Data", "name": "Other"}));
    assert!(room.clients[0].doc().unwrap().wb.sheets.iter().any(|s| s.name == "Data"));
}

#[test]
fn undo_takes_back_only_this_users_steps() {
    let mut room = Room::new(book());
    room.run(0, "cell.set", json!({"cell": "A1", "input": "a1"}));
    room.run(0, "cell.set", json!({"cell": "A2", "input": "a2"}));
    room.settle();
    // B overwrites A's A2 afterwards and adds its own cell.
    room.run(1, "cell.set", json!({"cell": "A2", "input": "b2"}));
    room.run(1, "cell.set", json!({"cell": "B1", "input": "b1"}));
    room.settle();
    // A's undo skips the A2 step (B changed it) and takes back A1; B's cells stay.
    room.run(0, "edit.undo", json!({}));
    room.settle();
    same(&room);
    assert_eq!(input(&room.clients[1], 0, "A1"), "");
    assert_eq!(input(&room.clients[1], 0, "A2"), "b2");
    assert_eq!(input(&room.clients[1], 0, "B1"), "b1");
    room.run(0, "edit.undo", json!({}));
    assert!(room.clients[0].live.as_mut().unwrap().take_feedback().is_some(), "nothing of A's left");
    // Redo brings A1 back everywhere.
    room.run(0, "edit.redo", json!({}));
    room.settle();
    same(&room);
    assert_eq!(input(&room.clients[1], 0, "A1"), "a1");
}

#[test]
fn undo_of_a_row_insert_puts_the_rows_back_everywhere() {
    let mut wb = book();
    Arc::make_mut(&mut wb.sheets[0]).set_value(cell("A3"), Value::Number(3.0));
    let mut room = Room::new(wb);
    room.run(0, "home.insertRows", json!({"rows": "1:1"}));
    room.settle();
    assert_eq!(input(&room.clients[1], 0, "A4"), "3");
    room.run(0, "edit.undo", json!({}));
    room.settle();
    same(&room);
    assert_eq!(input(&room.clients[1], 0, "A3"), "3");
    // B's later cell edit by address lands right after the undo.
    room.run(1, "cell.set", json!({"cell": "A3", "input": "4"}));
    room.settle();
    assert_eq!(input(&room.clients[0], 0, "A3"), "4");
}

#[test]
fn ids_survive_save_and_open_after_remote_applies() {
    let mut room = Room::new(book());
    room.run(0, "home.insertSheet", json!({}));
    room.run(1, "cell.set", json!({"cell": "A1", "input": "x"}));
    room.settle();
    let uids = |s: &Session| s.doc().unwrap().wb.sheets.iter().map(|sh| sh.uid.clone()).collect::<Vec<_>>();
    let before = uids(&room.clients[1]);
    let bytes = io::save_bytes(&room.clients[1].doc().unwrap().wb, "b.xlsx").unwrap();
    let (wb, _) = io::open_bytes("b.xlsx", &bytes).unwrap();
    assert_eq!(wb.sheets.iter().map(|s| s.uid.clone()).collect::<Vec<_>>(), before);
    assert_eq!(before, uids(&room.clients[0]));
}

#[test]
fn without_live_nothing_changes() {
    let mut s = Session::new();
    s.add_document(DocState::new(book(), None, "Book".into())).unwrap();
    s.execute("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    s.execute("cell.set", json!({"cell": "A1", "input": "2"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(input(&s, 0, "A1"), "1");
    assert!(s.live_begin_cell_edit(cell("A1")).is_ok());
}

#[test]
fn our_selection_is_reported_once_per_change() {
    let mut s = client("cccccc", book());
    s.live_tick(0.0);
    s.execute("selection.set", json!({"range": "B2:C3"})).ok();
    s.live_tick(10.0);
    s.live_tick(20.0);
    let calls: Vec<_> = s.live.as_mut().unwrap().take_calls().into_iter().filter(|c| matches!(c, Call::Selection(..))).collect();
    assert_eq!(calls.first(), Some(&Call::Selection("cell:f1:A1".into(), "A1".into())));
    assert!(calls.len() <= 2);
}
