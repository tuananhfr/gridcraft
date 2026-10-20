//! Excel keyboard shortcuts beyond the plain Ctrl/Cmd letters in [`crate::ribbon::shortcut`]:
//! number-format and border combinations, F-keys, Alt/Option combinations and the Excel for Mac
//! variants. Every entry maps to an existing command or dialog. `docs/ui-parity.md` holds the
//! audit against Microsoft's published Windows and Mac shortcut lists.

use egui::{Key, Modifiers};
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::model::Sheet;
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::grid::Geo;

/// What a shortcut does.
#[derive(Clone, Debug, PartialEq)]
pub enum Shortcut {
    /// Run a command with these parameters.
    Run(&'static str, Json),
    /// Open a dialog.
    Dialog(&'static str, Json),
    /// Start editing with the current date or time (Ctrl+; and Ctrl+Shift+;).
    Date,
    Time,
    /// Ctrl+' / Ctrl+Shift+": the formula / value of the cell above, ready to edit.
    FormulaAbove,
    ValueAbove,
    /// Extend the selection to A1, to the last used cell, or to the start of the row.
    ExtendToStart,
    ExtendToEnd,
    ExtendToRowStart,
    /// Alt+Page Down / Alt+Page Up: one screen right / left.
    ScreenRight,
    ScreenLeft,
    /// Excel for Mac Control+U: edit the active cell (like F2).
    Edit,
    /// Shift+F10: the cell context menu.
    ContextMenu,
    /// Alt+Down: the active cell's drop-down list.
    DropDown,
    /// Ctrl+Alt+= / Ctrl+Alt+-: zoom in / out.
    Zoom(i32),
    /// Ctrl+Y: redo, or repeat the last action when there is nothing to redo (Excel).
    RedoOrRepeat,
}

fn run(id: &'static str) -> Shortcut {
    Shortcut::Run(id, json!({}))
}

fn number_format(code: &str) -> Shortcut {
    Shortcut::Run("home.numberFormat", json!({"code": code}))
}

fn borders(preset: &str) -> Shortcut {
    Shortcut::Run("home.borders", json!({"preset": preset}))
}

/// The shortcut for a key press in the grid, if any. `physical` is the key's position on a US
/// layout (as Excel matches Ctrl+Shift+4 rather than the `$` character); `mac` selects the Excel
/// for Mac variants where they differ. Navigation keys go by the logical key: with Num Lock off
/// the numpad's arrows and Page Down report the digit keys (Num6, Num3) as their physical keys.
pub fn lookup(key: Key, physical: Option<Key>, m: Modifiers, mac: bool) -> Option<Shortcut> {
    use Key::*;
    let navigation = matches!(key, ArrowLeft | ArrowRight | ArrowUp | ArrowDown | PageUp | PageDown | Home | End | Insert | Delete);
    let k = if navigation { key } else { physical.unwrap_or(key) };
    // Control itself (Excel for Mac also uses Control for these), Ctrl on Windows / Cmd on Mac,
    // and Alt/Option alone.
    let ctrl = m.ctrl && !m.alt;
    let cmd = m.command && !m.alt;
    let alt = m.alt && !m.ctrl && !m.command;
    let ctrl_alt = m.alt && (m.ctrl || m.command);
    let plain = !m.ctrl && !m.command && !m.alt;
    let s = m.shift;
    Some(match k {
        // Ctrl+Shift+~ ! @ # $ % ^ & _ : number formats and borders (Windows and Mac).
        Backtick if ctrl && s => Shortcut::Run("home.numberFormat", json!({"format": "General"})),
        Num1 if ctrl && s => number_format("#,##0.00"),
        Num2 if ctrl && s => number_format("h:mm AM/PM"),
        Num3 if ctrl && s => number_format("d-mmm-yy"),
        Num4 if ctrl && s => number_format("$#,##0.00_);($#,##0.00)"),
        // Ctrl+Shift+T reopens like a browser tab; Excel for Mac's Cmd+Shift+T stays AutoSum.
        T if ctrl && s => run("file.reopenClosed"),
        Num5 if ctrl && s => number_format("0%"),
        Num6 if ctrl && s => number_format("0.00E+00"),
        Num7 if ctrl && s => borders("outside"),
        Minus if ctrl && s => borders("none"),
        Num8 if ctrl && s => run("selection.currentRegion"),
        Semicolon if ctrl && s => Shortcut::Time,
        Quote if ctrl && s => Shortcut::ValueAbove,
        Quote if ctrl => Shortcut::FormulaAbove,
        // Windows Ctrl+; is in `ribbon::shortcut`; Excel for Mac uses Control+; and Command+;.
        Semicolon if mac && ctrl => Shortcut::Date,
        Semicolon if mac && cmd && !s => Shortcut::Time,
        // Ctrl+2..5 (Windows).
        Num2 if ctrl && !mac => run("home.bold"),
        Num3 if ctrl && !mac => run("home.italic"),
        Num4 if ctrl && !mac => run("home.underline"),
        Num5 if ctrl && !mac => run("home.strikethrough"),
        // Letters that differ between Windows and Mac.
        L if (cmd || ctrl) && s => run("data.filter"),
        L if cmd && mac => run("home.alignLeft"),
        L if ctrl && mac => Shortcut::Dialog("defineName", json!({})),
        U if ctrl && mac && !s => Shortcut::Edit,
        L if cmd => run("insert.table"),
        E if cmd && mac && !s => run("home.alignCenter"),
        E if ctrl && mac && !s => run("edit.flashFill"),
        F if cmd && s && !mac => Shortcut::Dialog("formatCells", json!({"tab": "Font"})),
        G if cmd && s && !mac => Shortcut::Dialog("statistics", json!({})),
        H if cmd && s && mac => Shortcut::Dialog("find", json!({"replace": true})),
        R if cmd && s && mac => Shortcut::Dialog("sort", json!({})),
        K if cmd && s && mac => run("data.group"),
        J if cmd && s && mac => run("data.ungroup"),
        P if cmd && !s => run("file.print"),
        Period if cmd && s => run("home.increaseFontSize"),
        Comma if cmd && s => run("home.decreaseFontSize"),
        Space if cmd && s => run("edit.selectAll"),
        // Shift+Space selects the row; Ctrl+Space (Windows) / Control+Space (Excel for Mac) the
        // column. Cmd+Space stays free: it is the macOS Spotlight shortcut.
        Space if plain && s => run("selection.row"),
        Space if !mac && cmd && !s => run("selection.column"),
        Space if mac && ctrl && !m.command && !s => run("selection.column"),
        Home if cmd && s => Shortcut::ExtendToStart,
        End if cmd && s => Shortcut::ExtendToEnd,
        Home if plain && s => Shortcut::ExtendToRowStart,
        // Ctrl+Alt / Cmd+Option combinations.
        V if ctrl_alt || (mac && m.ctrl && m.command) => Shortcut::Dialog("pasteSpecial", json!({})),
        Equals | Plus if ctrl_alt => Shortcut::Zoom(10),
        Minus if ctrl_alt && !mac => Shortcut::Zoom(-10),
        F9 if ctrl_alt => run("formulas.calculateNow"),
        R if ctrl_alt && mac => run("view.collapseRibbon"),
        Num0 if ctrl_alt && mac => borders("outside"),
        Minus if ctrl_alt && mac => borders("none"),
        ArrowLeft if ctrl_alt && mac => borders("left"),
        ArrowRight if ctrl_alt && mac => borders("right"),
        ArrowUp if ctrl_alt && mac => borders("top"),
        ArrowDown if ctrl_alt && mac => borders("bottom"),
        // Alt / Option alone.
        Equals if alt && !s && !mac => run("formulas.autoSum"),
        ArrowDown if alt && !s => Shortcut::DropDown,
        PageDown if alt => Shortcut::ScreenRight,
        PageUp if alt => Shortcut::ScreenLeft,
        ArrowRight if alt && s && !mac => run("data.group"),
        ArrowLeft if alt && s && !mac => run("data.ungroup"),
        ArrowRight if alt && !s && mac => run("sheet.next"),
        ArrowLeft if alt && !s && mac => run("sheet.previous"),
        F1 if alt && s => run("home.insertSheet"),
        F1 if alt => run("insert.chart"),
        F10 if alt && s => Shortcut::Dialog("errorChecking", json!({})),
        // Function keys.
        F1 if cmd && !s => run("view.collapseRibbon"),
        F1 if plain && !s => Shortcut::Dialog("shortcuts", json!({})),
        F2 if (cmd || ctrl) && s => Shortcut::Dialog("comment", json!({"threaded": true})),
        F2 if plain && s => Shortcut::Dialog("comment", json!({"threaded": false})),
        F3 if cmd && s => run("formulas.createFromSelection"),
        F3 if cmd && mac => Shortcut::Dialog("defineName", json!({})),
        F3 if cmd => Shortcut::Dialog("nameManager", json!({})),
        F3 if plain && s => Shortcut::Dialog("insertFunction", json!({})),
        F4 if cmd && !s => run("file.close"),
        F4 if plain && !s => run("edit.repeat"),
        F5 if plain && s => Shortcut::Dialog("find", json!({})),
        F5 if plain => Shortcut::Dialog("goTo", json!({})),
        F7 if plain && !s => Shortcut::Dialog("spelling", json!({})),
        F9 if plain && s => run("formulas.calculateSheet"),
        F10 if plain && s => Shortcut::ContextMenu,
        F11 if plain && s => run("home.insertSheet"),
        F12 if plain && !s => run("file.saveAs"),
        _ => return None,
    })
}

/// Handles a key press in the grid. Returns whether it was a shortcut.
pub(crate) fn dispatch(app: &mut SheetApp, ctx: &egui::Context, key: Key, physical: Option<Key>, m: Modifiers, geo: &Geo, sh: &Sheet) -> bool {
    let mac = ctx.os() == egui::os::OperatingSystem::Mac;
    // AltGr arrives as Ctrl+Alt on Windows: a key that also typed a character (AltGr+= is `~`
    // on a German layout) is text, not a shortcut.
    if !mac && m.alt && m.ctrl && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(_)))) {
        return false;
    }
    let Some(shortcut) = lookup(key, physical, m, mac) else { return false };
    perform(app, shortcut, m, geo, sh);
    true
}

