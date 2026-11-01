//! Applying another member's payload: exactly the region it names changes, then the workbook is
//! recalculated here. The selection and view stay where the user left them.

use std::sync::Arc;

use gridcraft_model::{CellAt, CellId, CellStore, Workbook};

use super::content::put_cell;
use super::payload::{self, SheetPayload};
use super::{Live, Region};
use crate::Session;

pub(super) fn apply(session: &mut Session, live: &mut Live, region_id: &str, text: &str) -> Result<(), String> {
    let region = Region::parse(region_id).ok_or("unknown region")?;
    let Some(d) = session.docs.iter_mut().find(|d| d.uid == live.doc) else { return Err("the document is not open".into()) };
    let active_uid = d.wb.active().map(|s| s.uid.clone());
    match (&region, payload::kind(text).as_deref()) {
        (Region::Cell { sheet, cell }, Some("cell")) => {
            let p = payload::decode_cell(text)?;
            let Some(mut index) = d.wb.sheet_index_by_uid(sheet) else { return Ok(()) };
            let mut at = *cell;
            let layout = live.layout(sheet);
            if let Some(t) = &p.token
                && layout.token.as_ref() != Some(t)
                && let Some(seq) = layout.seq_of(t)
            {
                match d.wb.locate(&CellId { sheet: sheet.clone(), cell: *cell, seq }) {
                    CellAt::At(now) => {
                        at = now.cell;
                        index = d.wb.sheet_index_by_uid(&now.sheet).unwrap_or(index);
                    }
                    CellAt::Deleted => return Ok(()),
                    CellAt::UnknownSeq => {}
                }
            }
            let wb = Arc::make_mut(&mut d.wb);
            put_cell(wb, index, at, p.data.as_ref());
            d.calc.cells_changed(wb, &[(index, at)]);
        }
        (Region::Sheet(uid), Some("sheet")) => {
            let p = payload::decode_sheet(text)?;
            let Some(index) = d.wb.sheet_index_by_uid(uid) else { return Ok(()) };
            let wb = Arc::make_mut(&mut d.wb);
            let base = replace_sheet(wb, index, uid, p, live);
            reapply_offline(wb, uid, live, base);
            d.calc.structural_changed(wb);
        }
        (Region::Book, Some("book")) => {
            let mut wb = payload::decode_book(text)?;
            wb.active_sheet = active_uid.as_deref().and_then(|u| wb.sheet_index_by_uid(u)).unwrap_or(0).min(wb.sheets.len().saturating_sub(1));
            d.calc.structural_changed(&mut wb);
            d.wb = Arc::new(wb);
            live.layouts.clear();
            live.origin = None;
            live.sheet_edits.clear();
        }
        _ => return Err("the payload doesn't fit its region".into()),
    }
    d.revision += 1;
    Ok(())
}

/// Replaces sheet `index` with the payload's content, keeping this client's view of it, and
/// moves the layout to the payload's token. Returns the sequence number the payload's edits
/// were recorded from, when they were.
fn replace_sheet(wb: &mut Workbook, index: usize, uid: &str, p: SheetPayload, live: &mut Live) -> Option<u64> {
    let token = p.token().map(str::to_string);
    let SheetPayload { base, chunks, sheet: mut sh, cells, others, names, styles } = p;
    let old = wb.sheet(index).cloned();
    if let Some(old) = &old {
        sh.view_active = old.view_active;
        sh.view_top_left = old.view_top_left;
        sh.zoom = old.zoom;
        sh.view = old.view;
        sh.preserved = old.preserved.clone();
    }
    sh.uid = uid.to_string();
    sh.cells = CellStore::new();
    // Spills are recalculated here, never taken from a peer.
    sh.spill.clear();
    sh.spill_ranges.clear();
    for line in sh.cols.values_mut().chain(sh.rows.values_mut()) {
        if let Some(id) = line.style {
            line.style = styles.get(id.0 as usize).map(|s| wb.styles.intern(s.clone()));
        }
    }
    if let Some(slot) = wb.sheets.get_mut(index) {
        *slot = Arc::new(sh);
    }
    for (at, data) in &cells {
        put_cell(wb, index, *at, Some(data));
    }
    for (other, patch) in &others {
        let Some(i) = wb.sheet_index_by_uid(other) else { continue };
        for (at, data) in patch {
            put_cell(wb, i, *at, data.as_ref());
        }
    }
    if let Some(names) = names {
        wb.names = names;
    }
    // Record the structural edits this client hasn't seen, so ids taken before them still find their cells.
    let layout = live.layout(uid);
    let from = wb.structure_seq();
    let start = if layout.token == base { Some(0) } else { chunks.iter().position(|(t, _)| Some(t) == layout.token.as_ref()).map(|k| k + 1) };
    if let Some(k) = start {
        for (t, edits) in chunks.iter().skip(k) {
            for (s, to, e) in edits {
                wb.structure.record(s, to.as_deref(), e);
            }
            layout.set(t.clone(), wb.structure_seq());
        }
    }
    if let Some(t) = token
        && layout.token.as_ref() != Some(&t)
    {
        layout.set(t, wb.structure_seq());
    }
    start.map(|_| from)
}

/// Puts back this page's offline cell edits on sheet `uid` that a replayed sheet payload
/// replaced, at the cells its edits moved them to.
fn reapply_offline(wb: &mut Workbook, uid: &str, live: &mut Live, from: Option<u64>) {
    let offline: Vec<_> = live.offline.iter().filter(|o| o.sheet == uid).cloned().collect();
    for o in offline {
        let seq = match o.token.as_deref().and_then(|t| live.layout(uid).seq_of(t)) {
            Some(seq) => seq,
            None => match from {
                Some(seq) => seq,
                None => continue,
            },
        };
        let CellAt::At(now) = wb.locate(&CellId { sheet: uid.to_string(), cell: o.cell, seq }) else { continue };
        let Some(index) = wb.sheet_index_by_uid(&now.sheet) else { continue };
        put_cell(wb, index, now.cell, o.data.as_ref());
    }
}
