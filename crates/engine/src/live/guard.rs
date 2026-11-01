//! Bounds on another member's sheet payload, checked before it touches the workbook. Its sheet
//! comes through serde, which skips the constructors (`RangeRef::new` normalises, the readers clamp
//! sizes), while the interface loops over the ranges and sizes it holds.

use gridcraft_core::{MAX_COLS, MAX_ROWS};
use gridcraft_model::{LineInfo, Sheet};
use serde_json::Value as Json;

use super::payload::StepEdit;

/// Largest row height or column width a payload may set, in pixels (the interface stops at 1800).
const MAX_LINE_SIZE: f32 = 4096.0;
/// Smallest non-zero default row height or column width: below it one screen holds millions of cells.
const MIN_DEFAULT_SIZE: f32 = 2.0;
const MAX_OUTLINE: u8 = 7;

const REFUSED: &str = "the sheet in the payload is out of bounds";

/// Every cell reference (`{row, col}`) in the sheet's JSON lies on a sheet, and every range
/// (`{start, end}`) runs from its top-left to its bottom-right cell.
pub(super) fn check_json(v: &Json) -> Result<(), &'static str> {
    let mut todo = vec![v];
    while let Some(v) = todo.pop() {
        match v {
            Json::Object(m) => {
                if let (Some(start), Some(end), 2) = (m.get("start").and_then(cell_of), m.get("end").and_then(cell_of), m.len()) {
                    if start.0 > end.0 || start.1 > end.1 {
                        return Err(REFUSED);
                    }
                }
                if let Some((row, col)) = cell_of(v)
                    && (row >= u64::from(MAX_ROWS) || col >= u64::from(MAX_COLS))
                {
                    return Err(REFUSED);
                }
                todo.extend(m.values());
            }
            Json::Array(list) => todo.extend(list),
            _ => {}
        }
    }
    Ok(())
}

fn cell_of(v: &Json) -> Option<(u64, u64)> {
    let m = v.as_object().filter(|m| m.len() == 2)?;
    Some((m.get("row")?.as_u64()?, m.get("col")?.as_u64()?))
}

/// Row/column lines, sizes, outline levels, freeze and filter offsets the interface can draw.
pub(super) fn check_sheet(sh: &Sheet) -> Result<(), &'static str> {
    let lines_ok = |lines: &std::collections::BTreeMap<u32, LineInfo>, max: u32| {
        lines.iter().all(|(&at, l)| at < max && l.outline <= MAX_OUTLINE && l.size.is_none_or(|s| (0.0..=MAX_LINE_SIZE).contains(&s)))
    };
    let default_ok = |s: f32| s == 0.0 || (MIN_DEFAULT_SIZE..=MAX_LINE_SIZE).contains(&s);
    let freeze_ok = sh.freeze.is_none_or(|(r, c)| r < MAX_ROWS && c < MAX_COLS);
    let filter_ok = sh.autofilter.as_ref().is_none_or(|af| af.criteria.iter().all(|(o, _)| *o < af.range.width()));
    let ok = lines_ok(&sh.rows, MAX_ROWS)
        && lines_ok(&sh.cols, MAX_COLS)
        && default_ok(sh.default_row_height)
        && default_ok(sh.default_col_width)
        && freeze_ok
        && filter_ok;
    if ok { Ok(()) } else { Err(REFUSED) }
}

/// The structural edits move rows and columns of the region's sheet only: a move to another
/// sheet takes `*` (see `content::scope`).
pub(super) fn check_edits(uid: &str, chunks: &[(String, Vec<StepEdit>)]) -> Result<(), &'static str> {
    let own = |s: &str| s == uid;
    if chunks.iter().flat_map(|(_, edits)| edits).all(|(s, to, _)| own(s) && to.as_deref().is_none_or(own)) {
        Ok(())
    } else {
        Err("the payload moves rows or columns outside its sheet")
    }
}
