//! The fake room of the region co-editing tests: two simulated clients and a room that grants
//! locks like the server and delivers every update in one order.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::live::{Call, Live, Lock, Region};
use gridcraft_engine::model::{Sheet, Workbook};
use gridcraft_engine::{DocState, Session};

pub fn book() -> Workbook {
    let mut wb = Workbook::new();
    wb.sheets.push(Arc::new(Sheet::new("Two")));
    wb.fill_missing_sheet_uids();
    wb
}

pub fn client(prefix: &str, wb: Workbook) -> Session {
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
pub struct Room {
    pub clients: Vec<Session>,
    pub locks: BTreeMap<String, usize>,
    pub offline: Vec<bool>,
    /// Calls of offline clients, delivered when they come back.
    pub held: Vec<Vec<Call>>,
    pub log: Vec<(usize, String)>,
    pub clock: f64,
}

impl Room {
    pub fn new(wb: Workbook) -> Room {
        Room {
            clients: vec![client("aaaaaa", wb.clone()), client("bbbbbb", wb)],
            locks: BTreeMap::new(),
            offline: vec![false; 2],
            held: vec![vec![]; 2],
            log: vec![],
            clock: 0.0,
        }
    }

    pub fn clash(&self, who: usize, region: &str) -> bool {
        let r = Region::parse(region).unwrap();
        self.locks.iter().any(|(held, by)| *by != who && Region::parse(held).unwrap().clashes(&r))
    }

    pub fn covered(&self, who: usize, region: &str) -> bool {
        let r = Region::parse(region).unwrap();
        self.locks.iter().any(|(held, by)| *by == who && Region::parse(held).unwrap().covers(&r))
    }

    /// Delivers everything the clients asked for, until nothing moves.
    pub fn pump(&mut self) {
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

    pub fn update(&mut self, from: usize, region: &str, payload: &str) {
        assert!(self.covered(from, region) || self.offline_resent(from, region), "{region} sent without its lock");
        self.log.push((from, region.to_string()));
        for j in 0..self.clients.len() {
            if j != from {
                self.clients[j].live_apply_remote(region, payload).unwrap();
            }
        }
    }

    pub fn offline_resent(&mut self, from: usize, region: &str) -> bool {
        // A reconnected client locks first, as the shell's pump does.
        if self.clash(from, region) {
            return false;
        }
        self.locks.insert(region.to_string(), from);
        true
    }

    pub fn show_locks(&mut self) {
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

    pub fn reconnect(&mut self, who: usize) {
        self.offline[who] = false;
        for c in std::mem::take(&mut self.held[who]) {
            if let Call::Changed(r, p) = c {
                self.update(who, &r, &p);
            }
        }
        self.pump();
    }

    /// Hands back every lock as the clients would after moving on.
    pub fn settle(&mut self) {
        self.clock += 1_000_000.0;
        for s in &mut self.clients {
            s.live_tick(self.clock);
        }
        self.pump();
    }

    pub fn run(&mut self, who: usize, command: &str, params: serde_json::Value) {
        self.clients[who].execute(command, params).unwrap();
        self.pump();
    }
}

pub fn cell(a1: &str) -> CellRef {
    CellRef::parse(a1).unwrap()
}

pub fn input(s: &Session, sheet: usize, a1: &str) -> String {
    s.doc().unwrap().wb.sheet(sheet).and_then(|sh| sh.cell(cell(a1))).map(|c| c.input_text()).unwrap_or_default()
}

pub fn value(s: &Session, sheet: usize, a1: &str) -> Value {
    s.doc().unwrap().wb.sheet(sheet).and_then(|sh| sh.cell(cell(a1))).map(|c| c.value.clone()).unwrap_or(Value::Empty)
}

/// Both clients' cells (inputs and styles by value) and sheet lists.
pub fn same(room: &Room) {
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
