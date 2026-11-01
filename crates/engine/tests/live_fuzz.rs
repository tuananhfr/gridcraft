//! Fuzzing what other members send: honest region payloads and comment anchors, randomly mutated
//! (values swapped for huge, negative or odd ones, keys dropped, text cut or garbled) and given to
//! `live_apply_remote` / `place_comment_anchor` under real and aliased region ids. Nothing may
//! panic; a refused payload leaves the document as it was; an applied one leaves a workbook that
//! saves and that the grid can draw (references on the sheet, sizes bounded).
//!
//! Knobs: `GRIDCRAFT_LIVE_FUZZ_CASES` (default 1500) and `GRIDCRAFT_LIVE_FUZZ_SEED` (replay one seed).

mod live_room;

use std::sync::Arc;

use gridcraft_engine::core::{CellRef, MAX_COLS, MAX_ROWS, RangeRef, Value};
use gridcraft_engine::live::Call;
use gridcraft_engine::live::anchor::Anchor;
use gridcraft_engine::model::Workbook;
use gridcraft_engine::{Session, io};
use live_room::*;
use serde_json::{Value as Json, json};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn pick<'a, T>(&mut self, list: &'a [T]) -> &'a T {
        &list[self.below(list.len())]
    }
}

fn odd_value(rng: &mut Rng) -> Json {
    let cell = |r: u64, c: u64| json!({"row": r, "col": c});
    match rng.below(16) {
        0 => Json::Null,
        1 => json!(u32::MAX),
        2 => json!(u64::MAX),
        3 => json!(-1),
        4 => json!(1e-30),
        5 => json!(1e300),
        6 => json!(""),
        7 => json!("zz:9:$A$0"),
        8 => json!([]),
        9 => json!({}),
        10 => cell(u64::from(MAX_ROWS), 0),
        11 => json!({"start": cell(9, 9), "end": cell(0, 0)}),
        12 => json!([[[[[[[[1]]]]]]]]),
        13 => json!("x".repeat(rng.below(5000))),
        14 => json!(true),
        _ => json!(rng.next() % 2_000_000),
    }
}

/// One random change somewhere in `v`.
fn mutate(rng: &mut Rng, v: &mut Json, depth: usize) {
    let descend = depth < 12 && rng.below(4) != 0;
    match v {
        Json::Object(m) if !m.is_empty() => {
            let keys: Vec<String> = m.keys().cloned().collect();
            let k = rng.pick(&keys).clone();
            match rng.below(6) {
                0 => {
                    m.remove(&k);
                }
                1 => {
                    m.insert(k, odd_value(rng));
                }
                _ if descend => mutate(rng, m.get_mut(&k).unwrap_or(&mut Json::Null), depth + 1),
                _ => {
                    m.insert(k, odd_value(rng));
                }
            }
        }
        Json::Array(list) if !list.is_empty() => {
            let i = rng.below(list.len());
            match rng.below(6) {
                0 => {
                    list.remove(i);
                }
                1 => {
                    let copy = list[i].clone();
                    list.extend(std::iter::repeat_n(copy, rng.below(64)));
                }
                _ if descend => mutate(rng, &mut list[i], depth + 1),
                _ => list[i] = odd_value(rng),
            }
        }
        _ => *v = odd_value(rng),
    }
}

fn numbers(v: &Json) -> usize {
    match v {
        Json::Number(_) => 1,
        Json::Array(list) => list.iter().map(numbers).sum(),
        Json::Object(m) => m.values().map(numbers).sum(),
        _ => 0,
    }
}

/// Swaps the `k`-th number in `v` for an odd one: row/column indexes, sizes and counts are where
/// a payload hurts the grid. Returns how many numbers are still to skip.
fn swap_number(rng: &mut Rng, v: &mut Json, k: usize) -> usize {
    match v {
        Json::Number(_) if k == 0 => {
            *v = rng.pick(&[json!(u32::MAX), json!(1_048_576), json!(16_384), json!(-1), json!(1e-30), json!(1e30), json!(1_000_000)]).clone();
            usize::MAX
        }
        Json::Number(_) => k - 1,
        Json::Array(list) => {
            list.iter_mut().try_fold(k, |k, x| if k == usize::MAX { None } else { Some(swap_number(rng, x, k)) }).unwrap_or(usize::MAX)
        }
        Json::Object(m) => m.values_mut().try_fold(k, |k, x| if k == usize::MAX { None } else { Some(swap_number(rng, x, k)) }).unwrap_or(usize::MAX),
        _ => k,
    }
}

/// The payload text, sometimes garbled as text rather than as JSON.
fn garble(rng: &mut Rng, text: String) -> String {
    let chars: Vec<char> = text.chars().collect();
    match rng.below(8) {
        0 => chars[..rng.below(chars.len())].iter().collect(),
        1 => {
            let mut c = chars;
            let i = rng.below(c.len());
            c.insert(i, *rng.pick(&['"', '{', ']', '\\', '\u{0}', 'é', '9']));
            c.into_iter().collect()
        }
        _ => text,
    }
}

/// Honest payloads of client A: (region, payload).
fn corpus(room: &mut Room) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let steps: [(&str, Json); 7] = [
        ("cell.set", json!({"cell": "B2", "input": "=SUM(A1:A3)"})),
        ("home.bold", json!({"range": "C3"})),
        ("home.insertRows", json!({"rows": "2:3"})),
        ("home.mergeCells", json!({"range": "D4:E6"})),
        ("view.freezePanes", json!({"cell": "B3"})),
        ("data.filter", json!({"range": "A1:C9"})),
        ("sheet.rename", json!({"sheet": "Two", "name": "Data"})),
    ];
    for (command, params) in steps {
        let _ = room.clients[0].execute(command, params);
        for _ in 0..10 {
            for c in room.clients[0].live.as_mut().unwrap().take_calls() {
                match c {
                    Call::WillEdit(r) => room.clients[0].live_answer(&r, true),
                    Call::Changed(r, p) => out.push((r, p)),
                    _ => {}
                }
            }
        }
    }
    out
}

