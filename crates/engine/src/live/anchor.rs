//! Anchors of Cloud comments (ERPCons Office online M5, the `erpHost` comment calls): where a
//! thread sits in a workbook. The shell keeps them on its server; nothing here touches the file.
//!
//! An anchor names the top-left cell's region (`cell:<sheetUid>:<A1>`) at a layout token of the
//! room (`pos.t`, the token [`Live`] maps to a structure sequence number), the whole range when
//! more than one cell is selected (`pos.range`), and a soft fallback: the sheet's name, the cell's
//! A1 and its content (formula or value as typed; a hash when longer than [`CONTENT_MAX`]).
//! Placing it again prefers the region ([`Workbook::locate`]), then a cell holding the same content
//! nearest the old place, else the thread is orphaned.

use gridcraft_core::{CellRef, RangeRef};
use gridcraft_model::{CellAt, CellId, Sheet, Workbook};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::{Live, Region};
use crate::Session;

pub const VERSION: u32 = 1;
/// Longer content is kept as a hash, so an anchor stays small whatever the cell holds.
pub const CONTENT_MAX: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub v: u32,
    pub region: String,
    pub pos: Pos,
    pub soft: Soft,
    /// `Sheet1!B3` or `Sheet1!B3:C5`, for the comments panel.
    #[serde(default)]
    pub label: String,
    /// The same text: the shell shows `quote` as the thread's context.
    #[serde(default)]
    pub quote: String,
    /// Sheet index, row, column: the shell lists threads in this order.
    #[serde(default)]
    pub order: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pos {
    /// The layout token the region's coordinates are at; `null` when the page had none.
    #[serde(default)]
    pub t: Option<String>,
    /// The selected range when it is more than the region's cell (or its merge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Soft {
    pub sheet: String,
    pub a1: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// Where an anchor lies in the workbook now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placed {
    /// On sheet `sheet` (index), covering `range`; `anchor` is how it reads at that place now.
    At {
        sheet: usize,
        range: RangeRef,
        anchor: Box<Anchor>,
    },
    Orphaned,
}

impl Anchor {
    /// An anchor of this version, or `None` for any other shape.
    pub fn parse(json: &Json) -> Option<Anchor> {
        serde_json::from_value::<Anchor>(json.clone()).ok().filter(|a| a.v == VERSION)
    }

    pub fn to_json(&self) -> Json {
        serde_json::to_value(self).unwrap_or(Json::Null)
    }
}

