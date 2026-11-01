//! Region payloads, version 1: compact JSON strings. Each one holds the whole current content of
//! its region (the shell replaces a region's unsent payload with a newer one).
//!
//! - cell: `{"v":1,"k":"cell","t":token|null,"c":cell|null,"y":[style]}`
//! - sheet: `{"v":1,"k":"sheet","b":token|null,"e":[[token,[edit…]]…],"s":sheet,"c":{A1:cell},
//!   "x":{uid:{A1:cell|null}},"n":[name…]?,"y":[style…]}`
//! - book: `{"v":1,"k":"book","x":"<base64 XLSX>"}`
//!
//! A cell is `{"f":formula}` or one of `{"n":number}`, `{"s":text}`, `{"b":bool}`, `{"e":"#N/A"}`
//! (an empty value is `{}`), plus `"a":"A1:B2"` (legacy array range) and `"y":k` (index in the
//! payload's `y` styles, absent for the default style). A token names a sheet's row/column layout
//! (`t` of a cell, the `e` chunks of a sheet); `e` lists the structural edits leading to each.

use std::collections::HashMap;

use gridcraft_core::{CellError, CellRef, RangeRef, Value};
use gridcraft_formula::adjust::{Axis, Edit};
use gridcraft_model::{CellStore, DefinedName, Sheet, Style, StyleId, Workbook};
use serde_json::{Map, Value as Json, json};

use super::content::{CellData, Input, cell_data};

pub(crate) const VERSION: u64 = 1;
/// The shell sends at most 1 MiB; a longer payload didn't come through it and isn't read.
pub(crate) const MAX_LEN: usize = 2 << 20;
/// Structural edits one sheet payload may carry: each one stays in the structure history.
pub(crate) const MAX_EDITS: usize = 2_000;

#[derive(Default)]
struct Styles {
    list: Vec<Style>,
    index: HashMap<Style, usize>,
}

impl Styles {
    fn add(&mut self, s: &Style) -> usize {
        if let Some(i) = self.index.get(s) {
            return *i;
        }
        self.list.push(s.clone());
        self.index.insert(s.clone(), self.list.len() - 1);
        self.list.len() - 1
    }
    fn json(&self) -> Json {
        serde_json::to_value(&self.list).unwrap_or(Json::Array(vec![]))
    }
}

fn cell_json(d: &CellData, styles: &mut Styles) -> Json {
    let mut m = Map::new();
    match &d.input {
        Input::Formula(f) => {
            m.insert("f".into(), json!(f));
        }
        Input::Value(Value::Number(n)) => {
            m.insert("n".into(), json!(n));
        }
        Input::Value(Value::Text(t)) => {
            m.insert("s".into(), json!(t.as_ref()));
        }
        Input::Value(Value::Bool(b)) => {
            m.insert("b".into(), json!(b));
        }
        Input::Value(Value::Error(e)) => {
            m.insert("e".into(), json!(e.as_str()));
        }
        Input::Value(Value::Empty | Value::Array(_)) => {}
    }
    if let Some(a) = d.array {
        m.insert("a".into(), json!(a.a1()));
    }
    if d.style != Style::default() {
        m.insert("y".into(), json!(styles.add(&d.style)));
    }
    Json::Object(m)
}

fn cell_from_json(v: &Json, styles: &[Style]) -> Option<CellData> {
    let m = v.as_object()?;
    let input = if let Some(f) = m.get("f").and_then(Json::as_str) {
        Input::Formula(f.to_string())
    } else if let Some(n) = m.get("n").and_then(Json::as_f64) {
        Input::Value(Value::Number(n))
    } else if let Some(s) = m.get("s").and_then(Json::as_str) {
        Input::Value(Value::Text(s.into()))
    } else if let Some(b) = m.get("b").and_then(Json::as_bool) {
        Input::Value(Value::Bool(b))
    } else if let Some(e) = m.get("e").and_then(Json::as_str).and_then(CellError::parse) {
        Input::Value(Value::Error(e))
    } else {
        Input::Value(Value::Empty)
    };
    let array = m.get("a").and_then(Json::as_str).and_then(RangeRef::parse);
    let style = match m.get("y").and_then(Json::as_u64) {
        Some(k) => styles.get(usize::try_from(k).ok()?)?.clone(),
        None => Style::default(),
    };
    Some(CellData { input, array, style })
}

fn styles_from(v: &Json) -> Vec<Style> {
    v.get("y").and_then(|y| serde_json::from_value(y.clone()).ok()).unwrap_or_default()
}

fn header(v: &Json, kind: &str) -> Result<(), String> {
    if v.get("v").and_then(Json::as_u64) != Some(VERSION) {
        return Err("unknown payload version".into());
    }
    if v.get("k").and_then(Json::as_str) != Some(kind) {
        return Err(format!("not a {kind} payload"));
    }
    Ok(())
}

fn parse(payload: &str) -> Result<Json, String> {
    serde_json::from_str(payload).map_err(|_| "unreadable payload".to_string())
}

