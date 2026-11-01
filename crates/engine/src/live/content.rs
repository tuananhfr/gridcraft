//! What counts as document content for co-editing, and which region a change touched.
//! Recalculated values, spills and the view (active cell, scroll, zoom) are not content: they
//! differ between clients without anyone editing.

use std::sync::Arc;

use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_model::{Cell, Formula, Sheet, Style, Workbook};

/// A cell as typed: its formula or constant, its legacy array range and its style by value
/// (style ids differ between clients).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CellData {
    pub input: Input,
    pub array: Option<RangeRef>,
    pub style: Style,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Input {
    Formula(String),
    Value(Value),
}

/// `None` for a cell that holds nothing.
pub(crate) fn cell_data(wb: &Workbook, cell: Option<&Cell>) -> Option<CellData> {
    let cell = cell?;
    let (input, array) = match &cell.formula {
        Some(f) => (Input::Formula(f.text.clone()), f.array),
        None => (Input::Value(cell.value.clone()), None),
    };
    Some(CellData { input, array, style: wb.styles.get(cell.style).clone() })
}

pub(crate) fn data_at(wb: &Workbook, sheet: usize, at: CellRef) -> Option<CellData> {
    cell_data(wb, wb.sheet(sheet)?.cells.get(at))
}

/// Stores `data` at `at` of sheet `sheet` (removes the cell for `None`).
pub(crate) fn put_cell(wb: &mut Workbook, sheet: usize, at: CellRef, data: Option<&CellData>) {
    let cell = data.map(|d| {
        let style = wb.styles.intern(d.style.clone());
        match &d.input {
            Input::Formula(text) => {
                let mut f = Formula::new(text);
                f.array = d.array;
                Cell { value: Value::Empty, formula: Some(Arc::new(f)), style }
            }
            Input::Value(v) => Cell { value: v.clone(), formula: None, style },
        }
    });
    if let Some(sh) = wb.sheet_mut(sheet) {
        match cell {
            Some(c) => sh.cells.set(at, c),
            None => {
                sh.cells.remove(at);
            }
        }
    }
}

/// Same typed content in two snapshots of one workbook (style ids are comparable there).
fn same_input(a: Option<&Cell>, b: Option<&Cell>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.style == b.style
                && match (&a.formula, &b.formula) {
                    (Some(fa), Some(fb)) => fa.text == fb.text && fa.array == fb.array,
                    (None, None) => a.value == b.value,
                    _ => false,
                }
        }
        _ => false,
    }
}

/// Cells of `after` whose typed content differs from `before`'s (one workbook's snapshots).
pub(crate) fn changed_cells(before: &Sheet, after: &Sheet) -> Vec<CellRef> {
    let mut out = after.cells.diff(&before.cells);
    out.retain(|c| !same_input(before.cells.get(*c), after.cells.get(*c)));
    out.sort_unstable();
    out.dedup();
    out
}

/// Everything of a sheet but its cells and view is the same.
pub(crate) fn props_eq(a: &Sheet, b: &Sheet) -> bool {
    // Destructured so that a new field has to be classed here.
    let Sheet {
        name,
        uid,
        vba_code_name,
        cells: _,
        cell_pictures,
        spill: _,
        spill_ranges: _,
        cols,
        rows,
        default_col_width,
        default_row_height,
        merges,
        freeze,
        tab_color,
        visibility,
        show_gridlines,
        show_headings,
        show_formulas,
        show_zeros,
        right_to_left,
        zoom: _,
        cond_formats,
        validations,
        tables,
        autofilter,
        comments,
        hyperlinks,
        charts,
        images,
        shapes,
        sparklines,
        slicers,
        pivots,
        protection,
        edit_ranges,
        print,
        view_active: _,
        view_top_left: _,
        view: _,
        preserved: _,
    } = a;
    *name == b.name
        && *uid == b.uid
        && *vba_code_name == b.vba_code_name
        && *cell_pictures == b.cell_pictures
        && *cols == b.cols
        && *rows == b.rows
        && *default_col_width == b.default_col_width
        && *default_row_height == b.default_row_height
        && *merges == b.merges
        && *freeze == b.freeze
        && *tab_color == b.tab_color
        && *visibility == b.visibility
        && *show_gridlines == b.show_gridlines
        && *show_headings == b.show_headings
        && *show_formulas == b.show_formulas
        && *show_zeros == b.show_zeros
        && *right_to_left == b.right_to_left
        && *cond_formats == b.cond_formats
        && *validations == b.validations
        && *tables == b.tables
        && *autofilter == b.autofilter
        && *comments == b.comments
        && *hyperlinks == b.hyperlinks
        && *charts == b.charts
        && *images == b.images
        && *shapes == b.shapes
        && *sparklines == b.sparklines
        && *slicers == b.slicers
        && *pivots == b.pivots
        && *protection == b.protection
        && *edit_ranges == b.edit_ranges
        && *print == b.print
}

