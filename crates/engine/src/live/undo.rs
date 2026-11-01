//! Per-user undo while co-editing: each step remembers its region and the workbook around it.
//! Undo takes back the newest step whose region still holds what this user left there; a step
//! whose region someone else changed since is dropped, so undo never reverts their work.

use std::sync::Arc;

use gridcraft_formula::adjust::inverse;
use gridcraft_model::Workbook;
use serde_json::{Value as Json, json};

use super::content::{self, data_at, put_cell};
use super::{Action, Live, NOTHING_TO_UNDO, Region, Verdict};
use crate::Session;

const STEPS_KEPT: usize = 100;

#[derive(Clone, Debug)]
pub(super) struct Step {
    region: Region,
    before: Arc<Workbook>,
    after: Arc<Workbook>,
}

impl Step {
    pub(super) fn new(region: Region, before: Arc<Workbook>, after: Arc<Workbook>) -> Step {
        Step { region, before, after }
    }

    /// The region in `now` still holds what this step left there.
    fn current(&self, now: &Workbook) -> bool {
        match &self.region {
            Region::Cell { sheet, cell } => {
                let (Some(i), Some(j)) = (now.sheet_index_by_uid(sheet), self.after.sheet_index_by_uid(sheet)) else { return false };
                data_at(now, i, *cell) == data_at(&self.after, j, *cell)
            }
            Region::Sheet(uid) => {
                let (Some(a), Some(b)) = (sheet_by_uid(now, uid), sheet_by_uid(&self.after, uid)) else { return false };
                content::sheet_content_eq(now, a, &self.after, b) && self.side_cells_eq(now, &self.after)
            }
            Region::Book => content::book_content_eq(now, &self.after),
        }
    }

    /// Cells of other sheets this step changed hold the same in `a` as in `b`.
    fn side_cells_eq(&self, a: &Workbook, b: &Workbook) -> bool {
        self.side_cells().iter().all(|(uid, cell)| {
            let (Some(i), Some(j)) = (a.sheet_index_by_uid(uid), b.sheet_index_by_uid(uid)) else { return false };
            data_at(a, i, *cell) == data_at(b, j, *cell)
        })
    }

    fn side_cells(&self) -> Vec<(String, gridcraft_core::CellRef)> {
        let Region::Sheet(uid) = &self.region else { return Vec::new() };
        let mut out = Vec::new();
        for (x, y) in self.before.sheets.iter().zip(&self.after.sheets) {
            if x.uid == *uid || Arc::ptr_eq(x, y) {
                continue;
            }
            out.extend(content::changed_cells(x, y).into_iter().map(|c| (x.uid.clone(), c)));
        }
        out
    }

    /// `now` with this step's region as it was before the step.
    fn reverted(&self, now: &Arc<Workbook>) -> Arc<Workbook> {
        match &self.region {
            Region::Book => {
                let mut wb = (*self.before).clone();
                wb.active_sheet = now.active().and_then(|s| wb.sheet_index_by_uid(&s.uid)).unwrap_or(0).min(wb.sheets.len().saturating_sub(1));
                Arc::new(wb)
            }
            Region::Cell { sheet, cell } => {
                let mut wb = (**now).clone();
                if let (Some(i), Some(j)) = (wb.sheet_index_by_uid(sheet), self.before.sheet_index_by_uid(sheet)) {
                    put_cell(&mut wb, i, *cell, data_at(&self.before, j, *cell).as_ref());
                }
                Arc::new(wb)
            }
            Region::Sheet(uid) => {
                let mut wb = (**now).clone();
                let side = self.side_cells();
                if let (Some(i), Some(old)) = (wb.sheet_index_by_uid(uid), sheet_by_uid(&self.before, uid)) {
                    let mut sh = old.clone();
                    sh.cells = gridcraft_model::CellStore::new();
                    if let Some(cur) = wb.sheet(i) {
                        sh.view_active = cur.view_active;
                        sh.view_top_left = cur.view_top_left;
                        sh.zoom = cur.zoom;
                        sh.view = cur.view;
                    }
                    for line in sh.cols.values_mut().chain(sh.rows.values_mut()) {
                        if let Some(id) = line.style {
                            line.style = Some(wb.styles.intern(self.before.styles.get(id).clone()));
                        }
                    }
                    if let Some(slot) = wb.sheets.get_mut(i) {
                        *slot = Arc::new(sh);
                    }
                    for (at, cell) in old.cells.iter() {
                        put_cell(&mut wb, i, at, content::cell_data(&self.before, Some(cell)).as_ref());
                    }
                }
                for (other, cell) in side {
                    if let (Some(i), Some(j)) = (wb.sheet_index_by_uid(&other), self.before.sheet_index_by_uid(&other)) {
                        put_cell(&mut wb, i, cell, data_at(&self.before, j, cell).as_ref());
                    }
                }
                if self.before.names != self.after.names {
                    wb.names = self.before.names.clone();
                }
                // Undoing rows/columns moves cells back: ids taken meanwhile follow them.
                if let Some(steps) = self.after.structure.steps_since(self.before.structure_seq()) {
                    for (s, to, e) in steps.iter().rev() {
                        let back = inverse(e);
                        let (s, to) = match (&back, to) {
                            (gridcraft_formula::adjust::Edit::Move { .. }, Some(to)) => (to.clone(), Some(s.clone())),
                            _ => (s.clone(), to.clone()),
                        };
                        wb.structure.record(&s, to.as_deref(), &back);
                    }
                }
                Arc::new(wb)
            }
        }
    }
}