fn token_of(v: &Json, key: &str) -> Option<String> {
    v.get(key).and_then(Json::as_str).map(str::to_string)
}

// ---- cell

pub(crate) struct CellPayload {
    pub token: Option<String>,
    pub data: Option<CellData>,
}

pub(crate) fn encode_cell(token: Option<&str>, data: Option<&CellData>) -> String {
    let mut styles = Styles::default();
    let c = data.map_or(Json::Null, |d| cell_json(d, &mut styles));
    json!({"v": VERSION, "k": "cell", "t": token, "c": c, "y": styles.json()}).to_string()
}

pub(crate) fn decode_cell(payload: &str) -> Result<CellPayload, String> {
    let v = parse(payload)?;
    header(&v, "cell")?;
    let styles = styles_from(&v);
    let data = match v.get("c") {
        None | Some(Json::Null) => None,
        Some(c) => Some(cell_from_json(c, &styles).ok_or("unreadable cell")?),
    };
    Ok(CellPayload { token: token_of(&v, "t"), data })
}

// ---- structural edits

/// One recorded structural edit: (sheet uid, destination sheet uid, edit).
pub(crate) type StepEdit = (String, Option<String>, Edit);

fn axis(a: Axis) -> &'static str {
    match a {
        Axis::Rows => "r",
        Axis::Cols => "c",
    }
}

fn edit_json((sheet, to, e): &StepEdit) -> Json {
    let body = match e {
        Edit::Insert { axis: a, at, count } => json!({"i": [axis(*a), at, count]}),
        Edit::Delete { axis: a, at, count } => json!({"d": [axis(*a), at, count]}),
        Edit::Move { from, to_row, to_col } => json!({"m": [from.a1(), to_row, to_col]}),
        Edit::InsertCells { axis: a, range } => json!({"ic": [axis(*a), range.a1()]}),
        Edit::DeleteCells { axis: a, range } => json!({"dc": [axis(*a), range.a1()]}),
        Edit::MoveToSheet { from, to_sheet, to_row, to_col } => json!({"ms": [from.a1(), to_sheet, to_row, to_col]}),
    };
    let mut body = body;
    if let Some(m) = body.as_object_mut() {
        m.insert("s".into(), json!(sheet));
        if let Some(to) = to {
            m.insert("to".into(), json!(to));
        }
    }
    body
}

fn edit_from_json(v: &Json) -> Option<StepEdit> {
    let sheet = v.get("s")?.as_str()?.to_string();
    let to = v.get("to").and_then(Json::as_str).map(str::to_string);
    let axis_of = |x: &Json| match x.as_str()? {
        "r" => Some(Axis::Rows),
        "c" => Some(Axis::Cols),
        _ => None,
    };
    let num = |x: &Json| x.as_u64().and_then(|n| u32::try_from(n).ok());
    let range = |x: &Json| x.as_str().and_then(RangeRef::parse);
    let edit = if let Some([a, at, n]) = v.get("i").and_then(Json::as_array).map(Vec::as_slice) {
        Edit::Insert { axis: axis_of(a)?, at: num(at)?, count: num(n)? }
    } else if let Some([a, at, n]) = v.get("d").and_then(Json::as_array).map(Vec::as_slice) {
        Edit::Delete { axis: axis_of(a)?, at: num(at)?, count: num(n)? }
    } else if let Some([from, r, c]) = v.get("m").and_then(Json::as_array).map(Vec::as_slice) {
        Edit::Move { from: range(from)?, to_row: num(r)?, to_col: num(c)? }
    } else if let Some([a, r]) = v.get("ic").and_then(Json::as_array).map(Vec::as_slice) {
        Edit::InsertCells { axis: axis_of(a)?, range: range(r)? }
    } else if let Some([a, r]) = v.get("dc").and_then(Json::as_array).map(Vec::as_slice) {
        Edit::DeleteCells { axis: axis_of(a)?, range: range(r)? }
    } else if let Some([from, name, r, c]) = v.get("ms").and_then(Json::as_array).map(Vec::as_slice) {
        Edit::MoveToSheet { from: range(from)?, to_sheet: name.as_str()?.to_string(), to_row: num(r)?, to_col: num(c)? }
    } else {
        return None;
    };
    Some((sheet, to, edit))
}

// ---- sheet

pub(crate) struct SheetPayload {
    pub base: Option<String>,
    pub chunks: Vec<(String, Vec<StepEdit>)>,
    /// The sheet without its cells; row/column style ids index `styles`.
    pub sheet: Sheet,
    pub cells: Vec<(CellRef, CellData)>,
    pub others: Vec<(String, Vec<(CellRef, Option<CellData>)>)>,
    pub names: Option<Vec<DefinedName>>,
    pub styles: Vec<Style>,
}

impl SheetPayload {
    /// The token the layout is at once the payload is applied.
    pub fn token(&self) -> Option<&str> {
        self.chunks.last().map(|(t, _)| t.as_str()).or(self.base.as_deref())
    }
}