/// Workbook-level content other than the sheets, styles and names is the same.
fn book_props_eq(a: &Workbook, b: &Workbook) -> bool {
    let Workbook {
        sheets: _,
        names: _,
        styles: _,
        theme,
        date_system,
        calc,
        active_sheet: _,
        props,
        protected_structure,
        structure_password,
        custom_lists,
        cell_styles,
        vba_project,
        vba_signatures,
        legacy_vba_signature,
        vba_signature_loss,
        macro_buttons,
        csv_delimiter: _,
        preserved: _,
        source_dir: _,
        structure: _,
    } = a;
    *theme == b.theme
        && *date_system == b.date_system
        && *calc == b.calc
        && *props == b.props
        && *protected_structure == b.protected_structure
        && *structure_password == b.structure_password
        && *custom_lists == b.custom_lists
        && *cell_styles == b.cell_styles
        && *vba_project == b.vba_project
        && *vba_signatures == b.vba_signatures
        && *legacy_vba_signature == b.legacy_vba_signature
        && *vba_signature_loss == b.vba_signature_loss
        && *macro_buttons == b.macro_buttons
}

/// The sheet keeps its place among the others: same uid, name and visibility at each position.
fn same_sheet_list(a: &Workbook, b: &Workbook) -> bool {
    a.sheets.len() == b.sheets.len()
        && a.sheets.iter().zip(&b.sheets).all(|(x, y)| x.uid == y.uid && x.name == y.name && x.visibility == y.visibility)
}

/// The region one edit touched, from the workbook before and after it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Scope {
    None,
    Cell {
        sheet: usize,
        cell: CellRef,
    },
    /// `sheet` changed as a whole; `others` lists the cells other sheets changed with it
    /// (formulas following a row insert); `names` whether the defined names changed too.
    Sheet {
        sheet: usize,
        others: Vec<(usize, Vec<CellRef>)>,
        names: bool,
    },
    Book,
}

pub(crate) fn scope(before: &Workbook, after: &Workbook, active: usize) -> Scope {
    if !book_props_eq(before, after) || !same_sheet_list(before, after) {
        return Scope::Book;
    }
    let names = before.names != after.names;
    let structural: Vec<String> = if before.structure_seq() == after.structure_seq() {
        Vec::new()
    } else {
        match after.structure.steps_since(before.structure_seq()) {
            Some(steps) => steps.into_iter().flat_map(|(s, to, _)| std::iter::once(s).chain(to)).collect(),
            None => return Scope::Book,
        }
    };
    // (sheet, whole sheet changed, cells changed)
    let mut changed: Vec<(usize, bool, Vec<CellRef>)> = Vec::new();
    for (i, (a, b)) in before.sheets.iter().zip(&after.sheets).enumerate() {
        let moved = structural.contains(&b.uid);
        if Arc::ptr_eq(a, b) && !moved {
            continue;
        }
        let whole = moved || !props_eq(a, b);
        let cells = changed_cells(a, b);
        if whole || !cells.is_empty() {
            changed.push((i, whole, cells));
        }
    }
    match changed.as_slice() {
        [] if names => Scope::Book,
        [] => Scope::None,
        [(sheet, false, cells)] if cells.len() == 1 && !names => Scope::Cell { sheet: *sheet, cell: cells[0] },
        [(sheet, ..)] => Scope::Sheet { sheet: *sheet, others: Vec::new(), names },
        _ => {
            let Some(primary) = changed.iter().position(|(i, ..)| *i == active) else { return Scope::Book };
            let mut others = Vec::new();
            for (k, (i, whole, cells)) in changed.into_iter().enumerate() {
                if k == primary {
                    continue;
                }
                if whole {
                    return Scope::Book;
                }
                others.push((i, cells));
            }
            Scope::Sheet { sheet: active, others, names }
        }
    }
}

/// Two sheets have the same content, by value (they may come from different workbooks).
pub(crate) fn sheet_content_eq(wa: &Workbook, a: &Sheet, wb: &Workbook, b: &Sheet) -> bool {
    if !props_eq(a, b) || a.cells.len() != b.cells.len() {
        return false;
    }
    a.cells.iter().all(|(at, cell)| cell_data(wa, Some(cell)) == cell_data(wb, b.cells.get(at)))
}

/// Two workbooks have the same content, by value.
pub(crate) fn book_content_eq(a: &Workbook, b: &Workbook) -> bool {
    book_props_eq(a, b)
        && same_sheet_list(a, b)
        && a.names == b.names
        && a.sheets.iter().zip(&b.sheets).all(|(x, y)| Arc::ptr_eq(x, y) || sheet_content_eq(a, x, b, y))
}