/// Carries out a grid shortcut; `m` is the key press's modifiers.
pub(crate) fn perform(app: &mut SheetApp, shortcut: Shortcut, m: Modifiers, geo: &Geo, sh: &Sheet) {
    let (active, anchor) = app.session.active().map(|d| (d.selection.active, d.selection.anchor)).unwrap_or_default();
    match shortcut {
        Shortcut::Run(id, params) => {
            app.run_or_alert(id, params);
            app.grid.ensure_visible = true;
        }
        Shortcut::Dialog(name, params) => app.open_dialog(name, params),
        Shortcut::Date | Shortcut::Time => {
            let text = app
                .session
                .active()
                .map(|d| {
                    let now = gridcraft_engine::core::date::serial_in_system(d.wb.date_system, gridcraft_engine::calc::now_serial());
                    let (value, format) = if shortcut == Shortcut::Date { (now.floor(), "m/d/yyyy") } else { (now.fract(), "h:mm AM/PM") };
                    gridcraft_engine::display::format_in(&gridcraft_engine::core::Value::Number(value), format, &d.wb, app.ui.region()).text
                })
                .unwrap_or_default();
            edit_with(app, text);
        }
        Shortcut::FormulaAbove | Shortcut::ValueAbove => {
            let Some(above) = active.row.checked_sub(1).map(|r| CellRef::new(r, active.col)) else { return };
            let text = if shortcut == Shortcut::FormulaAbove {
                app.session.active().map(|d| gridcraft_engine::display::edit_text_in(&d.wb, sh, above, app.ui.region())).unwrap_or_default()
            } else {
                app.session
                    .active()
                    .and_then(|d| sh.value_ref(above).map(|v| gridcraft_engine::display::format_in(v, "General", &d.wb, app.ui.region()).text))
                    .unwrap_or_default()
            };
            edit_with(app, text);
        }
        Shortcut::ExtendToStart => extend(app, anchor, active, CellRef::new(0, 0)),
        Shortcut::ExtendToEnd => {
            let end = sh.used_range().map(|u| u.end).unwrap_or_default();
            extend(app, anchor, active, end);
        }
        Shortcut::ExtendToRowStart => extend(app, anchor, active, CellRef::new(active.row, 0)),
        Shortcut::ScreenRight | Shortcut::ScreenLeft => {
            let width = ((geo.cells.width() - geo.frozen_w) / geo.z).max(1.0);
            let cols = (width / sh.default_col_width.max(1.0)).floor().max(1.0) as i64;
            let dc = if shortcut == Shortcut::ScreenRight { cols } else { -cols };
            let _ = app.session.run("selection.move", json!({"dr": 0, "dc": dc, "extend": m.shift}));
            app.grid.ensure_visible = true;
            app.grid.reveal_moving_end = m.shift;
        }
        Shortcut::Edit => app.begin_edit(None, false),
        Shortcut::ContextMenu => app.grid.context_menu = Some(geo.cell_rect(sh, active).center()),
        Shortcut::DropDown => {
            if sh.validations.iter().any(|d| d.in_cell_dropdown && d.ranges.iter().any(|r| r.contains(active))) {
                app.grid.list_picker = Some(active);
            } else if sh.has_filter_button(active) {
                let at = geo.mirror_pos(crate::grid::filter_button_rect(geo, sh, active).center());
                app.grid.filter_menu = Some((active.col, active.row, at));
            }
        }
        Shortcut::Zoom(step) => {
            let percent = (i32::from(sh.zoom).saturating_add(step)).clamp(10, 400);
            app.run_or_alert("view.zoom", json!({"percent": percent}));
        }
        Shortcut::RedoOrRepeat => {
            let can_redo = app.session.active().is_some_and(|d| !d.redo.is_empty());
            app.run_or_alert(if can_redo { "edit.redo" } else { "edit.repeat" }, json!({}));
            app.grid.ensure_visible = true;
        }
    }
}