fn sheet_by_uid<'a>(wb: &'a Workbook, uid: &str) -> Option<&'a gridcraft_model::Sheet> {
    wb.sheet_index_by_uid(uid).and_then(|i| wb.sheet(i))
}

impl Live {
    pub(super) fn record(&mut self, step: Step) {
        self.undo.push(step);
        if self.undo.len() > STEPS_KEPT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Steps this user can still undo / redo (their regions may have changed since).
    pub fn undo_depth(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
}

impl Session {
    /// `edit.undo` / `edit.redo` while co-editing.
    pub(crate) fn live_undo(&mut self, redo: bool) -> crate::Result<Json> {
        let Some(mut live) = self.live.take() else { return Ok(json!({"undone": 0})) };
        let result = self.live_undo_with(&mut live, redo);
        self.live = Some(live);
        result
    }

    fn live_undo_with(&mut self, live: &mut Live, redo: bool) -> crate::Result<Json> {
        let Some(d) = self.docs.iter_mut().find(|d| d.uid == live.doc) else { return Err(crate::EngineError::NoDocument) };
        loop {
            let stack = if redo { &mut live.redo } else { &mut live.undo };
            let Some(step) = stack.pop() else {
                live.feedback = Some(super::Feedback::plain(NOTHING_TO_UNDO));
                return Ok(json!({"undone": 0}));
            };
            if !step.current(&d.wb) {
                continue;
            }
            match live.verdict(&step.region) {
                Verdict::Held => {}
                Verdict::Blocked(name) => {
                    live.feedback = Some(super::Feedback::locked_by(name));
                    let stack = if redo { &mut live.redo } else { &mut live.undo };
                    stack.push(step);
                    return Ok(json!({"undone": 0}));
                }
                Verdict::Ask => {
                    let region = step.region.clone();
                    let stack = if redo { &mut live.redo } else { &mut live.undo };
                    stack.push(step);
                    live.ask(region, Some(if redo { Action::Redo } else { Action::Undo }));
                    return Ok(json!({"undone": 0, "waiting": true}));
                }
            }
            let before = d.wb.clone();
            let mut after = step.reverted(&before);
            {
                let wb = Arc::make_mut(&mut after);
                d.calc.structural_changed(wb);
            }
            d.wb = after.clone();
            d.revision += 1;
            let scope = content::scope(&before, &after, before.active_sheet);
            if let Some(region) = super::region_of(&scope, &after) {
                live.emit(&before, &after, &scope, &region);
            }
            let back = Step::new(step.region.clone(), before, after);
            let other = if redo { &mut live.undo } else { &mut live.redo };
            other.push(back);
            return Ok(json!({"undone": 1}));
        }
    }
}
