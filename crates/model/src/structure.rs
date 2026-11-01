//! Stable identities for live editing (ERPCons Office online, M3): a sheet keeps its
//! [`Sheet::uid`](crate::Sheet::uid) through renames, moves, save and open; a cell is its
//! coordinates at a structure sequence number, which every row/column insert or delete (and every
//! move of cells) bumps, so a coordinate taken earlier can be carried to the current layout.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gridcraft_core::CellRef;
use gridcraft_formula::adjust::{Edit, adjust_cell, inverse};
use serde::{Deserialize, Serialize};

/// A cell by identity: the sheet's uid and the cell's coordinates at structure sequence `seq`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellId {
    pub sheet: String,
    pub cell: CellRef,
    pub seq: u64,
}

/// Where a [`CellId`] is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellAt {
    /// The same cell at the current sequence number.
    At(CellId),
    /// A delete removed it (or its sheet), or a move pushed it off the sheet.
    Deleted,
    /// The sequence number is not one of this workbook's (another document, or a later session).
    UnknownSeq,
}

/// One structural edit, made on the workbook whose structure was at `parent`.
#[derive(Clone, Debug)]
struct Step {
    parent: u64,
    sheet: String,
    /// Destination sheet of a cut-paste to another sheet.
    to_sheet: Option<String>,
    edit: Edit,
}

/// The structure history every snapshot of one workbook shares (undo and redo steps, the
/// clipboard), and where this snapshot stands in it. A sequence number is never reused: an edit
/// made after an undo starts a new branch, and translating walks back to the common ancestor.
/// Starts at 0 when a workbook is created or read; it is not saved in the file.
#[derive(Clone, Default)]
pub struct Structure {
    seq: u64,
    steps: Arc<Mutex<Vec<Step>>>,
}

// The history isn't document content: two workbooks with the same cells are equal whatever their
// structural pasts, as they were before the history existed.
impl PartialEq for Structure {
    fn eq(&self, _: &Structure) -> bool {
        true
    }
}

impl std::fmt::Debug for Structure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Structure({})", self.seq)
    }
}

impl Structure {
    pub fn seq(&self) -> u64 {
        self.seq
    }

    fn steps(&self) -> MutexGuard<'_, Vec<Step>> {
        self.steps.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records `edit` on the sheet with uid `sheet` (and `to_sheet`, the destination of a move to
    /// another sheet) and moves this snapshot to the new sequence number.
    pub fn record(&mut self, sheet: &str, to_sheet: Option<&str>, edit: &Edit) {
        let mut steps = self.steps();
        steps.push(Step { parent: self.seq, sheet: sheet.to_string(), to_sheet: to_sheet.map(str::to_string), edit: edit.clone() });
        let seq = steps.len() as u64;
        drop(steps);
        self.seq = seq;
    }

    /// Carries `cell` of sheet `sheet` from sequence `from` to this snapshot's sequence: back
    /// through the edits this snapshot doesn't have (undone ones), then forward through the ones
    /// it has. `Ok(None)` when an edit on the way deleted it.
    fn translate(&self, sheet: &str, cell: CellRef, from: u64) -> Result<Option<(String, CellRef)>, ()> {
        let steps = self.steps();
        let step = |n: u64| n.checked_sub(1).and_then(|i| steps.get(usize::try_from(i).ok()?));
        if from > steps.len() as u64 || self.seq > steps.len() as u64 {
            return Err(());
        }
        // A step's parent is always an earlier sequence number, so both walks end at 0.
        let mut ours = BTreeSet::from([0]);
        let mut n = self.seq;
        while let Some(s) = step(n) {
            ours.insert(n);
            n = s.parent;
        }
        let mut at = (sheet.to_string(), cell);
        let mut n = from;
        while !ours.contains(&n) {
            let Some(s) = step(n) else { return Err(()) };
            let Some(back) = undo_step(s, at) else { return Ok(None) };
            at = back;
            n = s.parent;
        }
        let mut forward = Vec::new();
        let mut m = self.seq;
        while m != n {
            let Some(s) = step(m) else { return Err(()) };
            forward.push(s);
            m = s.parent;
        }
        for s in forward.into_iter().rev() {
            let Some(next) = apply_step(s, at) else { return Ok(None) };
            at = next;
        }
        Ok(Some(at))
    }
}

fn apply_step(s: &Step, (sheet, cell): (String, CellRef)) -> Option<(String, CellRef)> {
    if sheet != s.sheet {
        return Some((sheet, cell));
    }
    let moved = adjust_cell(cell, &s.edit)?;
    match (&s.edit, &s.to_sheet) {
        (Edit::MoveToSheet { from, .. }, Some(to)) if from.contains(cell) => Some((to.clone(), moved)),
        _ => Some((sheet, moved)),
    }
}

fn undo_step(s: &Step, (sheet, cell): (String, CellRef)) -> Option<(String, CellRef)> {
    let back = inverse(&s.edit);
    match (&s.to_sheet, &back) {
        (Some(to), Edit::Move { from: block, .. }) => {
            if sheet == *to && block.contains(cell) {
                Some((s.sheet.clone(), adjust_cell(cell, &back)?))
            } else {
                Some((sheet, cell))
            }
        }
        _ if sheet == s.sheet => Some((sheet, adjust_cell(cell, &back)?)),
        _ => Some((sheet, cell)),
    }
}

impl crate::Workbook {
    /// The structure sequence number cell coordinates are currently at.
    pub fn structure_seq(&self) -> u64 {
        self.structure.seq()
    }