/// Starts editing the active cell with `text` and the caret at its end.
fn edit_with(app: &mut SheetApp, text: String) {
    app.begin_edit(Some(text), false);
    if let Some(ed) = app.editor.as_mut() {
        ed.caret = ed.text.chars().count();
    }
}

/// Selects `anchor`..`to`, keeping the anchor and the active cell.
fn extend(app: &mut SheetApp, anchor: CellRef, active: CellRef, to: CellRef) {
    let _ = app.session.run("selection.set", json!({"range": RangeRef::new(anchor, to).a1(), "active": active.a1()}));
    if let Some(d) = app.session.active_mut() {
        d.selection.anchor = anchor;
    }
    app.grid.ensure_visible = true;
    app.grid.reveal_moving_end = true;
}

/// One row of the Keyboard Shortcuts dialog (F1): a Windows key combination and what it does.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: Key,
    pub modifiers: Modifiers,
    pub shortcut: Shortcut,
    /// English; translated when shown.
    pub label: &'static str,
}

impl Entry {
    /// The key combination in Windows notation, e.g. `Ctrl+Shift+S`.
    pub fn keys(&self) -> String {
        let m = self.modifiers;
        let mut out = String::new();
        for (on, name) in [(m.ctrl || m.command, "Ctrl+"), (m.alt, "Alt+"), (m.shift, "Shift+")] {
            if on {
                out.push_str(name);
            }
        }
        out.push_str(match self.key {
            Key::Minus => "-",
            Key::Equals => "=",
            Key::Semicolon => ";",
            Key::Quote => "'",
            Key::Backtick => "`",
            Key::Comma => ",",
            Key::Period => ".",
            Key::PageDown => "Page Down",
            Key::PageUp => "Page Up",
            other => other.name(),
        });
        out
    }

