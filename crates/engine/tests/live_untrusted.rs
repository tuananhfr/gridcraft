//! Region ids and payloads of other members reach this client through the server unchecked (it
//! only checks who holds which lock string): whatever a peer sends, the receiver keeps to what
//! the region allows and leaves its document as it was when it refuses.

mod live_room;

use std::sync::Arc;

use gridcraft_engine::core::Value;
use gridcraft_engine::live::{Call, Region};
use gridcraft_engine::model::{DefinedName, Visibility};
use gridcraft_engine::{Session, io};
use live_room::*;
use serde_json::{Value as Json, json};

/// What client `who` sends for `command`, its locks granted, without delivering it.
fn sent(room: &mut Room, who: usize, command: &str, params: Json) -> Vec<(String, String)> {
    room.clients[who].execute(command, params).unwrap();
    let mut out = Vec::new();
    for _ in 0..10 {
        let calls = room.clients[who].live.as_mut().unwrap().take_calls();
        for c in calls {
            match c {
                Call::WillEdit(r) => room.clients[who].live_answer(&r, true),
                Call::Changed(r, p) => out.push((r, p)),
                _ => {}
            }
        }
    }
    out
}

/// A's payload for inserting row 2 of Sheet1 (uid `f1`).
fn sheet_payload(room: &mut Room) -> Json {
    let sent = sent(room, 0, "home.insertRows", json!({"rows": "2:2"}));
    let (_, p) = sent.iter().find(|(r, _)| r == "sheet:f1").expect("a sheet payload");
    serde_json::from_str(p).unwrap()
}

/// Everything of a client's document that a payload could change.
fn snapshot(s: &Session) -> String {
    let wb = &s.doc().unwrap().wb;
    let sheets: Vec<Json> = wb.sheets.iter().map(|sh| serde_json::to_value(&**sh).unwrap()).collect();
    json!({"sheets": sheets, "names": serde_json::to_value(&wb.names).unwrap(), "seq": wb.structure_seq()}).to_string()
}

/// B refuses `payload` for `region` and its document stays as it was.
fn refused(room: &mut Room, region: &str, payload: &str) {
    let before = snapshot(&room.clients[1]);
    assert!(room.clients[1].live_apply_remote(region, payload).is_err(), "{region} {payload:.300}");
    assert_eq!(snapshot(&room.clients[1]), before, "{region}: refused, yet changed");
}

#[test]
fn aliases_of_a_cell_region_are_not_regions() {
    // The server compares lock strings: `b2` next to a held `B2` would be a second lock on one cell.
    for alias in ["cell:f1:b2", "cell:f1:$B$2", "cell:f1:B02", "cell:F1:B2", "cell:f1:B2 ", "sheet:F1", "sheet:f1 "] {
        assert_eq!(Region::parse(alias), None, "{alias}");
    }
    assert!(Region::parse("cell:f1:B2").is_some() && Region::parse("sheet:aaaaaa-1").is_some());
    let mut room = Room::new(book());
    let sent = sent(&mut room, 0, "cell.set", json!({"cell": "B2", "input": "x"}));
    let (region, p) = &sent[0];
    assert_eq!(region, "cell:f1:B2");
    refused(&mut room, "cell:f1:b2", p);
    refused(&mut room, "cell:f1:$B$2", p);
}
