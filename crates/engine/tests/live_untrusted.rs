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

#[test]
fn a_sheet_payload_with_references_off_the_sheet_or_absurd_sizes_is_refused() {
    let mut room = Room::new(book());
    let honest = sheet_payload(&mut room);
    let cell = |row: u64, col: u64| json!({"row": row, "col": col});
    let line = |size: Json, outline: u64| json!({"size": size, "hidden": false, "outline": outline, "collapsed": false, "style": null});
    let values = json!({"Values": {"values": ["a"], "blanks": false}});
    let hostile: Vec<(&str, Json)> = vec![
        ("autofilter", json!({"range": {"start": cell(0, 0), "end": cell(0, 4_294_967_295)}, "criteria": []})),
        ("autofilter", json!({"range": {"start": cell(0, 0), "end": cell(9, 2)}, "criteria": [[4_294_967_295u64, values]]})),
        ("merges", json!([{"start": cell(5, 5), "end": cell(1, 1)}])),
        ("merges", json!([{"start": cell(0, 0), "end": cell(1_048_576, 0)}])),
        ("freeze", json!([4_294_967_295u64, 0])),
        ("default_row_height", json!(1e-30)),
        ("default_col_width", json!(-64.0)),
        ("default_col_width", json!(1e30)),
        ("rows", json!({"4294967295": line(json!(20.0), 0)})),
        ("cols", json!({"16384": line(Json::Null, 0)})),
        ("cols", json!({"3": line(json!(-1.0), 0)})),
        ("rows", json!({"3": line(json!(1e9), 0)})),
        ("rows", json!({"3": line(Json::Null, 200)})),
    ];
    for (key, value) in hostile {
        let mut p = honest.clone();
        p["s"][key] = value;
        refused(&mut room, "sheet:f1", &p.to_string());
    }
    room.clients[1].live_apply_remote("sheet:f1", &honest.to_string()).unwrap();
}

#[test]
fn a_sheet_payload_keeps_the_sheets_name_and_visibility() {
    // Renames and hiding take `*`: a duplicate name would make every later save fail.
    let mut room = Room::new(book());
    let mut p = sheet_payload(&mut room);
    p["s"]["name"] = json!("Two");
    p["s"]["visibility"] = json!("Hidden");
    room.clients[1].live_apply_remote("sheet:f1", &p.to_string()).unwrap();
    let sh = room.clients[1].doc().unwrap().wb.sheet(0).unwrap().clone();
    assert_eq!((sh.name.as_str(), sh.visibility), ("Sheet1", Visibility::Visible));
}

#[test]
fn a_sheet_payload_carries_structural_edits_of_its_own_sheet_only() {
    let mut room = Room::new(book());
    let honest = sheet_payload(&mut room);
    let first = |p: &mut Json| -> Json { p["e"][0][1][0].clone() };
    for (key, value) in [("s", "f2"), ("to", "f2")] {
        let mut p = honest.clone();
        let mut edit = first(&mut p);
        edit[key] = json!(value);
        p["e"][0][1][0] = edit;
        refused(&mut room, "sheet:f1", &p.to_string());
    }
}

#[test]
fn a_sheet_payload_changes_other_sheets_only_in_their_formulas() {
    let mut wb = book();
    Arc::make_mut(&mut wb.sheets[1]).set_value(cell("C3"), Value::Number(3.0));
    let mut room = Room::new(wb);
    let mut p = sheet_payload(&mut room);
    p["x"] = json!({"f2": {"C3": {"s": "overwritten"}, "D4": {"f": "1+1"}, "E5": null}});
    let _ = room.clients[1].live_apply_remote("sheet:f1", &p.to_string());
    assert_eq!(input(&room.clients[1], 1, "C3"), "3");
    assert_eq!(input(&room.clients[1], 1, "D4"), "");
}

#[test]
fn a_sheet_payload_cannot_add_or_drop_defined_names() {
    let mut wb = book();
    wb.names.push(DefinedName { name: "Rate".into(), scope: None, formula: "0.1".into(), comment: String::new(), hidden: false });
    let mut room = Room::new(wb);
    let mut p = sheet_payload(&mut room);
    p["n"] = json!([{"name": "Evil", "scope": null, "formula": "1", "comment": "", "hidden": false}]);
    let _ = room.clients[1].live_apply_remote("sheet:f1", &p.to_string());
    let names: Vec<String> = room.clients[1].doc().unwrap().wb.names.iter().map(|n| n.name.clone()).collect();
    assert_eq!(names, ["Rate"]);
}

#[test]
fn a_sheet_payload_with_too_many_structural_edits_is_refused() {
    // Each recorded edit stays in the structure history for good.
    let mut room = Room::new(book());
    let mut p = sheet_payload(&mut room);
    let edit = p["e"][0][1][0].clone();
    p["e"][0][1] = Json::Array(vec![edit; 5_000]);
    refused(&mut room, "sheet:f1", &p.to_string());
}

#[test]
fn an_oversized_payload_is_refused_unread() {
    let mut room = Room::new(book());
    let big = json!({"v": 1, "k": "cell", "t": null, "c": {"s": "x".repeat(3 << 20)}, "y": []});
    refused(&mut room, "cell:f1:A1", &big.to_string());
}