    /// Whether `query` appears in the label (in `language` or English) or the keys.
    pub fn matches(&self, query: &str, language: crate::i18n::Language) -> bool {
        let q = query.trim().to_lowercase();
        q.is_empty()
            || language.tr(self.label).to_lowercase().contains(&q)
            || self.label.to_lowercase().contains(&q)
            || self.keys().to_lowercase().contains(&q)
    }
}

/// What a grid key press does on Windows, in `grid::keyboard`'s order: [`lookup`], then the
/// Ctrl+letter table. `None` for the keys the grid handles itself (F2, F9, Delete…).
pub fn resolve(key: Key, m: Modifiers) -> Option<Shortcut> {
    lookup(key, Some(key), m, false).or_else(|| if m.command { crate::ribbon::shortcut(key, m) } else { None })
}

/// Every grid shortcut on Windows, for the Keyboard Shortcuts dialog. Tests check each row
/// against [`resolve`] (or, for the grid's own keys, a real key press), so it can't drift.
pub fn reference() -> &'static [Entry] {
    static ENTRIES: std::sync::OnceLock<Vec<Entry>> = std::sync::OnceLock::new();
    ENTRIES.get_or_init(build_reference)
}

fn build_reference() -> Vec<Entry> {
    use Key::*;
    const NONE: Modifiers = Modifiers::NONE;
    const SHIFT: Modifiers = Modifiers::SHIFT;
    const ALT: Modifiers = Modifiers::ALT;
    // Ctrl on Windows sets both `ctrl` and `command`.
    const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };
    const CTRL_SHIFT: Modifiers = Modifiers { shift: true, ..CTRL };
    const CTRL_ALT: Modifiers = Modifiers { alt: true, ..CTRL };
    const ALT_SHIFT: Modifiers = Modifiers { alt: true, shift: true, ..NONE };
    let entry = |key, modifiers, shortcut, label| Entry { key, modifiers, shortcut, label };
    // A command's own label, as the ribbon and the command search show it.
    let command = |key, modifiers, id: &'static str| {
        let label = gridcraft_engine::find_command(id).map_or(id, |c| c.label);
        Entry { key, modifiers, shortcut: run(id), label }
    };
    let dialog = |key, modifiers, name, params: Json, label| Entry { key, modifiers, shortcut: Shortcut::Dialog(name, params), label };
    vec![
        // Function keys.
        dialog(F1, NONE, "shortcuts", json!({}), "Keyboard Shortcuts"),
        entry(F1, CTRL, run("view.collapseRibbon"), "Show or Hide the Ribbon"),
        command(F1, ALT, "insert.chart"),
        command(F1, ALT_SHIFT, "home.insertSheet"),
        entry(F2, NONE, Shortcut::Edit, "Edit Cell"),
        dialog(F2, SHIFT, "comment", json!({"threaded": false}), "New Note"),
        dialog(F2, CTRL_SHIFT, "comment", json!({"threaded": true}), "New Comment"),
        dialog(F3, SHIFT, "insertFunction", json!({}), "Insert Function"),
        dialog(F3, CTRL, "nameManager", json!({}), "Name Manager"),
        command(F3, CTRL_SHIFT, "formulas.createFromSelection"),
        command(F4, NONE, "edit.repeat"),
        command(F4, CTRL, "file.close"),
        dialog(F5, NONE, "goTo", json!({}), "Go To"),
        dialog(F5, SHIFT, "find", json!({}), "Find"),
        dialog(F7, NONE, "spelling", json!({}), "Spelling"),
        dialog(F8, ALT, "macros", json!({}), "Macros"),
        command(F9, NONE, "formulas.calculateNow"),
        command(F9, SHIFT, "formulas.calculateSheet"),
        command(F9, CTRL_ALT, "formulas.calculateNow"),
        entry(F10, SHIFT, Shortcut::ContextMenu, "Open the Context Menu"),
        dialog(F10, ALT_SHIFT, "errorChecking", json!({}), "Error Checking"),
        command(F11, SHIFT, "home.insertSheet"),
        command(F12, NONE, "file.saveAs"),
        // File and editing.
        command(N, CTRL, "file.new"),
        command(O, CTRL, "file.open"),
        command(S, CTRL, "file.save"),
        command(S, CTRL_SHIFT, "file.saveAs"),
        command(P, CTRL, "file.print"),
        command(W, CTRL, "file.close"),
        entry(T, CTRL_SHIFT, run("file.reopenClosed"), "Reopen Closed Document"),
        command(Z, CTRL, "edit.undo"),
        entry(Y, CTRL, Shortcut::RedoOrRepeat, "Redo or Repeat"),
        command(Z, CTRL_SHIFT, "edit.redo"),
        dialog(V, CTRL_ALT, "pasteSpecial", json!({}), "Paste Special…"),
        dialog(F, CTRL, "find", json!({}), "Find"),
        dialog(H, CTRL, "find", json!({"replace": true}), "Replace"),
        dialog(G, CTRL, "goTo", json!({}), "Go To"),
        dialog(K, CTRL, "insertLink", json!({}), "Insert Hyperlink"),
        command(D, CTRL, "edit.fillDown"),
        command(R, CTRL, "edit.fillRight"),
        command(E, CTRL, "edit.flashFill"),
        command(Delete, NONE, "edit.clearContents"),
        dialog(Minus, CTRL, "deleteCells", json!({}), "Delete Cells…"),
        dialog(Equals, CTRL_SHIFT, "insertCells", json!({}), "Insert Cells…"),
        entry(Semicolon, CTRL, Shortcut::Date, "Insert the Current Date"),
        entry(Semicolon, CTRL_SHIFT, Shortcut::Time, "Insert the Current Time"),
        entry(Quote, CTRL, Shortcut::FormulaAbove, "Copy the Formula from the Cell Above"),
        entry(Quote, CTRL_SHIFT, Shortcut::ValueAbove, "Copy the Value from the Cell Above"),
        // Formatting.
        dialog(Num1, CTRL, "formatCells", json!({}), "Format Cells…"),
        dialog(F, CTRL_SHIFT, "formatCells", json!({"tab": "Font"}), "Format Cells: Font"),
        command(B, CTRL, "home.bold"),
        command(I, CTRL, "home.italic"),
        command(U, CTRL, "home.underline"),
        command(Num2, CTRL, "home.bold"),
        command(Num3, CTRL, "home.italic"),
        command(Num4, CTRL, "home.underline"),
        command(Num5, CTRL, "home.strikethrough"),
        command(Period, CTRL_SHIFT, "home.increaseFontSize"),
        command(Comma, CTRL_SHIFT, "home.decreaseFontSize"),
        entry(Backtick, CTRL_SHIFT, Shortcut::Run("home.numberFormat", json!({"format": "General"})), "General Number Format"),
        entry(Num1, CTRL_SHIFT, number_format("#,##0.00"), "Number Format with Two Decimals"),
        entry(Num2, CTRL_SHIFT, number_format("h:mm AM/PM"), "Time Format"),
        entry(Num3, CTRL_SHIFT, number_format("d-mmm-yy"), "Date Format"),
        entry(Num4, CTRL_SHIFT, number_format("$#,##0.00_);($#,##0.00)"), "Currency Format"),
        entry(Num5, CTRL_SHIFT, number_format("0%"), "Percent Style"),
        entry(Num6, CTRL_SHIFT, number_format("0.00E+00"), "Scientific Format"),
        entry(Num7, CTRL_SHIFT, borders("outside"), "Outside Borders"),
        entry(Minus, CTRL_SHIFT, borders("none"), "No Border"),
        // Rows, columns, tables and data.
        command(Num9, CTRL, "home.hideRows"),
        command(Num9, CTRL_SHIFT, "home.unhideRows"),
        command(Num0, CTRL, "home.hideColumns"),
        command(Num0, CTRL_SHIFT, "home.unhideColumns"),
        command(T, CTRL, "insert.table"),
        command(L, CTRL, "insert.table"),
        command(L, CTRL_SHIFT, "data.filter"),
        command(Equals, ALT, "formulas.autoSum"),
        command(ArrowRight, ALT_SHIFT, "data.group"),
        command(ArrowLeft, ALT_SHIFT, "data.ungroup"),
        entry(ArrowDown, ALT, Shortcut::DropDown, "Open the Drop-down List"),
        command(Backtick, CTRL, "formulas.showFormulas"),
        dialog(G, CTRL_SHIFT, "statistics", json!({}), "Workbook Statistics"),
        // Selection and view.
        command(A, CTRL, "edit.selectAll"),
        command(Space, CTRL_SHIFT, "edit.selectAll"),
        command(Space, SHIFT, "selection.row"),
        command(Space, CTRL, "selection.column"),
        command(Num8, CTRL_SHIFT, "selection.currentRegion"),
        entry(Home, CTRL_SHIFT, Shortcut::ExtendToStart, "Extend the Selection to the Start of the Sheet"),
        entry(End, CTRL_SHIFT, Shortcut::ExtendToEnd, "Extend the Selection to the Last Used Cell"),
        entry(Home, SHIFT, Shortcut::ExtendToRowStart, "Extend the Selection to the Start of the Row"),
        entry(PageDown, ALT, Shortcut::ScreenRight, "One Screen Right"),
        entry(PageUp, ALT, Shortcut::ScreenLeft, "One Screen Left"),
        command(PageDown, CTRL, "sheet.next"),
        command(PageUp, CTRL, "sheet.previous"),
        entry(Equals, CTRL_ALT, Shortcut::Zoom(10), "Zoom In"),
        entry(Minus, CTRL_ALT, Shortcut::Zoom(-10), "Zoom Out"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    /// Keys `grid::keyboard` handles itself; tests/keyboard_shortcuts.rs presses them for real.
    const GRID_KEYS: [(Key, Modifiers); 6] = [
        (Key::F2, Modifiers::NONE),
        (Key::F9, Modifiers::NONE),
        (Key::F8, Modifiers::ALT),
        (Key::Delete, Modifiers::NONE),
        (Key::PageDown, Modifiers { ctrl: true, command: true, ..Modifiers::NONE }),
        (Key::PageUp, Modifiers { ctrl: true, command: true, ..Modifiers::NONE }),
    ];

    #[test]
    fn reference_rows_resolve_to_what_they_say() {
        for e in reference() {
            match resolve(e.key, e.modifiers) {
                Some(found) => assert_eq!(found, e.shortcut, "{} is listed as {}", e.keys(), e.label),
                None => assert!(GRID_KEYS.contains(&(e.key, e.modifiers)), "{} does nothing in the grid", e.keys()),
            }
        }
    }

    #[test]
    fn reference_lists_each_key_combination_once() {
        let mut keys: Vec<String> = reference().iter().map(Entry::keys).collect();
        let n = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), n);
    }

    #[test]
    fn keys_use_windows_notation() {
        let keys = |key, modifiers| Entry { key, modifiers, shortcut: Shortcut::Edit, label: "" }.keys();
        let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
        assert_eq!(keys(Key::S, Modifiers { shift: true, ..ctrl }), "Ctrl+Shift+S");
        assert_eq!(keys(Key::F12, Modifiers::NONE), "F12");
        assert_eq!(keys(Key::F9, Modifiers { alt: true, ..ctrl }), "Ctrl+Alt+F9");
        assert_eq!(keys(Key::Minus, ctrl), "Ctrl+-");
        assert_eq!(keys(Key::PageDown, Modifiers::ALT), "Alt+Page Down");
    }

    #[test]
    fn search_matches_translated_or_english_labels_and_keys() {
        let find = |q: &str, language| reference().iter().filter(|e| e.matches(q, language)).map(Entry::keys).collect::<Vec<_>>();
        assert_eq!(find("", Language::Vi).len(), reference().len());
        assert!(find("lưu như", Language::Vi).contains(&"F12".to_string()));
        assert!(find("save as", Language::Vi).contains(&"Ctrl+Shift+S".to_string()));
        assert_eq!(find("ctrl+shift+s", Language::En), ["Ctrl+Shift+S", "Ctrl+Shift+Space"]);
        assert!(find("PHÍM TẮT", Language::Vi).contains(&"F1".to_string()));
        assert!(find("nothing like this", Language::Vi).is_empty());
    }

    #[test]
    fn every_reference_label_has_a_vietnamese_translation() {
        let missing: Vec<&str> = reference().iter().map(|e| e.label).filter(|l| Language::Vi.tr(l) == *l).collect();
        assert!(missing.is_empty(), "untranslated: {missing:?}");
        assert_eq!(Language::Vi.tr("Keyboard Shortcuts"), "Phím tắt");
    }

    #[test]
    fn f4_repeats_outside_the_cell_editor_and_ctrl_y_falls_back_to_repeat() {
        assert_eq!(lookup(Key::F4, Some(Key::F4), Modifiers::NONE, false), Some(run("edit.repeat")));
        assert_eq!(lookup(Key::F4, Some(Key::F4), Modifiers::SHIFT, false), None);
        let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
        assert_eq!(resolve(Key::Y, ctrl), Some(Shortcut::RedoOrRepeat));
        assert_eq!(resolve(Key::F1, Modifiers::NONE), Some(Shortcut::Dialog("shortcuts", json!({}))));
    }
}