    /// Records a structural edit on sheet `target` (by name), so [`CellId`]s taken before it
    /// follow their cells. Called wherever formulas are adjusted for such an edit.
    pub fn record_structure_edit(&mut self, target: &str, edit: &Edit) {
        let uid = |name: &str| self.sheet_index(name).and_then(|i| self.sheets.get(i)).map(|s| s.uid.clone()).unwrap_or_default();
        let sheet = uid(target);
        let to_sheet = match edit {
            Edit::MoveToSheet { to_sheet, .. } => Some(uid(to_sheet)),
            _ => None,
        };
        self.structure.record(&sheet, to_sheet.as_deref(), edit);
    }

    pub fn sheet_index_by_uid(&self, uid: &str) -> Option<usize> {
        self.sheets.iter().position(|s| s.uid == uid)
    }

    /// The identity of `cell` on sheet `sheet` (index) at the current sequence number.
    pub fn cell_id(&self, sheet: usize, cell: CellRef) -> Option<CellId> {
        let uid = self.sheets.get(sheet)?.uid.clone();
        Some(CellId { sheet: uid, cell, seq: self.structure_seq() })
    }

    /// Where the cell `id` names is now.
    pub fn locate(&self, id: &CellId) -> CellAt {
        match self.structure.translate(&id.sheet, id.cell, id.seq) {
            Err(()) => CellAt::UnknownSeq,
            Ok(Some((sheet, cell))) if self.sheet_index_by_uid(&sheet).is_some() => CellAt::At(CellId { sheet, cell, seq: self.structure_seq() }),
            Ok(_) => CellAt::Deleted,
        }
    }

    /// Gives every sheet without a uid, or whose uid an earlier sheet already has, one derived from
    /// its position (`f2`, then `f2.2`… when taken). Deterministic, so every client that opens the
    /// same bytes agrees; uids made while editing carry the client prefix and contain a `-`.
    pub fn fill_missing_sheet_uids(&mut self) {
        let mut used = std::collections::HashSet::new();
        let mut missing = Vec::new();
        for (i, s) in self.sheets.iter().enumerate() {
            if s.uid.is_empty() || !used.insert(s.uid.clone()) {
                missing.push(i);
            }
        }
        for i in missing {
            let base = format!("f{}", i + 1);
            let uid = (1u32..).map(|k| if k == 1 { base.clone() } else { format!("{base}.{k}") }).find(|u| !used.contains(u)).unwrap_or_default();
            used.insert(uid.clone());
            if let Some(s) = self.sheet_mut(i) {
                s.uid = uid;
            }
        }
    }
}

/// Whether `uid` may be a sheet uid read from a file: 1 to 32 characters of `0-9 a-z . -`.
pub fn is_valid_sheet_uid(uid: &str) -> bool {
    (1..=32).contains(&uid.len()) && uid.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b == b'.' || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Sheet, Workbook};
    use gridcraft_core::RangeRef;
    use gridcraft_formula::adjust::Axis;

    fn book() -> Workbook {
        let mut wb = Workbook::new();
        wb.sheets.push(Arc::new(Sheet::new("Two")));
        wb.fill_missing_sheet_uids();
        wb
    }

    fn cell(a1: &str) -> CellRef {
        CellRef::parse(a1).unwrap()
    }

    fn at(wb: &Workbook, id: &CellId) -> Option<CellRef> {
        match wb.locate(id) {
            CellAt::At(now) => {
                assert_eq!(now.seq, wb.structure_seq());
                Some(now.cell)
            }
            CellAt::Deleted => None,
            CellAt::UnknownSeq => panic!("unknown seq {}", id.seq),
        }
    }

    #[test]
    fn missing_uids_are_derived_from_the_position_and_kept_otherwise() {
        let mut wb = Workbook::new();
        for name in ["B", "C", "D"] {
            wb.sheets.push(Arc::new(Sheet::new(name)));
        }
        wb.sheet_mut(1).unwrap().uid = "a1b2c3-4".into();
        wb.sheet_mut(2).unwrap().uid = "a1b2c3-4".into();
        wb.sheet_mut(3).unwrap().uid = "f3".into();
        wb.fill_missing_sheet_uids();
        let uids: Vec<_> = wb.sheets.iter().map(|s| s.uid.as_str()).collect();
        assert_eq!(uids, ["f1", "a1b2c3-4", "f3.2", "f3"]);
        let again = uids.iter().map(|u| u.to_string()).collect::<Vec<_>>();
        wb.fill_missing_sheet_uids();
        assert_eq!(wb.sheets.iter().map(|s| s.uid.clone()).collect::<Vec<_>>(), again);
        assert!(is_valid_sheet_uid("a1b2c3-4") && is_valid_sheet_uid("f3.2"));
        assert!(!is_valid_sheet_uid("") && !is_valid_sheet_uid("A1") && !is_valid_sheet_uid(&"a".repeat(33)) && !is_valid_sheet_uid("a b"));
    }