fn snapshot(s: &Session) -> String {
    let wb = &s.doc().unwrap().wb;
    let sheets: Vec<Json> = wb.sheets.iter().map(|sh| serde_json::to_value(&**sh).unwrap()).collect();
    json!({"sheets": sheets, "names": serde_json::to_value(&wb.names).unwrap(), "seq": wb.structure_seq()}).to_string()
}

fn on_sheet(r: &RangeRef) -> bool {
    r.start.row <= r.end.row && r.start.col <= r.end.col && r.end.row < MAX_ROWS && r.end.col < MAX_COLS
}

/// What the grid loops over is bounded, and the workbook still saves without panicking.
fn drawable(wb: &Workbook) {
    for sh in &wb.sheets {
        assert!(sh.merges.iter().all(on_sheet), "{:?}", sh.merges);
        assert!(sh.autofilter.as_ref().is_none_or(|af| on_sheet(&af.range) && af.criteria.iter().all(|(o, _)| *o < af.range.width())));
        assert!(sh.freeze.is_none_or(|(r, c)| r < MAX_ROWS && c < MAX_COLS));
        for (at, l) in sh.rows.iter().chain(&sh.cols) {
            assert!(*at < MAX_ROWS && l.size.is_none_or(|s| (0.0..=4096.0).contains(&s)) && l.outline <= 7);
        }
        for s in [sh.default_row_height, sh.default_col_width] {
            assert!(s == 0.0 || (2.0..=4096.0).contains(&s), "{s}");
        }
    }
    let _ = io::save_bytes(wb, "fuzz.xlsx");
}

fn regions(rng: &mut Rng, honest: &str) -> String {
    let odd = [
        "cell:f1:b2",
        "cell:f1:$B$2",
        "cell:f1:B02",
        "cell:F1:B2",
        "sheet:f9",
        "cell:f1:XFE1",
        "cell:f1:A1048577",
        "*",
        "",
        "sheet:",
        "para:0000ABCD",
    ];
    if rng.below(3) == 0 { rng.pick(&odd).to_string() } else { honest.to_string() }
}

fn seed_count() -> (u64, u64) {
    if let Some(seed) = std::env::var("GRIDCRAFT_LIVE_FUZZ_SEED").ok().and_then(|s| s.parse().ok()) {
        return (seed, seed + 1);
    }
    let cases = std::env::var("GRIDCRAFT_LIVE_FUZZ_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(1500);
    (1, 1 + cases)
}

#[test]
fn mutated_payloads_never_panic_and_refusals_change_nothing() {
    let mut wb = book();
    for (a1, v) in [("A1", 1.0), ("A2", 2.0), ("A3", 3.0), ("C5", 5.0)] {
        Arc::make_mut(&mut wb.sheets[0]).set_value(CellRef::parse(a1).unwrap(), Value::Number(v));
    }
    let mut room = Room::new(wb);
    let honest = corpus(&mut room);
    assert!(honest.iter().any(|(r, _)| r.starts_with("cell:")) && honest.iter().any(|(r, _)| r.starts_with("sheet:")));
    assert!(honest.iter().any(|(r, _)| r == "*"));
    let (from, to) = seed_count();
    let mut applied = 0;
    for seed in from..to {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let (region, payload) = rng.pick(&honest).clone();
        let mut v: Json = serde_json::from_str(&payload).unwrap();
        for _ in 0..1 + rng.below(4) {
            if rng.below(2) == 0 {
                let k = rng.below(numbers(&v));
                swap_number(&mut rng, &mut v, k);
            } else {
                mutate(&mut rng, &mut v, 0);
            }
        }
        let text = garble(&mut rng, v.to_string());
        let region = regions(&mut rng, &region);
        let b = &mut room.clients[1];
        let before = snapshot(b);
        match b.live_apply_remote(&region, &text) {
            Err(_) => assert_eq!(snapshot(b), before, "seed {seed}: refused, yet changed ({region})"),
            Ok(()) => {
                applied += 1;
                drawable(&b.doc().unwrap().wb);
            }
        }
    }
    eprintln!("live fuzz: {applied} of {} mutated payloads applied", to - from);
    assert!(to - from < 50 || applied > 0, "no mutated payload was ever applied: the fuzzer only hits the parser");
}

#[test]
fn mutated_anchors_never_panic_and_change_nothing() {
    let mut room = Room::new(book());
    room.run(0, "cell.set", json!({"cell": "B3", "input": "hello"}));
    room.run(0, "selection.set", json!({"range": "B3:C5"}));
    let s = &room.clients[0];
    let honest = s.comment_anchor(s.doc().unwrap().uid).unwrap().to_json();
    let (from, to) = seed_count();
    for seed in from..to {
        let mut rng = Rng(seed.wrapping_mul(0xD6E8_FEB8_6659_FD93) | 1);
        let mut v = honest.clone();
        for _ in 0..1 + rng.below(4) {
            mutate(&mut rng, &mut v, 0);
        }
        let s = &room.clients[1];
        let before = snapshot(s);
        if let Some(anchor) = Anchor::parse(&v) {
            let _ = s.place_comment_anchor(s.doc().unwrap().uid, &anchor);
        }
        assert_eq!(snapshot(s), before, "seed {seed}");
    }
}
