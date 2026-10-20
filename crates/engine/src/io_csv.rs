//! Delimited file exports use displayed values and neutralise executable text.

use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Sheet, Workbook};

use crate::{EngineError, Result};

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

pub(crate) fn write(wb: &Workbook, delimiter: u8, bom: bool) -> Result<Vec<u8>> {
    let mut bytes = match wb.sheet(wb.active_sheet) {
        Some(sh) => {
            let limit = gridcraft_xlsx::MAX_CSV_BYTES.saturating_sub(if bom { UTF8_BOM.len() } else { 0 });
            gridcraft_xlsx::write_csv_limited(sh, delimiter, limit, |c| cell_text(wb, sh, c, delimiter))
                .map_err(|e| EngineError::Other(e.to_string()))?
        }
        None => Vec::new(),
    };
    if bom {
        bytes.try_reserve(UTF8_BOM.len()).map_err(|_| EngineError::Other(crate::io::FILE_TOO_LARGE.into()))?;
        drop(bytes.splice(..0, UTF8_BOM));
    }
    Ok(bytes)
}

fn cell_text(wb: &Workbook, sh: &Sheet, c: CellRef, delimiter: u8) -> String {
    if delimiter == b'\t' && sh.merges.iter().any(|m| m.contains(c) && m.start != c) {
        return String::new();
    }
    // Delimited files have no column width, so General must preserve numeric precision.
    let mut text = if wb.styles.get(sh.style_id(c)).num_fmt.as_str().eq_ignore_ascii_case("General")
        && let Value::Number(number) = sh.value(c)
        && number.is_finite()
    {
        number.to_string()
    } else {
        crate::display::value_text(wb, sh, c)
    };
    let is_text = sh.value_ref(c).is_some_and(|value| value.scalar().is_text());
    if is_text && matches!(text.as_bytes().first(), Some(b'=' | b'+' | b'-' | b'@' | b'\t' | b'\r')) {
        text.insert(0, '\'');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{open_bytes, save_bytes};
    use gridcraft_core::{RangeRef, parse::parse_input};
    use gridcraft_model::{Cell, Formula, NumFmt, StyleId};

    #[test]
    fn delimited_files_neutralise_each_dangerous_text_prefix() {
        for input in ["=1+1", "+SUM(A1)", "-cmd", "@x", "\tx", "\rx", "-5", "+3"] {
            let mut wb = Workbook::new();
            wb.sheet_mut(0).unwrap().set_value(CellRef::new(0, 0), Value::text(input));
            for path in ["out.csv", "out.tsv", "out.tab", "out.txt"] {
                let bytes = save_bytes(&wb, path).unwrap();
                let (back, _) = open_bytes(path, &bytes).unwrap();
                assert_eq!(back.sheet(0).unwrap().value(CellRef::new(0, 0)), Value::text(format!("'{input}")), "{path}: {input:?}");
                assert!(back.sheet(0).unwrap().cell(CellRef::new(0, 0)).unwrap().formula.is_none());
                assert_eq!(save_bytes(&back, path).unwrap(), bytes, "the prefix is applied once");
            }
        }
    }

    #[test]
    fn delimited_files_keep_typed_numbers_dates_and_formula_results() {
        let mut wb = Workbook::new();
        let signed = wb.styles.derive(StyleId::DEFAULT, |style| style.num_fmt = NumFmt::new("+0;-0;0"));
        let date = wb.styles.derive(StyleId::DEFAULT, |style| style.num_fmt = NumFmt::new("yyyy-mm-dd"));
        let sys = wb.date_system;
        let sh = wb.sheet_mut(0).unwrap();
        for (col, input) in [(0, "-5"), (1, "+3")] {
            let parsed = parse_input(input, sys);
            assert!(parsed.value.is_number());
            sh.set_value(CellRef::new(0, col), parsed.value);
        }
        sh.set_cell(CellRef::new(0, 2), Cell { value: Value::Number(3.0), style: signed, ..Default::default() });
        sh.set_cell(CellRef::new(0, 3), Cell { value: Value::Number(46023.0), style: date, ..Default::default() });
        sh.set_cell(CellRef::new(0, 4), Cell { value: Value::Number(-5.0), ..Cell::formula(Formula::new("-2-3")) });
        sh.set_cell(CellRef::new(0, 5), Cell { value: Value::text("=1+1"), ..Cell::formula(Formula::new("\"=1+1\"")) });
        sh.show_formulas = true;
        for (path, separator, bom) in [("out.csv", ",", "\u{feff}"), ("out.tsv", "\t", "")] {
            let output = String::from_utf8(save_bytes(&wb, path).unwrap()).unwrap();
            assert_eq!(output, format!("{bom}-5{separator}3{separator}+3{separator}2026-01-01{separator}-5{separator}'=1+1\r\n"));
        }
    }

    #[test]
    fn calculated_text_and_spill_results_are_neutralised() {
        let mut session = crate::Session::new();
        session.new_workbook();
        session.execute("cell.set", serde_json::json!({"cell": "A1", "input": "={\"=1+1\",\"@x\",-5}"})).unwrap();
        let wb = &session.active().unwrap().wb;
        assert_eq!(save_bytes(wb, "out.csv").unwrap(), b"\xEF\xBB\xBF'=1+1,'@x,-5\r\n");
        assert_eq!(save_bytes(wb, "out.tsv").unwrap(), b"'=1+1\t'@x\t-5\r\n");
    }

    #[test]
    fn safe_text_is_escaped_before_delimited_quoting() {
        let inputs = ["=SUM(1,2)", "+\"quoted\"\nline", "\rnext", "\tnext", "plain", "", "'=literal"];
        let mut wb = Workbook::new();
        for (col, input) in inputs.iter().enumerate() {
            wb.sheet_mut(0).unwrap().set_value(CellRef::new(0, col as u32), Value::text(*input));
        }
        let csv = String::from_utf8(save_bytes(&wb, "out.csv").unwrap()).unwrap();
        assert_eq!(csv, "\u{feff}\"'=SUM(1,2)\",\"'+\"\"quoted\"\"\nline\",\"'\rnext\",\"'\tnext\",plain,,'=literal\r\n");
        for path in ["out.csv", "out.tsv"] {
            let bytes = save_bytes(&wb, path).unwrap();
            let (back, _) = open_bytes(path, &bytes).unwrap();
            for (col, expected) in ["'=SUM(1,2)", "'+\"quoted\"\nline", "'\rnext", "'\tnext", "plain", "", "'=literal"].iter().enumerate() {
                assert_eq!(back.sheet(0).unwrap().value(CellRef::new(0, col as u32)).display(), *expected);
            }
        }
    }

    #[test]
    fn csv_utf8_bom_round_trips_vietnamese_and_empty_workbooks() {
        let mut wb = Workbook::new();
        assert_eq!(save_bytes(&wb, "empty.csv").unwrap(), UTF8_BOM);
        assert!(save_bytes(&wb, "empty.tsv").unwrap().is_empty());
        wb.sheet_mut(0).unwrap().set_value(CellRef::new(0, 0), Value::text("Tiếng Việt"));
        let bytes = save_bytes(&wb, "out.csv").unwrap();
        assert!(bytes.starts_with(&UTF8_BOM));
        assert_eq!(bytes, "\u{feff}Tiếng Việt\r\n".as_bytes());
        let (back, _) = open_bytes("out.csv", &bytes).unwrap();
        assert_eq!(back.sheet(0).unwrap().value(CellRef::new(0, 0)), Value::text("Tiếng Việt"));
    }

    #[test]
    fn csv_keeps_imported_delimiters_with_safe_text_and_bom() {
        for delimiter in [b',', b';', b'\t'] {
            let mut wb = Workbook::new();
            wb.csv_delimiter = Some(delimiter);
            let sh = wb.sheet_mut(0).unwrap();
            sh.set_value(CellRef::new(0, 0), Value::text("=1+1"));
            sh.set_value(CellRef::new(0, 1), Value::Number(-5.0));
            let bytes = save_bytes(&wb, "out.csv").unwrap();
            assert_eq!(bytes, format!("\u{feff}'=1+1{}-5\r\n", delimiter as char).as_bytes());
            let (back, _) = open_bytes("out.csv", &bytes).unwrap();
            assert_eq!(back.csv_delimiter, Some(delimiter));
            assert_eq!(back.sheet(0).unwrap().value(CellRef::new(0, 0)), Value::text("'=1+1"));
        }
    }

    #[test]
    fn file_safety_does_not_change_clipboard_text_or_source_values() {
        let mut wb = Workbook::new();
        let cell = CellRef::new(0, 0);
        wb.sheet_mut(0).unwrap().set_value(cell, Value::text("=1+1"));
        let range = RangeRef::new(cell, cell);
        assert_eq!(crate::display::range_text(&wb, 0, range), "=1+1\n");
        assert_eq!(save_bytes(&wb, "out.tsv").unwrap(), b"'=1+1\r\n");
        assert_eq!(crate::display::range_text(&wb, 0, range), "=1+1\n");
        assert_eq!(wb.sheet(0).unwrap().value(cell), Value::text("=1+1"));
    }
}
