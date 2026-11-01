//! Cloud comment marks on the grid (web, `erpHost` comment calls): a corner marker and a light
//! tint on the cells of each open thread the shell shows, placed again whenever the workbook
//! changes. View only: nothing here touches the document, its undo history or what is saved.
//! Without threads (desktop, older shells) nothing is drawn.

use std::sync::{Arc, Weak};

use egui::{Color32, Painter, Stroke, StrokeKind, pos2};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::live::anchor::{Anchor, Placed};
use gridcraft_engine::model::{Sheet, Workbook};

use crate::SheetApp;
use crate::grid::Geo;

const MARK: Color32 = Color32::from_rgb(0xE3, 0x9B, 0x00);

/// An open thread the shell asked to show.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkThread {
    pub id: String,
    pub anchor: Anchor,
}

#[derive(Clone, Debug, PartialEq)]
struct Mark {
    id: String,
    sheet: String,
    range: RangeRef,
}

#[derive(Default)]
pub struct CommentMarks {
    /// The document (uid) the threads belong to.
    doc: Option<u64>,
    threads: Vec<MarkThread>,
    placed: Vec<Mark>,
    /// What `placed` was computed from: (document, workbook snapshot, threads generation). The
    /// weak pointer keeps the snapshot's address from being reused by a later one.
    key: Option<(u64, Weak<Workbook>, u64)>,
    generation: u64,
    clicked: Vec<String>,
    /// The thread the shell last focused: drawn stronger.
    pub focused: Option<String>,
}

impl CommentMarks {
    /// `showComments`: the open threads of document `doc` replace those shown.
    pub fn show(&mut self, doc: u64, threads: Vec<MarkThread>) {
        self.doc = Some(doc);
        self.threads = threads;
        self.generation += 1;
        self.key = None;
        if self.focused.as_ref().is_some_and(|f| !self.threads.iter().any(|t| &t.id == f)) {
            self.focused = None;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }

    /// Places the threads again when the workbook or the threads changed since the last time.
    pub fn refresh(&mut self, session: &Session) {
        let Some(doc) = self.doc else { return };
        if self.threads.is_empty() {
            self.placed.clear();
            return;
        }
        let Some(d) = session.documents().iter().find(|d| d.uid == doc) else {
            self.placed.clear();
            return;
        };
        if self.key.as_ref().is_some_and(|(k, wb, g)| *k == doc && *g == self.generation && Weak::as_ptr(wb) == Arc::as_ptr(&d.wb)) {
            return;
        }
        self.key = Some((doc, Arc::downgrade(&d.wb), self.generation));
        self.placed = self
            .threads
            .iter()
            .filter_map(|t| match session.place_comment_anchor(doc, &t.anchor) {
                Placed::At { sheet, range, .. } => Some(Mark { id: t.id.clone(), sheet: d.wb.sheet(sheet)?.uid.clone(), range }),
                Placed::Orphaned => None,
            })
            .collect();
    }

    /// The threads whose mark the user clicked since the last call.
    pub fn take_clicked(&mut self) -> Vec<String> {
        std::mem::take(&mut self.clicked)
    }

    /// A click on `cell` of sheet `sheet` (uid) in document `doc`: the smallest mark holding it.
    pub fn click(&mut self, doc: u64, sheet: &str, cell: CellRef) {
        if self.doc != Some(doc) {
            return;
        }
        let hit = self.placed.iter().filter(|m| m.sheet == sheet && m.range.contains(cell)).min_by_key(|m| m.range.count());
        if let Some(mark) = hit {
            self.clicked.push(mark.id.clone());
        }
    }

    /// The marks on sheet `sheet` (uid) of document `doc`, as (thread, range).
    pub fn marks_on(&self, doc: u64, sheet: &str) -> Vec<(&str, RangeRef)> {
        if self.doc != Some(doc) {
            return Vec::new();
        }
        self.placed.iter().filter(|m| m.sheet == sheet).map(|m| (m.id.as_str(), m.range)).collect()
    }
}

/// Paints the marks of sheet `sh` (clipped to the cells by the caller).
pub(crate) fn paint(app: &SheetApp, p: &Painter, geo: &Geo, sh: &Sheet) {
    let Some(doc) = app.session.active().map(|d| d.uid) else { return };
    let marks = &app.comment_marks;
    for (id, range) in marks.marks_on(doc, &sh.uid) {
        let top = range.start;
        let cell = if range.is_single() { sh.merge_at(top).unwrap_or(range) } else { range };
        let r = geo.range_rect(sh, cell);
        if !r.intersects(geo.cells) || geo.hidden_under_frozen(sh, top) {
            continue;
        }
        let focused = marks.focused.as_deref() == Some(id);
        p.rect_filled(r, 0.0, MARK.gamma_multiply(if focused { 0.22 } else { 0.10 }));
        if focused || !range.is_single() {
            p.rect_stroke(r, 0.0, Stroke::new(if focused { 2.0 } else { 1.0 }, MARK), StrokeKind::Inside);
        }
        let corner = geo.cell_rect(sh, top);
        let s = 8.0 * geo.z.max(0.7);
        p.add(egui::Shape::convex_polygon(
            vec![corner.right_top(), pos2(corner.right(), corner.top() + s), pos2(corner.right() - s, corner.top())],
            MARK,
            Stroke::NONE,
        ));
        // A note in the file keeps its own (smaller) marker visible on top.
        if let Some(cm) = sh.comments.get(&top) {
            let tri = if cm.threaded { Color32::from_rgb(0x88, 0x55, 0xC8) } else { Color32::from_rgb(0xD0, 0x2E, 0x2E) };
            let s = 5.0 * geo.z.max(0.7);
            p.add(egui::Shape::convex_polygon(
                vec![corner.right_top(), pos2(corner.right(), corner.top() + s), pos2(corner.right() - s, corner.top())],
                tri,
                Stroke::NONE,
            ));
        }
    }
}
