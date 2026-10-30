//! What the user was looking at in a document (sheet, selection, first visible cell), carried
//! over when its page host replaces it with a newer version.

use gridcraft_engine::Selection;
use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::{SheetViewMode, Visibility};

use crate::{SheetApp, SheetView};

#[derive(Clone, Debug, PartialEq)]
pub struct ViewSnapshot {
    /// By name: the newer version may have added, removed or reordered sheets.
    pub sheet: String,
    pub selection: Selection,
    pub top_left: CellRef,
}

impl SheetApp {
    /// The view of open document `uid`, as [`SheetApp::restore_view`] takes it.
    pub fn view_snapshot(&self, uid: u64) -> Option<ViewSnapshot> {
        let d = self.session.documents().iter().find(|d| d.uid == uid)?;
        let si = d.wb.active_sheet;
        let sh = d.wb.sheets.get(si)?;
        let scrolled = self.views.get(&(uid, si)).filter(|_| sh.view == SheetViewMode::Normal);
        let top_left = match scrolled {
            Some(v) => {
                let (fr, fc) = sh.freeze.unwrap_or((0, 0));
                // A hair past the edge so a row that starts exactly at the offset is the one found.
                let row = sh.row_at(sh.row_top(fr) + v.scroll.y.max(0.0) as f64 + 0.01);
                let col = sh.col_at(sh.col_left(fc) + v.scroll.x.max(0.0) as f64 + 0.01);
                CellRef::new(row, col)
            }
            None => sh.view_top_left,
        };
        Some(ViewSnapshot { sheet: sh.name.clone(), selection: d.selection.clone(), top_left })
    }

    /// Shows `view` in open document `uid` when its sheet still exists and is visible; otherwise
    /// the document keeps the view it opened with.
    pub fn restore_view(&mut self, uid: u64, view: &ViewSnapshot) {
        let Some(d) = self.session.document_mut(uid) else { return };
        let Some(si) = d.wb.sheet_index(&view.sheet) else { return };
        let Some(sh) = d.wb.sheets.get(si).filter(|sh| sh.visibility == Visibility::Visible) else { return };
        let (fr, fc) = sh.freeze.unwrap_or((0, 0));
        let top = view.top_left;
        let y = sh.row_top(top.row.max(fr)) - sh.row_top(fr);
        let x = sh.col_left(top.col.max(fc)) - sh.col_left(fc);
        let scroll = egui::vec2(x.max(0.0) as f32, y.max(0.0) as f32);
        let normal = sh.view == SheetViewMode::Normal;
        d.wb_switch_sheet(si);
        d.selection = view.selection.clone();
        if normal {
            self.views.insert((uid, si), SheetView { scroll });
        }
    }
}