pub(crate) struct SheetChange<'a> {
    pub sheet: usize,
    pub base: Option<&'a str>,
    pub chunks: &'a [(String, Vec<StepEdit>)],
    pub others: &'a [(usize, Vec<CellRef>)],
    pub names: bool,
}

pub(crate) fn encode_sheet(wb: &Workbook, change: &SheetChange) -> Result<String, String> {
    let sh = wb.sheet(change.sheet).ok_or("no such sheet")?;
    let mut styles = Styles::default();
    let mut bare = sh.clone();
    bare.cells = CellStore::new();
    bare.spill.clear();
    bare.spill_ranges.clear();
    bare.preserved = None;
    for line in bare.cols.values_mut().chain(bare.rows.values_mut()) {
        if let Some(id) = line.style {
            line.style = Some(StyleId(u32::try_from(styles.add(wb.styles.get(id))).unwrap_or(0)));
        }
    }
    let mut cells = Map::new();
    for (at, cell) in sh.cells.iter() {
        if let Some(d) = cell_data(wb, Some(cell)) {
            cells.insert(at.a1(), cell_json(&d, &mut styles));
        }
    }
    let mut others = Map::new();
    for (i, list) in change.others {
        let Some(other) = wb.sheet(*i) else { continue };
        let mut patch = Map::new();
        for at in list {
            let d = cell_data(wb, other.cells.get(*at));
            patch.insert(at.a1(), d.map_or(Json::Null, |d| cell_json(&d, &mut styles)));
        }
        others.insert(other.uid.clone(), Json::Object(patch));
    }
    let chunks: Vec<Json> = change.chunks.iter().map(|(t, edits)| json!([t, edits.iter().map(edit_json).collect::<Vec<_>>()])).collect();
    let mut v = json!({
        "v": VERSION, "k": "sheet", "b": change.base, "e": chunks,
        "s": serde_json::to_value(&bare).map_err(|e| e.to_string())?,
        "c": cells, "x": others,
    });
    if let Some(m) = v.as_object_mut() {
        if change.names {
            m.insert("n".into(), serde_json::to_value(&wb.names).map_err(|e| e.to_string())?);
        }
        m.insert("y".into(), styles.json());
    }
    Ok(v.to_string())
}

pub(crate) fn decode_sheet(payload: &str) -> Result<SheetPayload, String> {
    let v = parse(payload)?;
    header(&v, "sheet")?;
    let styles = styles_from(&v);
    let s = v.get("s").cloned().unwrap_or(Json::Null);
    super::guard::check_json(&s)?;
    let sheet: Sheet = serde_json::from_value(s).map_err(|_| "unreadable sheet")?;
    super::guard::check_sheet(&sheet)?;
    let cell_map = |m: &Json| -> Vec<(CellRef, Option<CellData>)> {
        m.as_object()
            .map(|m| {
                m.iter().filter_map(|(a1, c)| Some((CellRef::parse(a1)?, if c.is_null() { None } else { cell_from_json(c, &styles) }))).collect()
            })
            .unwrap_or_default()
    };
    let cells = v.get("c").map(cell_map).unwrap_or_default().into_iter().filter_map(|(at, d)| Some((at, d?))).collect();
    let others =
        v.get("x").and_then(Json::as_object).map(|m| m.iter().map(|(uid, patch)| (uid.clone(), cell_map(patch))).collect()).unwrap_or_default();
    let names = v.get("n").and_then(|n| serde_json::from_value(n.clone()).ok());
    let chunks = v
        .get("e")
        .and_then(Json::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|c| {
                    let [t, edits] = c.as_array()?.as_slice() else { return None };
                    Some((t.as_str()?.to_string(), edits.as_array()?.iter().filter_map(edit_from_json).collect()))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(SheetPayload { base: token_of(&v, "b"), chunks, sheet, cells, others, names, styles })
}

// ---- book

pub(crate) fn encode_book(wb: &Workbook) -> Result<String, String> {
    let name = if wb.vba_project.is_some() { "live.xlsm" } else { "live.xlsx" };
    let bytes = crate::io::save_xlsx_fast(wb, name).map_err(|e| e.to_string())?;
    Ok(json!({"v": VERSION, "k": "book", "x": crate::io::base64_encode(&bytes)}).to_string())
}

pub(crate) fn decode_book(payload: &str) -> Result<Workbook, String> {
    let v = parse(payload)?;
    header(&v, "book")?;
    let bytes = v.get("x").and_then(Json::as_str).and_then(crate::io::base64_decode).ok_or("unreadable workbook")?;
    let mut wb = crate::io::open_live_xlsx(&bytes)?;
    wb.fill_missing_sheet_uids();
    Ok(wb)
}

/// The kind a payload says it is (`cell`, `sheet`, `book`).
pub(crate) fn kind(payload: &str) -> Option<String> {
    parse(payload).ok()?.get("k")?.as_str().map(str::to_string)
}