    #[test]
    fn coordinates_follow_inserts_and_deletes() {
        let mut wb = book();
        let b5 = wb.cell_id(0, cell("B5")).unwrap();
        let other = wb.cell_id(1, cell("B5")).unwrap();
        assert_eq!(b5.seq, 0);
        wb.record_structure_edit("Sheet1", &Edit::Insert { axis: Axis::Rows, at: 1, count: 3 });
        assert_eq!(wb.structure_seq(), 1);
        assert_eq!(at(&wb, &b5), Some(cell("B8")));
        assert_eq!(at(&wb, &other), Some(cell("B5")), "another sheet's cells stay");
        wb.record_structure_edit("Sheet1", &Edit::Delete { axis: Axis::Cols, at: 0, count: 1 });
        assert_eq!(at(&wb, &b5), Some(cell("A8")));
        let a8 = wb.cell_id(0, cell("A8")).unwrap();
        assert_eq!(at(&wb, &a8), Some(cell("A8")));
        wb.record_structure_edit("Sheet1", &Edit::Delete { axis: Axis::Rows, at: 7, count: 1 });
        assert_eq!(at(&wb, &b5), None);
        assert_eq!(at(&wb, &a8), None);
        assert_eq!(wb.locate(&CellId { seq: 99, ..b5 }), CellAt::UnknownSeq);
    }

    #[test]
    fn undo_and_redo_carry_ids_both_ways() {
        let mut wb = book();
        let before = wb.clone();
        let c3 = wb.cell_id(0, cell("C3")).unwrap();
        wb.record_structure_edit("Sheet1", &Edit::Insert { axis: Axis::Cols, at: 0, count: 2 });
        let after = wb.clone();
        let e3 = after.cell_id(0, cell("E3")).unwrap();
        let a3 = after.cell_id(0, cell("A3")).unwrap();
        // Undo restores the earlier snapshot: ids taken after the insert come back.
        assert_eq!(at(&before, &e3), Some(cell("C3")));
        assert_eq!(at(&before, &a3), None, "an inserted column did not exist");
        assert_eq!(at(&after, &c3), Some(cell("E3")));
        // A new edit after the undo gets a new number and both branches still translate.
        let mut branch = before.clone();
        branch.record_structure_edit("Sheet1", &Edit::Delete { axis: Axis::Rows, at: 0, count: 1 });
        assert_eq!(branch.structure_seq(), 2);
        assert_eq!(at(&branch, &e3), Some(cell("C2")));
        let c2 = branch.cell_id(0, cell("C2")).unwrap();
        assert_eq!(at(&after, &c2), Some(cell("E3")));
        assert_eq!(at(&after, &c3), Some(cell("E3")));
    }

    #[test]
    fn a_cut_to_another_sheet_takes_its_cells_along() {
        let mut wb = book();
        let b2 = wb.cell_id(0, cell("B2")).unwrap();
        let outside = wb.cell_id(0, cell("D4")).unwrap();
        let mv = Edit::MoveToSheet { from: RangeRef::parse("A1:B2").unwrap(), to_sheet: "Two".into(), to_row: 9, to_col: 2 };
        let before = wb.clone();
        wb.record_structure_edit("Sheet1", &mv);
        let now = wb.locate(&b2);
        assert_eq!(now, CellAt::At(CellId { sheet: "f2".into(), cell: cell("D11"), seq: 1 }));
        assert_eq!(at(&wb, &outside), Some(cell("D4")));
        let CellAt::At(moved) = now else { panic!("moved cell lost") };
        assert_eq!(before.locate(&moved), CellAt::At(CellId { sheet: "f1".into(), cell: cell("B2"), seq: 0 }));
    }

    #[test]
    fn a_deleted_sheet_deletes_its_cells_and_renames_change_nothing() {
        let mut wb = book();
        let b2 = wb.cell_id(1, cell("B2")).unwrap();
        wb.sheet_mut(1).unwrap().name = "Renamed".into();
        wb.record_structure_edit("Renamed", &Edit::Insert { axis: Axis::Rows, at: 0, count: 1 });
        assert_eq!(at(&wb, &b2), Some(cell("B3")));
        wb.sheets.remove(1);
        assert_eq!(wb.locate(&b2), CellAt::Deleted);
    }

    #[test]
    fn equality_ignores_the_history() {
        let wb = book();
        let mut moved = wb.clone();
        moved.record_structure_edit("Sheet1", &Edit::Insert { axis: Axis::Rows, at: 0, count: 1 });
        assert_eq!(wb, moved);
        assert_ne!(wb.structure_seq(), moved.structure_seq());
    }
}
