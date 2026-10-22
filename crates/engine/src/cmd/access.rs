//! Access table import shared by native, WASM, CLI and MCP callers.
use std::sync::Arc;

use gridcraft_core::{CellRef, MAX_COLS, MAX_ROWS, Value};
use gridcraft_mdb::{ColumnType, Database, Value as AccessValue};
use gridcraft_model::{Cell, NumFmt, Sheet, Style};
use serde_json::{Value as Json, json};

use super::*;

const ID: &str = "data.importAccess";

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "data.importAccess",
        "Import Access Table",
        [],
        None,
        "{data: base64, table?: string, sheet?: string} → {tables: [{name, columns: [{name, type}], rowCount}]} or {sheet, rows, columns}; no table lists only",
        has_doc,
        import_access
    )]
}
fn import_access(s: &mut Session, p: &Json) -> Result<Json> {
    let data = str_param(p, "data").ok_or_else(|| bad(ID, "data must be a base64 string"))?;
    if data.len() > gridcraft_mdb::MAX_BYTES.div_ceil(3) * 4 {
        return Err(bad(ID, "Access data exceeds the 128 MiB input limit"));
    }
    for key in ["table", "sheet"] {
        if p.get(key).is_some_and(|v| !v.is_string()) {
            return Err(bad(ID, format!("{key} must be a string")));
        }
    }
    let bytes = crate::io::base64_decode(data).ok_or_else(|| bad(ID, "invalid base64"))?;
    let db = Database::open(&bytes).map_err(|e| bad(ID, e.to_string()))?;
    let Some(name) = str_param(p, "table") else {
        let tables: Vec<_> = db
            .tables()
            .iter()
            .map(|table| {
                let columns: Vec<_> = table.columns.iter().map(|c| json!({"name": c.name, "type": type_name(c.kind)})).collect();
                json!({"name": table.name, "columns": columns, "rowCount": table.row_count})
            })
            .collect();
        return Ok(json!({"tables": tables}));
    };
    let table = db.table(name).map_err(|e| bad(ID, e.to_string()))?;
    let info = db.tables().iter().find(|t| t.name.eq_ignore_ascii_case(name)).ok_or_else(|| bad(ID, "table not found"))?;
    if info.row_count >= MAX_ROWS as usize || table.columns().len() > MAX_COLS as usize {
        return Err(bad(ID, "Access table is too large for a sheet (1,048,576 rows including headers; 16,384 columns)"));
    }
    let wb = &s.doc()?.wb;
    let sheet_name = str_param(p, "sheet").map(str::to_owned).unwrap_or_else(|| wb.next_sheet_name());
    let existing = wb.sheet_index(&sheet_name);
    wb.check_sheet_name(&sheet_name, existing).map_err(|e| bad(ID, e))?;
    if existing.is_none() && wb.protected_structure {
        return Err(bad(ID, "The workbook is protected and cannot be changed."));
    }
    let date_system = wb.date_system;
    // Decode before committing so a malformed row cannot leave a partially imported table.
    let mut rows = Vec::new();
    for row in table.rows() {
        let row = row.map_err(|e| bad(ID, e.to_string()))?;
        rows.push(row.into_iter().map(|value| cell_value(value, date_system)).collect::<Result<Vec<_>>>()?);
    }
    let row_count = rows.len();
    let columns = table.columns().len();
    edit(s, |cx| {
        let index = if let Some(index) = existing {
            index
        } else {
            let index = cx.wb.sheets.len();
            cx.wb.sheets.push(Arc::new(Sheet::new(sheet_name.clone())));
            index
        };
        let mut header = Style::default();
        header.font.bold = true;
        let header = cx.wb.styles.intern(header);
        let date = Style { num_fmt: NumFmt::new("yyyy-mm-dd hh:mm:ss"), ..Style::default() };
        let date = cx.wb.styles.intern(date);
        let sheet = cx.wb.sheet_mut(index).ok_or_else(|| bad(ID, "sheet not found"))?;
        for (c, column) in table.columns().iter().enumerate() {
            cx.changed.push((index, CellRef::new(0, c as u32)));
            sheet.set_cell(CellRef::new(0, c as u32), Cell { value: Value::Text(column.name.clone().into()), formula: None, style: header });
        }
        for (r, row) in rows.into_iter().enumerate() {
            for (c, (value, is_date)) in row.into_iter().enumerate() {
                cx.changed.push((index, CellRef::new(r as u32 + 1, c as u32)));
                sheet.set_cell(
                    CellRef::new(r as u32 + 1, c as u32),
                    Cell { value, formula: None, style: if is_date { date } else { Default::default() } },
                );
            }
        }
        cx.wb.active_sheet = index;
        *cx.sel = crate::Selection::default();
        cx.structural = existing.is_none();
        Ok(())
    })?;
    Ok(json!({"sheet": sheet_name, "rows": row_count, "columns": columns}))
}
fn type_name(kind: ColumnType) -> &'static str {
    match kind {
        ColumnType::Bool => "bool",
        ColumnType::Byte => "byte",
        ColumnType::Int => "int",
        ColumnType::Long => "long",
        ColumnType::Single => "single",
        ColumnType::Double => "double",
        ColumnType::Currency => "currency",
        ColumnType::DateTime => "dateTime",
        ColumnType::Text => "text",
        ColumnType::Memo => "memo",
        ColumnType::Numeric => "numeric",
        ColumnType::Binary => "binary",
    }
}
fn cell_value(value: AccessValue, date_system: gridcraft_core::DateSystem) -> Result<(Value, bool)> {
    let mut date = false;
    let value = match value {
        AccessValue::Null => Value::Empty,
        AccessValue::Bool(b) => Value::Bool(b),
        AccessValue::Byte(n) => Value::Number(f64::from(n)),
        AccessValue::Int(n) => Value::Number(f64::from(n)),
        AccessValue::Long(n) => Value::Number(f64::from(n)),
        AccessValue::Single(n) => Value::Number(f64::from(n)),
        AccessValue::Double(n) => Value::Number(n),
        AccessValue::DateTime(n) => {
            date = true;
            // OLE dates use the absolute fractional part even before the epoch; Excel also has its fictitious leap day.
            let n = n.trunc() + n.fract().abs();
            Value::Number(gridcraft_core::date::serial_in_system(date_system, if n < 61.0 { n - 1.0 } else { n }))
        }
        AccessValue::Currency(n) | AccessValue::Numeric(n) => Value::Number(n.parse().map_err(|_| bad(ID, "invalid decimal value"))?),
        AccessValue::Text(text) => Value::Text(text.into()),
        AccessValue::Binary(_) => return Err(bad(ID, "Binary/OLE columns cannot be imported into spreadsheet cells")),
    };
    if matches!(&value, Value::Number(n) if !n.is_finite()) {
        return Err(bad(ID, "Access table contains a non-finite number"));
    }
    Ok((value, date))
}
