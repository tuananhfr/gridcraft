//! Excel capabilities granted to a single isolated macro transaction.
use crate::{Session, UiRequest};
use gridcraft_core::{CellRef, MAX_COLS, MAX_ROWS, RangeRef};
use gridcraft_vbarun::{Argument, Array, Clock, Error, Host, Object, Result, Value};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;

const CELL_CAP: u64 = 100_000;
#[derive(Clone)]
enum ExcelObject {
    Application,
    Workbook,
    Sheets,
    Sheet(usize),
    Range(usize, RangeRef),
    Areas(usize, Vec<RangeRef>),
    Dimension(usize, RangeRef, bool),
    Interior(usize, RangeRef),
    Font(usize, RangeRef),
    Functions,
    Global(String),
}
pub(crate) struct ExcelHost {
    pub session: Session,
    objects: Vec<ExcelObject>,
    pub messages: Vec<String>,
    pub changed: std::collections::BTreeSet<(usize, CellRef)>,
    properties: BTreeMap<String, Value>,
    analysis: Option<std::sync::Arc<gridcraft_vba::Project>>,
    edit_bytes: usize,
    message_bytes: usize,
    cancellation: std::sync::Arc<std::sync::atomic::AtomicBool>,
    deadline: u64,
    clock: super::RunClock,
    max_edit_bytes: usize,
}
fn invalid(message: &str) -> Error {
    Error { fatal: message.starts_with("VBA sandbox limit:"), ..Error::runtime(1004, message) }
}
fn engine(error: crate::EngineError) -> Error {
    invalid(&error.to_string())
}
fn arg<'a>(args: &'a [Argument], index: usize, name: &str) -> Option<&'a Value> {
    args.iter()
        .find(|a| a.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .or_else(|| args.get(index).filter(|a| a.name.is_none()))
        .map(|a| &a.value)
        .filter(|v| !matches!(v, Value::Missing))
}
fn number(args: &[Argument], index: usize, name: &str, default: i64) -> Result<i64> {
    if name == "ColumnIndex"
        && let Some(Value::String(s)) = arg(args, index, name)
        && let Some(col) = gridcraft_core::letters_to_col(s)
    {
        return Ok(i64::from(col) + 1);
    }
    arg(args, index, name).map(Value::integer).transpose().map(|v| v.unwrap_or(default))
}
fn bounded(row: i64, col: i64) -> Result<CellRef> {
    if !(0..i64::from(MAX_ROWS)).contains(&row) || !(0..i64::from(MAX_COLS)).contains(&col) {
        return Err(invalid("Cell is outside the worksheet"));
    }
    Ok(CellRef::new(row as u32, col as u32))
}
fn cap(range: RangeRef) -> Result<()> {
    if range.count() > CELL_CAP { Err(invalid("VBA sandbox limit: range cells")) } else { Ok(()) }
}
impl ExcelHost {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            objects: vec![ExcelObject::Application],
            messages: vec![],
            changed: Default::default(),
            properties: BTreeMap::new(),
            analysis: None,
            edit_bytes: 0,
            message_bytes: 0,
            cancellation: Default::default(),
            deadline: u64::MAX,
            clock: super::RunClock::new(),
            max_edit_bytes: 32 * 1024 * 1024,
        }
    }
    pub fn set_budget(&mut self, cancellation: std::sync::Arc<std::sync::atomic::AtomicBool>, wall_ms: u64, memory: usize, clock: super::RunClock) {
        self.cancellation = cancellation;
        self.deadline = wall_ms;
        self.clock = clock;
        self.max_edit_bytes = memory;
    }
    fn check_budget(&self) -> Result<()> {
        if self.cancellation.load(std::sync::atomic::Ordering::Relaxed) || self.clock.monotonic_ms() >= self.deadline {
            return Err(Error { fatal: true, ..invalid("VBA sandbox limit: cancelled or deadline") });
        }
        Ok(())
    }
    pub fn set_analysis(&mut self, analysis: std::sync::Arc<gridcraft_vba::Project>) {
        self.analysis = Some(analysis);
    }
    fn object(&mut self, object: ExcelObject) -> Result<Value> {
        if self.objects.len() >= CELL_CAP as usize {
            return Err(invalid("VBA sandbox limit: object handles"));
        }
        let id = self.objects.len() as u64;
        self.objects.push(object);
        Ok(Value::Object(Some(id)))
    }
    fn resolve(&self, id: Object) -> Result<ExcelObject> {
        usize::try_from(id).ok().and_then(|i| self.objects.get(i)).cloned().ok_or_else(|| invalid("Invalid Excel object"))
    }
    fn active_sheet(&self) -> Result<usize> {
        Ok(self.session.doc().map_err(engine)?.wb.active_sheet)
    }
    fn run(&mut self, command: &str, params: Json) -> Result<Value> {
        self.check_budget()?;
        self.session.execute(command, params).map_err(engine)?;
        Ok(Value::Empty)
    }
    fn range(&mut self, sheet: usize, range: RangeRef) -> Result<Value> {
        self.object(ExcelObject::Range(sheet, range))
    }
    fn range_arg(&self, value: &Value, sheet: usize) -> Result<(usize, RangeRef)> {
        match value {
            Value::Object(Some(id)) => match self.resolve(*id)? {
                ExcelObject::Range(si, range) | ExcelObject::Dimension(si, range, _) => Ok((si, range)),
                _ => Err(invalid("Expected a Range")),
            },
            Value::String(text) => {
                let (si, address) = if let Some((name, address)) = text.rsplit_once('!') {
                    let name = name.trim_matches('\'').replace("''", "'");
                    let si = self
                        .session
                        .doc()
                        .map_err(engine)?
                        .wb
                        .sheets
                        .iter()
                        .position(|s| s.name.eq_ignore_ascii_case(&name))
                        .ok_or_else(|| invalid("Worksheet not found"))?;
                    (si, address)
                } else {
                    (sheet, text.as_str())
                };
                RangeRef::parse(address).map(|range| (si, range)).ok_or_else(|| invalid("Invalid range address"))
            }
            _ => Err(invalid("Expected a Range or address")),
        }
    }
    fn sheet_member(&mut self, sheet: usize, name: &str, args: &[Argument]) -> Result<Value> {
        match name {
            "name" => {
                Ok(Value::String(self.session.doc().map_err(engine)?.wb.sheet(sheet).ok_or_else(|| invalid("Worksheet not found"))?.name.clone()))
            }
            "range" => {
                let first = arg(args, 0, "Cell1").ok_or_else(|| invalid("Range requires an address"))?;
                let (si, mut range) = self.range_arg(first, sheet)?;
                if let Some(last) = arg(args, 1, "Cell2") {
                    let (other, end) = self.range_arg(last, sheet)?;
                    if si != other {
                        return Err(invalid("Ranges must belong to the same worksheet"));
                    }
                    range = range.union(&end);
                }
                self.range(si, range)
            }
            "cells" => self.range_member(sheet, RangeRef::all(), "cells", args),
            "rows" | "columns" => self.range_member(sheet, RangeRef::all(), name, args),
            "usedrange" => {
                let range = self.session.doc().map_err(engine)?.wb.sheet(sheet).and_then(|s| s.used_range()).unwrap_or_default();
                self.range(sheet, range)
            }
            "activate" | "select" => self.run("sheet.activate", json!({"sheet":sheet})),
            "calculate" => {
                let previous = self.active_sheet()?;
                self.run("sheet.activate", json!({"sheet":sheet}))?;
                let result = self.run("formulas.calculateSheet", json!({}));
                self.run("sheet.activate", json!({"sheet":previous}))?;
                result
            }
            "visible" => {
                let visibility = self.session.doc().map_err(engine)?.wb.sheet(sheet).ok_or_else(|| invalid("Worksheet not found"))?.visibility;
                Ok(Value::Long(match visibility {
                    gridcraft_model::Visibility::Visible => -1,
                    gridcraft_model::Visibility::Hidden => 0,
                    gridcraft_model::Visibility::VeryHidden => 2,
                }))
            }
            "protect" | "unprotect" => {
                if arg(args, 0, "Password").map(Value::text).transpose()?.is_some_and(|s| !s.is_empty()) {
                    return Err(Error::unsupported("Password protection"));
                }
                self.run(if name == "protect" { "review.protectSheet" } else { "review.unprotectSheet" }, json!({"sheet":sheet}))
            }
            _ => Err(Error::unsupported(name)),
        }
    }
    fn read_range(&self, sheet: usize, range: RangeRef, member: &str) -> Result<Value> {
        cap(range)?;
        let doc = self.session.doc().map_err(engine)?;
        let sh = doc.wb.sheet(sheet).ok_or_else(|| invalid("Worksheet not found"))?;
        let read = |at: CellRef| -> Result<Value> {
            if matches!(member, "formula" | "formulalocal" | "formular1c1")
                && let Some(formula) = sh.cell(at).and_then(|c| c.formula.as_ref())
            {
                if member == "formulalocal"
                    && let Some(expr) = formula.expr()
                {
                    let locale = gridcraft_formula::FormulaLocale::of(self.session.region);
                    let text = gridcraft_formula::print_input(&expr, locale, |name| doc.wb.name(name, sheet).is_some())
                        .map_err(|e| invalid(&e.to_string()))?;
                    return Ok(Value::String(format!("={text}")));
                }
                if member == "formular1c1"
                    && let Some(expr) = formula.expr()
                {
                    return Ok(Value::String(format!("={}", gridcraft_formula::print_r1c1(&expr, at))));
                }
                return Ok(Value::String(sh.input_text(at)));
            }
            if member == "text" {
                let code = doc.wb.styles.get(sh.style_id(at)).num_fmt.as_str();
                return Ok(Value::String(gridcraft_numfmt::text_function(&sh.value(at), code, doc.wb.date_system).map_err(|e| invalid(e.as_str()))?));
            }
            from_cell(sh.value(at))
        };
        if range.is_single() {
            return read(range.start);
        }
        let mut values = Vec::new();
        let mut bytes = 0usize;
        for at in range.iter() {
            let value = read(at)?;
            bytes = bytes.saturating_add(value.memory());
            if bytes > self.max_edit_bytes {
                return Err(invalid("VBA sandbox limit: range memory"));
            }
            values.push(value);
        }
        Ok(Value::Array(Array {
            bounds: vec![(1, range.height() as i32), (1, range.width() as i32)],
            values,
            element_type: "variant".into(),
            dynamic: false,
        }))
    }
    fn write_range(&mut self, sheet: usize, range: RangeRef, member: &str, value: Value) -> Result<()> {
        cap(range)?;
        if let Value::Array(array) = &value
            && array.bounds.len() != 2
        {
            return Err(invalid("Range.Value requires a two-dimensional array"));
        }
        for at in range.iter() {
            let item = if let Value::Array(array) = &value {
                let (row, col) = (at.row - range.start.row, at.col - range.start.col);
                let lo_row = array.bounds.first().map_or(0, |b| b.0);
                let lo_col = array.bounds.get(1).map_or(0, |b| b.0);
                let offset = array.offset(&[lo_row.saturating_add(row as i32), lo_col.saturating_add(col as i32)])?;
                array.values.get(offset).cloned().ok_or_else(|| invalid("Array cell missing"))?
            } else {
                value.clone()
            };
            let mut input = match &item {
                Value::String(s) => s.clone(),
                Value::Empty | Value::Null => String::new(),
                Value::Boolean(b) => {
                    if *b {
                        "TRUE".into()
                    } else {
                        "FALSE".into()
                    }
                }
                _ => item.text()?,
            };
            if member == "formulalocal" && input.starts_with('=') {
                let doc = self.session.doc().map_err(engine)?;
                let expr = gridcraft_formula::parse_input(
                    input.strip_prefix('=').unwrap_or(&input),
                    gridcraft_formula::FormulaLocale::of(self.session.region),
                    |name| doc.wb.name(name, sheet).is_some(),
                )
                .map_err(|e| invalid(&e.to_string()))?;
                input = format!("={}", gridcraft_formula::print(&expr));
            } else if member == "formular1c1" && input.starts_with('=') {
                input = r1c1_to_a1(&input, at)?;
            } else if matches!(member, "value" | "value2" | "") && matches!(item, Value::String(_)) {
                input.insert(0, '\'');
            }
            self.edit_bytes = self.edit_bytes.saturating_add(input.len()).saturating_add(128);
            if self.edit_bytes > self.max_edit_bytes {
                return Err(invalid("VBA sandbox limit: host edit memory"));
            }
            self.run("cell.set", json!({"sheet":sheet, "cell":at.a1(), "input":input,"region":if member == "formulalocal" { self.session.region.code() } else { "en-US" }}))?;
            self.changed.insert((sheet, at));
        }
        Ok(())
    }
    fn range_member(&mut self, sheet: usize, range: RangeRef, name: &str, args: &[Argument]) -> Result<Value> {
        match name {
            "" if args.is_empty() => self.read_range(sheet, range, "value"),
            "value" | "value2" | "formula" | "formulalocal" | "formular1c1" | "text" => self.read_range(sheet, range, name),
            "" | "item" | "cells" => {
                if args.is_empty() {
                    return self.range(sheet, range);
                }
                let row = number(args, 0, "RowIndex", 1)?.checked_sub(1).ok_or_else(|| invalid("Invalid cell index"))?;
                let at = if arg(args, 1, "ColumnIndex").is_some() {
                    bounded(
                        i64::from(range.start.row).saturating_add(row),
                        i64::from(range.start.col).saturating_add(number(args, 1, "ColumnIndex", 1)?.saturating_sub(1)),
                    )?
                } else {
                    let width = i64::from(range.width());
                    bounded(i64::from(range.start.row).saturating_add(row / width), i64::from(range.start.col).saturating_add(row % width))?
                };
                self.range(sheet, RangeRef::cell(at))
            }
            "range" => {
                let first = arg(args, 0, "Cell1").ok_or_else(|| invalid("Range requires an address"))?;
                let (si, relative) = self.range_arg(first, sheet)?;
                let start =
                    bounded(i64::from(range.start.row) + i64::from(relative.start.row), i64::from(range.start.col) + i64::from(relative.start.col))?;
                let end =
                    bounded(i64::from(range.start.row) + i64::from(relative.end.row), i64::from(range.start.col) + i64::from(relative.end.col))?;
                self.range(si, RangeRef::new(start, end))
            }
            "offset" | "resize" => {
                let (start, end) = if name == "offset" {
                    let row = number(args, 0, "RowOffset", 0)?;
                    let col = number(args, 1, "ColumnOffset", 0)?;
                    (
                        bounded(i64::from(range.start.row).saturating_add(row), i64::from(range.start.col).saturating_add(col))?,
                        bounded(i64::from(range.end.row).saturating_add(row), i64::from(range.end.col).saturating_add(col))?,
                    )
                } else {
                    let rows = number(args, 0, "RowSize", i64::from(range.height()))?;
                    let cols = number(args, 1, "ColumnSize", i64::from(range.width()))?;
                    if rows < 1 || cols < 1 {
                        return Err(invalid("Resize requires positive dimensions"));
                    }
                    (range.start, bounded(i64::from(range.start.row).saturating_add(rows - 1), i64::from(range.start.col).saturating_add(cols - 1))?)
                };
                self.range(sheet, RangeRef::new(start, end))
            }
            "row" => Ok(Value::Long(range.start.row as i32 + 1)),
            "column" => Ok(Value::Long(range.start.col as i32 + 1)),
            "count" | "countlarge" => Ok(Value::Double(range.count() as f64)),
            "rows" | "columns" => {
                let rows = name == "rows";
                if args.is_empty() {
                    return self.object(ExcelObject::Dimension(sheet, range, rows));
                }
                if let Some(Value::String(text)) = arg(args, 0, "Index") {
                    let text = if text.contains(':') { text.clone() } else { format!("{text}:{text}") };
                    let relative = RangeRef::parse(&text).ok_or_else(|| invalid("Invalid row or column index"))?;
                    let target = if rows {
                        let start = bounded(i64::from(range.start.row) + i64::from(relative.start.row), i64::from(range.start.col))?;
                        let end = bounded(i64::from(range.start.row) + i64::from(relative.end.row), i64::from(range.end.col))?;
                        RangeRef::new(start, end)
                    } else {
                        let start = bounded(i64::from(range.start.row), i64::from(range.start.col) + i64::from(relative.start.col))?;
                        let end = bounded(i64::from(range.end.row), i64::from(range.start.col) + i64::from(relative.end.col))?;
                        RangeRef::new(start, end)
                    };
                    return self.range(sheet, target);
                }
                let index = number(args, 0, "Index", 1)?.saturating_sub(1);
                let r = if rows {
                    let at = bounded(i64::from(range.start.row).saturating_add(index), i64::from(range.start.col))?;
                    RangeRef::new(at, CellRef::new(at.row, range.end.col))
                } else {
                    let at = bounded(i64::from(range.start.row), i64::from(range.start.col).saturating_add(index))?;
                    RangeRef::new(at, CellRef::new(range.end.row, at.col))
                };
                self.range(sheet, r)
            }
            "address" => {
                let row_abs = arg(args, 0, "RowAbsolute").map(Value::truth).transpose()?.unwrap_or(true);
                let col_abs = arg(args, 1, "ColumnAbsolute").map(Value::truth).transpose()?.unwrap_or(true);
                let address = |c: CellRef| {
                    format!(
                        "{}{}{}{}",
                        if col_abs { "$" } else { "" },
                        gridcraft_core::col_to_letters(c.col),
                        if row_abs { "$" } else { "" },
                        c.row + 1
                    )
                };
                Ok(Value::String(if range.is_single() { address(range.start) } else { format!("{}:{}", address(range.start), address(range.end)) }))
            }
            "end" => {
                let direction = number(args, 0, "Direction", -4162)?;
                let (dr, dc) = match direction {
                    -4162 => (-1, 0),
                    -4121 => (1, 0),
                    -4159 => (0, -1),
                    -4161 => (0, 1),
                    _ => return Err(invalid("Invalid End direction")),
                };
                let sh = self.session.doc().map_err(engine)?.wb.sheet(sheet).ok_or_else(|| invalid("Worksheet not found"))?;
                let mut at = range.start;
                let next = bounded(i64::from(at.row) + dr, i64::from(at.col) + dc);
                if let Ok(next) = next {
                    let occupied = !sh.value(at).is_empty() && !sh.value(next).is_empty();
                    at = next;
                    while let Ok(next) = bounded(i64::from(at.row) + dr, i64::from(at.col) + dc) {
                        if occupied {
                            if sh.value(next).is_empty() {
                                break;
                            }
                        } else if !sh.value(at).is_empty() {
                            break;
                        }
                        at = next;
                    }
                }
                self.range(sheet, RangeRef::cell(at))
            }
            "find" => {
                let case = arg(args, 6, "MatchCase").map(Value::truth).transpose()?.unwrap_or(false);
                let what = arg(args, 0, "What").ok_or_else(|| invalid("Find requires What"))?.text()?;
                let what = if case { what } else { what.to_lowercase() };
                let whole = number(args, 3, "LookAt", 2)? == 1;
                let formulas = number(args, 2, "LookIn", -4163)? == -4123;
                let columns = number(args, 4, "SearchOrder", 1)? == 2;
                let previous = number(args, 5, "SearchDirection", 1)? == 2;
                let after = if let Some(value) = arg(args, 1, "After") {
                    let (si, r) = self.range_arg(value, sheet)?;
                    if si != sheet || !r.is_single() || !range.contains(r.start) {
                        return Err(invalid("Find.After must be a cell in the range"));
                    }
                    r.start
                } else {
                    range.start
                };
                let key = |at: CellRef| if columns { (at.col, at.row) } else { (at.row, at.col) };
                let mut first: Option<CellRef> = None;
                let mut next: Option<CellRef> = None;
                let sh = self.session.doc().map_err(engine)?.wb.sheet(sheet).ok_or_else(|| invalid("Worksheet not found"))?;
                for (index, (at, cell)) in sh.cells.iter().enumerate() {
                    if index % 256 == 0 {
                        self.check_budget()?;
                    }
                    if !range.contains(at) {
                        continue;
                    }
                    let text = if formulas && cell.formula.is_some() { sh.input_text(at) } else { sh.value(at).to_text().unwrap_or_default() };
                    let text = if case { text } else { text.to_lowercase() };
                    if if whole { text != what } else { !text.contains(&what) } {
                        continue;
                    }
                    let better = |old: Option<CellRef>| old.is_none_or(|old| if previous { key(at) > key(old) } else { key(at) < key(old) });
                    if better(first) {
                        first = Some(at);
                    }
                    if (if previous { key(at) < key(after) } else { key(at) > key(after) }) && better(next) {
                        next = Some(at);
                    }
                }
                if let Some(at) = next.or(first) { self.range(sheet, RangeRef::cell(at)) } else { Ok(Value::Object(None)) }
            }
            "clearcontents" | "clear" => {
                cap(range)?;
                self.run(if name == "clear" { "edit.clearAll" } else { "edit.clearContents" }, json!({"sheet":sheet,"range":range.a1()}))?;
                self.changed.extend(range.iter().map(|at| (sheet, at)));
                Ok(Value::Empty)
            }
            "copy" => {
                cap(range)?;
                if let Some(destination) = arg(args, 0, "Destination") {
                    let (si, target) = self.range_arg(destination, sheet)?;
                    let values = self.read_range(sheet, range, "value")?;
                    let end = bounded(
                        i64::from(target.start.row) + i64::from(range.height()) - 1,
                        i64::from(target.start.col) + i64::from(range.width()) - 1,
                    )?;
                    self.write_range(si, RangeRef::new(target.start, end), "value", values)?;
                    Ok(Value::Empty)
                } else {
                    self.run("edit.copy", json!({"sheet":sheet,"range":range.a1()}))
                }
            }
            "pastespecial" => {
                let paste = number(args, 0, "Paste", -4163)?;
                if paste != -4163 {
                    return Err(Error::unsupported("PasteSpecial supports values only"));
                }
                let clip = self.session.clipboard.clone().ok_or_else(|| invalid("Clipboard is empty"))?;
                let source = clip.wb.sheet(clip.sheet).ok_or_else(|| invalid("Clipboard worksheet missing"))?;
                cap(clip.range)?;
                let values = clip.range.iter().map(|at| from_cell(source.value(at))).collect::<Result<Vec<_>>>()?;
                let array = Value::Array(Array {
                    bounds: vec![(1, clip.range.height() as i32), (1, clip.range.width() as i32)],
                    values,
                    element_type: "variant".into(),
                    dynamic: false,
                });
                let end = bounded(
                    i64::from(range.start.row) + i64::from(clip.range.height()) - 1,
                    i64::from(range.start.col) + i64::from(clip.range.width()) - 1,
                )?;
                self.write_range(sheet, RangeRef::new(range.start, end), "value", array)?;
                Ok(Value::Empty)
            }
            "interior" => self.object(ExcelObject::Interior(sheet, range)),
            "font" => self.object(ExcelObject::Font(sheet, range)),
            "numberformat" => {
                let doc = self.session.doc().map_err(engine)?;
                let sh = doc.wb.sheet(sheet).ok_or_else(|| invalid("Worksheet not found"))?;
                Ok(Value::String(doc.wb.styles.get(sh.style_id(range.start)).num_fmt.as_str().into()))
            }
            "merge" | "unmerge" => {
                cap(range)?;
                self.run(if name == "merge" { "home.mergeCells" } else { "home.unmergeCells" }, json!({"sheet":sheet,"range":range.a1()}))
            }
            "autofit" => Ok(Value::Empty),
            "select" | "activate" => self.run("selection.set", json!({"sheet":sheet,"range":range.a1()})),
            _ => Err(Error::unsupported(name)),
        }
    }
    fn application(&mut self, name: &str, args: &[Argument]) -> Result<Value> {
        match name {
            "application" => Ok(Value::Object(Some(0))),
            "activeworkbook" | "thisworkbook" => self.object(ExcelObject::Workbook),
            "activesheet" => self.object(ExcelObject::Sheet(self.active_sheet()?)),
            "sheets" | "worksheets" => {
                if args.is_empty() {
                    self.object(ExcelObject::Sheets)
                } else {
                    self.sheets(args)
                }
            }
            "activecell" | "selection" => {
                let doc = self.session.doc().map_err(engine)?;
                self.range(doc.wb.active_sheet, if name == "activecell" { RangeRef::cell(doc.selection.active) } else { doc.selection.current() })
            }
            "range" | "cells" | "rows" | "columns" => self.sheet_member(self.active_sheet()?, name, args),
            "worksheetfunction" => self.object(ExcelObject::Functions),
            "calculate" => self.run("formulas.calculateNow", json!({})),
            "screenupdating" | "enableevents" | "displayalerts" => Ok(self.properties.get(name).cloned().unwrap_or(Value::Boolean(true))),
            "statusbar" => Ok(self.properties.get(name).cloned().unwrap_or(Value::Boolean(false))),
            "calculation" => {
                let mode = &self.session.doc().map_err(engine)?.wb.calc.mode;
                Ok(Value::Long(match mode {
                    gridcraft_model::CalcMode::Manual => -4135,
                    gridcraft_model::CalcMode::AutomaticExceptTables => 2,
                    _ => -4105,
                }))
            }
            "getopenfilename" => {
                self.notice("macro.filePicker", args, "GetOpenFilename returns False until file bindings are supported");
                Ok(Value::Boolean(false))
            }
            "inputbox" => self.input_box(args),
            "wait" => {
                if arg(args, 0, "Time").is_some() {
                    let _ = arg(args, 0, "Time").map(Value::number).transpose()?;
                }
                Ok(Value::Boolean(true))
            }
            "union" | "intersect" => {
                let mut sets = Vec::new();
                let mut sheet = None;
                for argument in args {
                    let (si, ranges) = self.parts(&argument.value)?;
                    if sheet.is_some_and(|sheet| sheet != si) {
                        return Err(invalid("Ranges must belong to one worksheet"));
                    }
                    sheet = Some(si);
                    let mut cells = std::collections::BTreeSet::new();
                    for r in ranges {
                        cap(r)?;
                        cells.extend(r.iter());
                        if cells.len() > CELL_CAP as usize {
                            return Err(invalid("VBA sandbox limit: range cells"));
                        }
                    }
                    sets.push(cells);
                }
                let mut iter = sets.into_iter();
                let mut cells = iter.next().ok_or_else(|| invalid("Expected ranges"))?;
                for other in iter {
                    if name == "union" {
                        cells.extend(other);
                    } else {
                        cells.retain(|at| other.contains(at));
                    }
                    if cells.len() > CELL_CAP as usize {
                        return Err(invalid("VBA sandbox limit: range cells"));
                    }
                }
                if cells.is_empty() {
                    return Ok(Value::Object(None));
                }
                let ranges = rectangles(cells);
                if ranges.len() == 1
                    && let Some(range) = ranges.first()
                {
                    return self.range(sheet.unwrap_or(0), *range);
                }
                self.object(ExcelObject::Areas(sheet.unwrap_or(0), ranges))
            }
            _ => Err(Error::unsupported(name)),
        }
    }
    fn parts(&self, value: &Value) -> Result<(usize, Vec<RangeRef>)> {
        if let Value::Object(Some(id)) = value {
            return match self.resolve(*id)? {
                ExcelObject::Range(si, r) | ExcelObject::Dimension(si, r, _) => Ok((si, vec![r])),
                ExcelObject::Areas(si, ranges) => Ok((si, ranges)),
                _ => Err(invalid("Expected range")),
            };
        }
        Err(invalid("Expected range"))
    }
    fn sheets(&mut self, args: &[Argument]) -> Result<Value> {
        let value = arg(args, 0, "Index").ok_or_else(|| invalid("Worksheet index required"))?;
        let doc = self.session.doc().map_err(engine)?;
        let si = if let Value::String(name) = value {
            doc.wb.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(name)).ok_or_else(|| invalid("Worksheet not found"))?
        } else {
            let index = value.integer()?.checked_sub(1).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| invalid("Invalid worksheet index"))?;
            if doc.wb.sheet(index).is_none() {
                return Err(invalid("Worksheet not found"));
            }
            index
        };
        self.object(ExcelObject::Sheet(si))
    }
    fn notice(&mut self, name: &str, args: &[Argument], message: &str) {
        let bytes = args.iter().fold(message.len(), |n, a| n.saturating_add(a.value.memory()));
        if self.edit_bytes.saturating_add(bytes) > self.max_edit_bytes {
            return;
        }
        self.edit_bytes = self.edit_bytes.saturating_add(bytes);
        if self.messages.len() < 1000 && self.message_bytes.saturating_add(message.len()) <= 64 * 1024 {
            self.message_bytes = self.message_bytes.saturating_add(message.len());
            self.messages.push(message.into());
        }
        if self.session.ui_requests.len() < 1000 {
            let prompt = arg(args, 0, "Prompt").and_then(|v| v.text().ok()).unwrap_or_default();
            self.session.ui_requests.push(UiRequest::Dialog(
                name.into(),
                json!({"prompt":prompt,"message":message,"headlessDefault":true,
                "buttons":arg(args,1,"Buttons").and_then(|v|v.integer().ok()),
                "title":arg(args,if name == "macro.inputBox" {1} else {2},"Title").and_then(|v|v.text().ok()),
                "default":arg(args,2,"Default").and_then(|v|v.text().ok())}),
            ));
        }
    }
}
impl Host for ExcelHost {
    async fn set_member_async(&mut self, id: Object, name: &str, args: &[Argument], value: Value) -> Result<()> {
        let name = name.to_ascii_lowercase();
        let object = self.resolve(id)?;
        let object = match &object {
            ExcelObject::Global(global) => {
                let Value::Object(Some(id)) = self.application(global, args)? else { return Err(invalid("Expected range")) };
                self.resolve(id)?
            }
            ExcelObject::Sheet(si) if matches!(name.as_str(), "cells" | "range") => {
                let Value::Object(Some(id)) = self.sheet_member(*si, &name, args)? else { return Err(invalid("Expected range")) };
                self.resolve(id)?
            }
            ExcelObject::Range(si, r) if matches!(name.as_str(), "" | "cells" | "range") && !args.is_empty() => {
                let member = if name.is_empty() { "item" } else { &name };
                let Value::Object(Some(id)) = self.range_member(*si, *r, member, args)? else { return Err(invalid("Expected range")) };
                self.resolve(id)?
            }
            _ => object,
        };
        let (sheet, ranges) = match object {
            ExcelObject::Range(si, r)
                if matches!(name.as_str(), "" | "value" | "value2" | "formula" | "formulalocal" | "formular1c1" | "cells" | "range") =>
            {
                (si, vec![r])
            }
            ExcelObject::Areas(si, ranges) if matches!(name.as_str(), "" | "value" | "value2" | "formula" | "formulalocal" | "formular1c1") => {
                (si, ranges)
            }
            _ => return self.set_member(id, &name, args, value),
        };
        let mut writes = 0usize;
        for range in ranges {
            cap(range)?;
            for at in range.iter() {
                if writes > 0 && writes.is_multiple_of(128) {
                    gridcraft_vbarun::yield_once().await;
                    self.check_budget()?;
                }
                writes = writes.saturating_add(1);
                let item = if let Value::Array(a) = &value {
                    let row = a.bounds.first().map(|b| b.0).ok_or_else(|| invalid("Expected 2-D array"))?;
                    let col = a.bounds.get(1).map(|b| b.0).ok_or_else(|| invalid("Expected 2-D array"))?;
                    let offset =
                        a.offset(&[row.saturating_add((at.row - range.start.row) as i32), col.saturating_add((at.col - range.start.col) as i32)])?;
                    a.values.get(offset).cloned().ok_or_else(|| invalid("Invalid array cell"))?
                } else {
                    value.clone()
                };
                self.write_range(sheet, RangeRef::cell(at), if matches!(name.as_str(), "cells" | "range") { "value" } else { &name }, item)?;
            }
        }
        Ok(())
    }
    fn scalar(&mut self, value: Value) -> Result<Value> {
        if let Value::Object(Some(id)) = value {
            match self.resolve(id)? {
                ExcelObject::Range(sheet, range) => return self.read_range(sheet, range, "value"),
                ExcelObject::Areas(sheet, ranges) => {
                    let range = ranges.first().copied().ok_or_else(|| invalid("Empty range"))?;
                    return self.read_range(sheet, range, "value");
                }
                _ => {}
            }
        }
        Ok(value)
    }
    fn enter_procedure(&mut self, module: &str, procedure: &str) -> Result<()> {
        let (procedure, kind) = if let Some(name) = procedure.strip_suffix(".let") {
            (name, Some(gridcraft_vba::ProcKind::PropertyLet))
        } else if let Some(name) = procedure.strip_suffix(".set") {
            (name, Some(gridcraft_vba::ProcKind::PropertySet))
        } else if let Some(name) = procedure.strip_suffix(".get") {
            (name, Some(gridcraft_vba::ProcKind::PropertyGet))
        } else {
            (procedure, None)
        };
        if let Some(analysis) = &self.analysis
            && let Some((m, p)) = analysis.modules.iter().find(|m| m.name.eq_ignore_ascii_case(module)).and_then(|m| {
                m.procedures
                    .iter()
                    .find(|p| {
                        p.name.eq_ignore_ascii_case(procedure)
                            && match kind {
                                Some(kind) => p.kind == kind,
                                None => !matches!(p.kind, gridcraft_vba::ProcKind::PropertyLet | gridcraft_vba::ProcKind::PropertySet),
                            }
                    })
                    .map(|p| (m, p))
            })
            && p.compatibility.level > 1
        {
            let reason = p.compatibility.reasons.first();
            return Err(Error {
                fatal: true,
                module: m.name.clone(),
                procedure: p.name.clone(),
                line: reason.map_or(p.line as usize, |r| r.line as usize),
                ..Error::runtime(
                    445,
                    format!("Unsupported macro level {}: {}", p.compatibility.level, reason.map_or("Unsupported feature", |r| r.detail.as_str())),
                )
            });
        }
        Ok(())
    }
    fn global(&mut self, name: &str) -> Option<Value> {
        let name = name.to_ascii_lowercase();
        let constant = match name.as_str() {
            "xlup" => Some(-4162),
            "xldown" => Some(-4121),
            "xltoleft" => Some(-4159),
            "xltoright" => Some(-4161),
            "xlcalculationmanual" => Some(-4135),
            "xlcalculationautomatic" => Some(-4105),
            "xlcalculationsemiautomatic" => Some(2),
            "xlnext" => Some(1),
            "xlprevious" => Some(2),
            "xlpastevalues" => Some(-4163),
            "xlcelltypevisible" => Some(12),
            "xlnone" | "xlcolorindexnone" => Some(-4142),
            "xlcolorindexautomatic" => Some(-4105),
            "xlwhole" => Some(1),
            "xlpart" => Some(2),
            "xlbyrows" => Some(1),
            "xlbycolumns" => Some(2),
            "xlformulas" => Some(-4123),
            "xlvalues" => Some(-4163),
            "xlvisible" | "xlsheetvisible" => Some(-1),
            "xlsheethidden" => Some(0),
            "xlsheetveryhidden" => Some(2),
            "xla1" => Some(1),
            "xlr1c1" => Some(-4150),
            "vbyesno" => Some(4),
            "vbokcancel" => Some(1),
            "vbokonly" => Some(0),
            "vbok" => Some(1),
            "vbcancel" => Some(2),
            "vbyes" => Some(6),
            "vbno" => Some(7),
            _ => None,
        };
        if let Some(n) = constant {
            return Some(Value::Long(n));
        }
        match name.as_str() {
            "application" => Some(Value::Object(Some(0))),
            "activeworkbook" | "thisworkbook" | "activesheet" | "activecell" | "selection" | "worksheetfunction" => self.application(&name, &[]).ok(),
            "sheets" | "worksheets" | "cells" | "rows" | "columns" => self.application(&name, &[]).ok(),
            "range" | "union" | "intersect" => self.object(ExcelObject::Global(name)).ok(),
            _ => None,
        }
    }
    fn get_member(&mut self, id: Object, name: &str, args: &[Argument]) -> Result<Value> {
        let name = name.to_ascii_lowercase();
        match self.resolve(id)? {
            ExcelObject::Application => self.application(&name, args),
            ExcelObject::Global(global) => self.application(if name.is_empty() { &global } else { &name }, args),
            ExcelObject::Workbook => match name.as_str() {
                "name" => Ok(Value::String(self.session.doc().map_err(engine)?.display_title())),
                "path" => Ok(Value::String(String::new())),
                "sheets" | "worksheets" => self.application(&name, args),
                "save" => Err(Error::unsupported("Workbook.Save")),
                _ => Err(Error::unsupported(&name)),
            },
            ExcelObject::Sheets => match name.as_str() {
                "" | "item" => self.sheets(args),
                "count" => Ok(Value::Long(self.session.doc().map_err(engine)?.wb.sheets.len() as i32)),
                _ => Err(Error::unsupported(&name)),
            },
            ExcelObject::Sheet(sheet) => self.sheet_member(sheet, &name, args),
            ExcelObject::Range(sheet, range) => self.range_member(sheet, range, &name, args),
            ExcelObject::Areas(sheet, ranges) => {
                if name == "cells" && args.is_empty() {
                    return self.object(ExcelObject::Areas(sheet, ranges));
                }
                if matches!(name.as_str(), "clearcontents" | "clear" | "merge" | "unmerge") {
                    for range in ranges {
                        self.range_member(sheet, range, &name, args)?;
                    }
                    return Ok(Value::Empty);
                }
                if name == "address" {
                    let mut addresses = Vec::new();
                    for range in ranges {
                        addresses.push(self.range_member(sheet, range, "address", args)?.text()?);
                    }
                    return Ok(Value::String(addresses.join(",")));
                }
                if matches!(name.as_str(), "select" | "activate") {
                    return self.run("selection.set", json!({"sheet":sheet,"range":ranges.iter().map(RangeRef::a1).collect::<Vec<_>>().join(",")}));
                }
                if name == "count" {
                    return Ok(Value::Double(ranges.iter().map(RangeRef::count).sum::<u64>() as f64));
                }
                let first = ranges.first().copied().ok_or_else(|| invalid("Empty range"))?;
                self.range_member(sheet, first, &name, args)
            }
            ExcelObject::Dimension(sheet, range, rows) => {
                if name == "count" {
                    Ok(Value::Long(if rows { range.height() } else { range.width() } as i32))
                } else if name.is_empty() || name == "item" {
                    self.range_member(sheet, range, if rows { "rows" } else { "columns" }, args)
                } else {
                    self.range_member(sheet, range, &name, args)
                }
            }
            ExcelObject::Font(sheet, range) => {
                let doc = self.session.doc().map_err(engine)?;
                let sh = doc.wb.sheet(sheet).ok_or_else(|| invalid("Worksheet not found"))?;
                let style = doc.wb.styles.get(sh.style_id(range.start));
                match name.as_str() {
                    "bold" => Ok(Value::Boolean(style.font.bold)),
                    "color" => Ok(Value::Long(color_number(&style.font.color, &doc.wb))),
                    _ => Err(Error::unsupported(&name)),
                }
            }
            ExcelObject::Interior(sheet, range) => {
                let doc = self.session.doc().map_err(engine)?;
                let sh = doc.wb.sheet(sheet).ok_or_else(|| invalid("Worksheet not found"))?;
                let style = doc.wb.styles.get(sh.style_id(range.start));
                match name.as_str() {
                    "color" => Ok(Value::Long(color_number(&style.fill.fg, &doc.wb))),
                    "colorindex" => {
                        let rgb = style.fill.fg.resolve(&doc.wb.theme);
                        let index = (1..=56).find(|i| {
                            palette(*i).ok().and_then(|s| gridcraft_model::Color::from_hex(&s)).and_then(|c| c.resolve(&doc.wb.theme)) == rgb
                        });
                        Ok(Value::Long(index.unwrap_or(-4142) as i32))
                    }
                    _ => Err(Error::unsupported(&name)),
                }
            }
            ExcelObject::Functions => {
                let function =
                    if gridcraft_calc::is_known_function(&name) { name.to_ascii_uppercase() } else { name.replace('_', ".").to_ascii_uppercase() };
                if !gridcraft_calc::is_known_function(&function) {
                    return Err(Error::unsupported(&name));
                }
                if args.len() > 255 {
                    return Err(invalid("Too many WorksheetFunction arguments"));
                }
                let mut supplied: Vec<Option<&Value>> = vec![None; args.len()];
                for (index, argument) in args.iter().enumerate() {
                    let index = if let Some(name) = &argument.name {
                        name.to_ascii_lowercase()
                            .strip_prefix("arg")
                            .and_then(|n| n.parse::<usize>().ok())
                            .filter(|n| (1..=255).contains(n))
                            .map(|n| n - 1)
                            .ok_or_else(|| invalid("WorksheetFunction named arguments use Arg1, Arg2, ..."))?
                    } else {
                        index
                    };
                    if supplied.len() <= index {
                        supplied.resize(index.saturating_add(1), None);
                    }
                    let slot = supplied.get_mut(index).ok_or_else(|| invalid("Invalid function argument"))?;
                    if slot.is_some() {
                        return Err(invalid("Duplicate function argument"));
                    }
                    *slot = Some(&argument.value);
                }
                while supplied.last().is_some_and(|v| v.is_none_or(|v| matches!(v, Value::Missing))) {
                    supplied.pop();
                }
                let mut values = Vec::new();
                for argument in supplied {
                    values.push(if let Some(value) = argument { self.formula_argument(value)? } else { String::new() });
                }
                let doc = self.session.doc().map_err(engine)?;
                from_cell(gridcraft_calc::evaluate(&doc.wb, doc.wb.active_sheet, doc.selection.active, &format!("={function}({})", values.join(","))))
            }
        }
    }
    fn set_member(&mut self, id: Object, name: &str, args: &[Argument], value: Value) -> Result<()> {
        let name = name.to_ascii_lowercase();
        match self.resolve(id)? {
            ExcelObject::Areas(sheet, ranges) => {
                for range in ranges {
                    let Value::Object(Some(target)) = self.range(sheet, range)? else { return Err(invalid("Expected range")) };
                    self.set_member(target, &name, args, value.clone())?;
                }
            }
            ExcelObject::Global(global) => {
                let Value::Object(Some(target)) = self.application(&global, args)? else { return Err(invalid("Expected range")) };
                return self.set_member(target, "value", &[], value);
            }
            ExcelObject::Application => match name.as_str() {
                "screenupdating" | "enableevents" | "displayalerts" | "statusbar" => {
                    self.properties.insert(name, value);
                }
                "calculation" => {
                    let mode = match value.integer()? {
                        -4135 => "manual",
                        -4105 => "automatic",
                        2 => "automaticExceptTables",
                        _ => return Err(invalid("Invalid calculation mode")),
                    };
                    self.run("formulas.calculationOptions", json!({"mode":mode}))?;
                }
                _ => return Err(Error::unsupported(&name)),
            },
            ExcelObject::Sheet(sheet) => match name.as_str() {
                "cells" | "range" => {
                    let Value::Object(Some(target)) = self.sheet_member(sheet, &name, args)? else { return Err(invalid("Expected range")) };
                    return self.set_member(target, "value", &[], value);
                }
                "name" => {
                    self.run("sheet.rename", json!({"sheet":sheet,"name":value.text()?}))?;
                }
                "visible" => {
                    let visibility = value.integer()?;
                    if !matches!(visibility, -1 | 0 | 2) {
                        return Err(invalid("Invalid worksheet visibility"));
                    }
                    self.run(if visibility == -1 { "sheet.unhide" } else { "sheet.hide" }, json!({"sheet":sheet,"veryHidden":visibility==2}))?;
                }
                _ => return Err(Error::unsupported(&name)),
            },
            ExcelObject::Range(sheet, range) | ExcelObject::Dimension(sheet, range, _) => {
                if matches!(name.as_str(), "" | "value" | "value2" | "formula" | "formulalocal" | "formular1c1") {
                    if name.is_empty() && !args.is_empty() {
                        let Value::Object(Some(target)) = self.range_member(sheet, range, "item", args)? else { return Err(invalid("Invalid cell")) };
                        return self.set_member(target, "value", &[], value);
                    }
                    return self.write_range(sheet, range, &name, value);
                }
                cap(range)?;
                if name == "numberformat" {
                    self.run("home.numberFormat", json!({"sheet":sheet,"range":range.a1(),"code":value.text()?}))?;
                    self.changed.extend(range.iter().map(|at| (sheet, at)));
                } else {
                    return Err(Error::unsupported(&name));
                }
            }
            ExcelObject::Font(sheet, range) => {
                cap(range)?;
                match name.as_str() {
                    "bold" => {
                        self.run("home.bold", json!({"sheet":sheet,"range":range.a1(),"on":value.truth()?}))?;
                    }
                    "color" => {
                        self.run("home.fontColor", json!({"sheet":sheet,"range":range.a1(),"color":color_text(value.integer()?)}))?;
                    }
                    _ => return Err(Error::unsupported(&name)),
                }
                self.changed.extend(range.iter().map(|at| (sheet, at)));
            }
            ExcelObject::Interior(sheet, range) => {
                cap(range)?;
                let color = if name == "colorindex" {
                    let index = value.integer()?;
                    if matches!(index, -4142 | -4105) { "none".into() } else { palette(index)? }
                } else if name == "color" {
                    color_text(value.integer()?)
                } else {
                    return Err(Error::unsupported(&name));
                };
                self.run("home.fillColor", json!({"sheet":sheet,"range":range.a1(),"color":color}))?;
                self.changed.extend(range.iter().map(|at| (sheet, at)));
            }
            _ => return Err(Error::unsupported(&name)),
        }
        Ok(())
    }
    fn enumerate(&mut self, id: Object) -> Result<Vec<Value>> {
        match self.resolve(id)? {
            ExcelObject::Range(sheet, range) | ExcelObject::Dimension(sheet, range, _) => {
                cap(range)?;
                range.iter().map(|at| self.range(sheet, RangeRef::cell(at))).collect()
            }
            ExcelObject::Areas(sheet, ranges) => {
                let mut values = Vec::new();
                for range in ranges {
                    cap(range)?;
                    for at in range.iter() {
                        values.push(self.range(sheet, RangeRef::cell(at))?);
                    }
                }
                Ok(values)
            }
            ExcelObject::Sheets => (0..self.session.doc().map_err(engine)?.wb.sheets.len()).map(|si| self.object(ExcelObject::Sheet(si))).collect(),
            _ => Err(Error::unsupported("For Each")),
        }
    }
    fn msg_box(&mut self, args: &[Argument]) -> Result<Value> {
        self.notice("macro.msgBox", args, "MsgBox used its headless default");
        let buttons = number(args, 1, "Buttons", 0)? & 7;
        Ok(Value::Long(if buttons == 4 {
            7
        } else if matches!(buttons, 1 | 3 | 5) {
            2
        } else {
            1
        }))
    }
    fn input_box(&mut self, args: &[Argument]) -> Result<Value> {
        self.notice("macro.inputBox", args, "InputBox used its headless default");
        Ok(Value::String(arg(args, 2, "Default").map(Value::text).transpose()?.unwrap_or_default()))
    }
    fn log(&mut self, message: &str) {
        let message: String = message.chars().take(4096).collect();
        if self.message_bytes.saturating_add(message.len()) > 64 * 1024 {
            return;
        }
        if self.messages.len() < 1000 && self.edit_bytes.saturating_add(message.len()) <= self.max_edit_bytes {
            self.edit_bytes = self.edit_bytes.saturating_add(message.len());
            self.message_bytes = self.message_bytes.saturating_add(message.len());
            self.messages.push(message);
        }
    }
}
impl ExcelHost {
    fn formula_argument(&self, value: &Value) -> Result<String> {
        match value {
            Value::Object(Some(id)) => {
                let (sheet, ranges) = self.parts(&Value::Object(Some(*id)))?;
                let doc = self.session.doc().map_err(engine)?;
                let name = &doc.wb.sheet(sheet).ok_or_else(|| invalid("Worksheet not found"))?.name;
                let references = ranges.iter().map(|range| format!("{}!{}", gridcraft_formula::quote_sheet(name), range.a1())).collect::<Vec<_>>();
                Ok(if references.len() == 1 { references.first().cloned().unwrap_or_default() } else { format!("({})", references.join(",")) })
            }
            Value::String(s) => Ok(format!("\"{}\"", s.replace('"', "\"\""))),
            Value::Boolean(b) => Ok(if *b { "TRUE" } else { "FALSE" }.into()),
            Value::Empty => Ok("0".into()),
            Value::Missing => Ok(String::new()),
            Value::Array(a) => {
                if a.bounds.len() != 2 {
                    return Err(invalid("Expected a two-dimensional array"));
                }
                let width =
                    a.bounds.get(1).map(|(a, b)| i64::from(*b) - i64::from(*a) + 1).filter(|n| *n > 0).ok_or_else(|| invalid("Invalid array"))?
                        as usize;
                let rows = a
                    .values
                    .chunks(width)
                    .map(|row| row.iter().map(|v| self.formula_argument(v)).collect::<Result<Vec<_>>>().map(|r| r.join(",")))
                    .collect::<Result<Vec<_>>>()?;
                Ok(format!("{{{}}}", rows.join(";")))
            }
            _ => value.text(),
        }
    }
}
fn from_cell(value: gridcraft_core::Value) -> Result<Value> {
    from_cell_depth(value, 0)
}
fn from_cell_depth(value: gridcraft_core::Value, depth: usize) -> Result<Value> {
    if depth > 32 {
        return Err(invalid("VBA sandbox limit: value nesting"));
    }
    use gridcraft_core::Value as CellValue;
    Ok(match value {
        CellValue::Empty => Value::Empty,
        CellValue::Number(n) => Value::Double(n),
        CellValue::Text(s) => {
            if s.len() > 1_000_000 {
                return Err(invalid("VBA sandbox limit: string length"));
            }
            Value::String(s.to_string())
        }
        CellValue::Bool(b) => Value::Boolean(b),
        CellValue::Error(e) => return Err(invalid(e.as_str())),
        CellValue::Array(a) => Value::Array(Array {
            bounds: vec![(1, a.rows as i32), (1, a.cols as i32)],
            values: a.data.iter().cloned().map(|v| from_cell_depth(v, depth + 1)).collect::<Result<Vec<_>>>()?,
            element_type: "variant".into(),
            dynamic: false,
        }),
    })
}
fn color_text(color: i64) -> String {
    format!("#{:02X}{:02X}{:02X}", color & 255, (color >> 8) & 255, (color >> 16) & 255)
}
fn color_number(color: &gridcraft_model::Color, wb: &gridcraft_model::Workbook) -> i32 {
    let [r, g, b] = color.resolve(&wb.theme).unwrap_or([0, 0, 0]);
    i32::from(r) | (i32::from(g) << 8) | (i32::from(b) << 16)
}
fn palette(index: i64) -> Result<String> {
    const COLORS: [u32; 56] = [
        0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000, 0x000080, 0x808000, 0x800080, 0x008080,
        0xC0C0C0, 0x808080, 0x9999FF, 0x993366, 0xFFFFCC, 0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF,
        0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF, 0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC,
        0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600, 0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399, 0x333333,
    ];
    let value = usize::try_from(index.saturating_sub(1)).ok().and_then(|i| COLORS.get(i)).ok_or_else(|| invalid("Invalid color index"))?;
    Ok(format!("#{value:06X}"))
}
fn r1c1_to_a1(input: &str, at: CellRef) -> Result<String> {
    fn component(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, base: u32) -> Result<(i64, bool)> {
        let relative = chars.peek() == Some(&'[');
        if relative {
            chars.next();
        }
        let mut digits = String::new();
        while chars.peek().is_some_and(|c| c.is_ascii_digit() || (relative && matches!(c, '+' | '-'))) {
            if let Some(c) = chars.next() {
                digits.push(c);
            }
        }
        if relative && chars.next() != Some(']') {
            return Err(invalid("Invalid R1C1 reference"));
        }
        if digits.is_empty() {
            return Ok((i64::from(base), true));
        }
        let n = digits.parse::<i64>().map_err(|_| invalid("Invalid R1C1 reference"))?;
        Ok((if relative { i64::from(base).saturating_add(n) } else { n.saturating_sub(1) }, relative))
    }
    let mut output = String::new();
    let mut chars = input.chars().peekable();
    let mut quoted = false;
    let mut sheet_quoted = false;
    let mut brackets = 0usize;
    while let Some(ch) = chars.next() {
        if ch == '"' && !sheet_quoted {
            quoted = !quoted;
            output.push(ch);
            continue;
        }
        if ch == '\'' && !quoted {
            sheet_quoted = !sheet_quoted;
            output.push(ch);
            continue;
        }
        if !quoted && !sheet_quoted {
            if ch == '[' {
                brackets = brackets.saturating_add(1);
            }
            if ch == ']' {
                brackets = brackets.saturating_sub(1);
            }
        }
        let boundary = output.chars().next_back().is_none_or(|c| !c.is_alphanumeric() && !matches!(c, '_' | '.' | '$'));
        let candidate = matches!(ch, 'R' | 'r' | 'C' | 'c')
            && chars.peek().is_none_or(|c| {
                c.is_ascii_digit() || *c == '[' || (!c.is_alphanumeric() && *c != '_') || (matches!(ch, 'R' | 'r') && matches!(c, 'C' | 'c'))
            });
        if quoted || sheet_quoted || brackets > 0 || !boundary || !candidate {
            output.push(ch);
            continue;
        }
        let mut trial = chars.clone();
        let row = if matches!(ch, 'R' | 'r') { Some(component(&mut trial, at.row)?) } else { None };
        let col = if matches!(ch, 'C' | 'c') {
            Some(component(&mut trial, at.col)?)
        } else if matches!(trial.peek(), Some('C' | 'c')) {
            trial.next();
            Some(component(&mut trial, at.col)?)
        } else {
            None
        };
        if trial.peek().is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '!')) {
            output.push(ch);
            continue;
        }
        let row_number = row.map_or(i64::from(at.row), |v| v.0);
        let col_number = col.map_or(i64::from(at.col), |v| v.0);
        let cell = bounded(row_number, col_number)?;
        let row_text = format!("{}{}", if row.is_some_and(|v| !v.1) { "$" } else { "" }, cell.row + 1);
        let col_text = format!("{}{}", if col.is_some_and(|v| !v.1) { "$" } else { "" }, gridcraft_core::col_to_letters(cell.col));
        let in_range = output.ends_with(':') || trial.peek() == Some(&':');
        match (row, col) {
            (Some(_), Some(_)) => output.push_str(&format!("{col_text}{row_text}")),
            (Some(_), None) => output.push_str(&if in_range { row_text.clone() } else { format!("{row_text}:{row_text}") }),
            (None, Some(_)) => output.push_str(&if in_range { col_text.clone() } else { format!("{col_text}:{col_text}") }),
            _ => return Err(invalid("Invalid R1C1 reference")),
        }
        chars = trial;
    }
    Ok(output)
}

fn rectangles(cells: std::collections::BTreeSet<CellRef>) -> Vec<RangeRef> {
    let mut rows: Vec<RangeRef> = Vec::new();
    for at in cells {
        if let Some(last) = rows.last_mut()
            && last.start.row == at.row
            && last.end.col.saturating_add(1) == at.col
        {
            last.end.col = at.col;
        } else {
            rows.push(RangeRef::cell(at));
        }
    }
    let mut ranges: Vec<RangeRef> = Vec::new();
    let mut spans: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for row in rows {
        let span = (row.start.col, row.end.col);
        if let Some(previous) = spans.get(&span).and_then(|index| ranges.get_mut(*index))
            && previous.end.row.saturating_add(1) == row.start.row
        {
            previous.end.row = row.end.row;
        } else {
            spans.insert(span, ranges.len());
            ranges.push(row);
        }
    }
    ranges
}