/// FNV-1a, 64 bits: stable across builds and platforms, unlike the std hasher.
fn fnv(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn content_of(text: String) -> (Option<String>, Option<String>) {
    if text.chars().count() <= CONTENT_MAX { (Some(text), None) } else { (None, Some(fnv(&text))) }
}

/// What a soft fallback looks for.
enum Wanted {
    Text(String),
    Hash(String),
}

impl Wanted {
    fn of(soft: &Soft) -> Wanted {
        match (&soft.content, &soft.hash) {
            (Some(text), _) => Wanted::Text(text.clone()),
            (None, Some(hash)) => Wanted::Hash(hash.clone()),
            (None, None) => Wanted::Text(String::new()),
        }
    }
    fn is_empty(&self) -> bool {
        matches!(self, Wanted::Text(t) if t.is_empty())
    }
    fn matches(&self, text: &str) -> bool {
        match self {
            Wanted::Text(t) => t == text,
            Wanted::Hash(h) => text.chars().count() > CONTENT_MAX && fnv(text) == *h,
        }
    }
}

/// The anchor of `range` on sheet `si`, its coordinates at layout `token`.
fn build(wb: &Workbook, si: usize, range: RangeRef, token: Option<String>) -> Option<Anchor> {
    let sh = wb.sheet(si)?;
    let top = range.start;
    let single = range.is_single() || sh.merge_at(top) == Some(range);
    let shown = if single { top.a1() } else { range.a1() };
    let label = format!("{}!{shown}", sh.name);
    let (content, hash) = content_of(sh.input_text(top));
    Some(Anchor {
        v: VERSION,
        region: Region::Cell { sheet: sh.uid.clone(), cell: top }.id(),
        pos: Pos { t: token, range: (!single).then(|| range.a1()) },
        soft: Soft { sheet: sh.name.clone(), a1: top.a1(), content, hash },
        label: label.clone(),
        quote: label,
        order: vec![u32::try_from(si).unwrap_or(u32::MAX), top.row, top.col],
    })
}

impl Live {
    /// The layout token cell coordinates on sheet `uid` are at now.
    fn current_token(&self, uid: &str) -> Option<String> {
        match self.layouts.get(uid) {
            Some(layout) => layout.token.clone(),
            None => self.origin.clone(),
        }
    }

    /// The structure sequence number layout token `t` of sheet `uid` stands for here.
    fn seq_for(&self, uid: &str, t: &str) -> Option<u64> {
        match self.layouts.get(uid) {
            Some(layout) => layout.seq_of(t),
            None => (self.origin.as_deref() == Some(t)).then_some(0),
        }
    }
}

/// The nearest cell of `sh` holding what `wanted` names (ties: the upper, then the left one).
fn nearest(sh: &Sheet, from: CellRef, wanted: &Wanted) -> Option<CellRef> {
    let distance = |c: CellRef| u64::from(c.row.abs_diff(from.row)) + u64::from(c.col.abs_diff(from.col));
    sh.cells.iter().map(|(c, _)| c).filter(|c| wanted.matches(&sh.input_text(*c))).min_by_key(|c| (distance(*c), c.row, c.col))
}

/// `range` started at `start` before: the same size from `start` now, kept on the sheet.
fn sized(start: CellRef, range: Option<RangeRef>) -> RangeRef {
    match range {
        Some(r) => RangeRef::new(start, start.offset_clamped(i64::from(r.height()) - 1, i64::from(r.width()) - 1)),
        None => RangeRef::cell(start),
    }
}

fn place(wb: &Workbook, live: Option<&Live>, anchor: &Anchor) -> Placed {
    let (uid, cell) = match Region::parse(&anchor.region) {
        Some(Region::Cell { sheet, cell }) => (Some(sheet), Some(cell)),
        _ => (None, None),
    };
    let range = anchor.pos.range.as_deref().and_then(RangeRef::parse).filter(|r| cell.is_none_or(|c| r.start == c));
    let token_of = |si: usize| live.zip(wb.sheet(si)).and_then(|(l, sh)| l.current_token(&sh.uid));
    let at = |si: usize, range: RangeRef| match build(wb, si, range, token_of(si)) {
        Some(anchor) => Placed::At { sheet: si, range, anchor: Box::new(anchor) },
        None => Placed::Orphaned,
    };
    let mut deleted = false;
    if let (Some(uid), Some(cell), Some(t)) = (uid.as_deref(), cell, anchor.pos.t.as_deref())
        && let Some(seq) = live.and_then(|l| l.seq_for(uid, t))
    {
        match wb.locate(&CellId { sheet: uid.to_string(), cell, seq }) {
            CellAt::At(now) => {
                if let Some(si) = wb.sheet_index_by_uid(&now.sheet) {
                    let end = range.map(|r| wb.locate(&CellId { sheet: uid.to_string(), cell: r.end, seq }));
                    let moved = match end {
                        Some(CellAt::At(e)) if e.sheet == now.sheet && e.cell.row >= now.cell.row && e.cell.col >= now.cell.col => {
                            RangeRef::new(now.cell, e.cell)
                        }
                        _ => sized(now.cell, range),
                    };
                    return at(si, moved);
                }
                deleted = true;
            }
            CellAt::Deleted => deleted = true,
            CellAt::UnknownSeq => {}
        }
    }
    // Soft fallback: the region is gone or its layout unknown here (another version, a reload).
    let Some(si) = uid.as_deref().and_then(|u| wb.sheet_index_by_uid(u)).or_else(|| wb.sheet_index(&anchor.soft.sheet)) else {
        return Placed::Orphaned;
    };
    let Some(sh) = wb.sheet(si) else { return Placed::Orphaned };
    let Some(old) = CellRef::parse(&anchor.soft.a1).or(cell) else { return Placed::Orphaned };
    let wanted = Wanted::of(&anchor.soft);
    let found = if wanted.is_empty() {
        // An empty cell can't be searched for: it stays put unless its row or column was deleted.
        (!deleted).then_some(old)
    } else if wanted.matches(&sh.input_text(old)) {
        Some(old)
    } else {
        nearest(sh, old, &wanted)
    };
    match found {
        Some(start) => at(si, sized(start, range)),
        None => Placed::Orphaned,
    }
}

impl Session {
    fn anchor_live(&self, doc: u64) -> Option<&Live> {
        self.live.as_ref().filter(|l| l.doc == doc)
    }

    /// The anchor of the selection in document `doc` (uid) when it is the active one.
    pub fn comment_anchor(&self, doc: u64) -> Option<Anchor> {
        let d = self.active().filter(|d| d.uid == doc)?;
        let si = d.wb.active_sheet;
        let token = self.anchor_live(doc).zip(d.wb.sheet(si)).and_then(|(l, sh)| l.current_token(&sh.uid));
        build(&d.wb, si, d.selection.current(), token)
    }

    /// Where `anchor` lies in document `doc` now.
    pub fn place_comment_anchor(&self, doc: u64, anchor: &Anchor) -> Placed {
        match self.documents().iter().find(|d| d.uid == doc) {
            Some(d) => place(&d.wb, self.anchor_live(doc), anchor),
            None => Placed::Orphaned,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_stable_and_long_content_is_hashed() {
        assert_eq!(fnv(""), "cbf29ce484222325");
        assert_eq!(fnv("a"), "af63dc4c8601ec8c");
        let long = "x".repeat(CONTENT_MAX + 1);
        let (content, hash) = content_of(long.clone());
        assert!(content.is_none());
        let wanted = Wanted::Hash(hash.unwrap());
        assert!(wanted.matches(&long) && !wanted.matches("x"));
        assert_eq!(content_of("=A1".into()), (Some("=A1".into()), None));
    }

    #[test]
    fn anchors_round_trip_as_json_and_refuse_other_versions() {
        let a = Anchor {
            v: VERSION,
            region: "cell:f1:B3".into(),
            pos: Pos { t: Some("v4".into()), range: Some("B3:C5".into()) },
            soft: Soft { sheet: "Sheet1".into(), a1: "B3".into(), content: Some("=SUM(A1:A2)".into()), hash: None },
            label: "Sheet1!B3:C5".into(),
            quote: "Sheet1!B3:C5".into(),
            order: vec![0, 2, 1],
        };
        let json = a.to_json();
        assert_eq!(json["pos"]["range"], "B3:C5");
        assert!(json["soft"].get("hash").is_none());
        assert_eq!(Anchor::parse(&json), Some(a));
        let mut other = json.clone();
        other["v"] = 2.into();
        assert_eq!(Anchor::parse(&other), None);
        assert_eq!(Anchor::parse(&serde_json::json!({"kind": "document"})), None);
    }
}
