//! Typed constants in a regional notation (`cell.set`'s `region`) and the ₫ formats; what is
//! stored, and written to XLSX, stays invariant.
use std::io::Read;

use gridcraft_core::{CellRef, Region, Value, region::VND_ACCOUNTING};
use gridcraft_engine::{Session, display};
use serde_json::json;

fn workbook() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

fn set(s: &mut Session, cell: &str, input: &str, region: Option<&str>) {
    let mut p = json!({"cell": cell, "input": input});
    if let Some(r) = region {
        p["region"] = json!(r);
    }
    s.execute("cell.set", p).unwrap();
}

fn at(s: &Session, a1: &str) -> (Value, String) {
    let d = s.active().unwrap();
    let sh = d.wb.active().unwrap();
    let c = CellRef::parse(a1).unwrap();
    (sh.value(c), d.wb.styles.get(sh.style_id(c)).num_fmt.as_str().to_string())
}

fn zip_part(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut out).unwrap();
    out
}

#[test]
fn cell_set_reads_constants_in_the_requested_region() {
    let mut s = workbook();
    for (cell, input, value, format) in [
        ("A1", "1.234,5", Value::Number(1234.5), "#,##0.00"),
        ("A2", "1234,5", Value::Number(1234.5), "General"),
        ("A3", "1.234", Value::Number(1234.0), "#,##0"),
        ("A4", "-1.234", Value::Number(-1234.0), "#,##0"),
        ("A5", "50%", Value::Number(0.5), "0%"),
        ("A6", "1.000.000 ₫", Value::Number(1_000_000.0), "#,##0 [$₫-42A]"),
        ("A7", "1.000.000đ", Value::Number(1_000_000.0), "#,##0 [$₫-42A]"),
        ("A8", "19/10/2026", Value::Number(46314.0), "m/d/yyyy"),
        ("A9", "1.23", Value::text("1.23"), "General"),
    ] {
        set(&mut s, cell, input, Some("vi-VN"));
        assert_eq!(at(&s, cell), (value, format.to_string()), "{input}");
    }
    // Without `region` the API stays en-US.
    set(&mut s, "B1", "1.234", None);
    assert_eq!(at(&s, "B1").0, Value::Number(1.234));
    set(&mut s, "B2", "1.000.000 ₫", None);
    assert_eq!(at(&s, "B2").0, Value::text("1.000.000 ₫"));
    // Formulas are always canonical, whatever the region.
    set(&mut s, "B3", "=SUM(1.5,2)", Some("vi-VN"));
    assert_eq!(at(&s, "B3").0, Value::Number(3.5));
    assert!(s.execute("cell.set", json!({"cell": "B4", "input": "=SUM(1;2)", "region": "vi-VN"})).is_err());
    assert!(s.execute("cell.set", json!({"cell": "B5", "input": "1", "region": "xx-YY"})).is_err());
    s.execute("selection.set", json!({"range": "C1:C3"})).unwrap();
    s.execute("range.fill", json!({"input": "2.500 ₫", "region": "vi-VN"})).unwrap();
    assert_eq!(at(&s, "C3"), (Value::Number(2500.0), "#,##0 [$₫-42A]".into()));
}

#[test]
fn accounting_button_takes_the_dong() {
    let mut s = workbook();
    set(&mut s, "A1", "1000000", None);
    s.execute("home.accounting", json!({"range": "A1", "symbol": "₫"})).unwrap();
    assert_eq!(at(&s, "A1").1, VND_ACCOUNTING);
    s.execute("home.accounting", json!({"range": "A2"})).unwrap();
    assert!(at(&s, "A2").1.contains("\"$\""), "no symbol keeps the dollar");
    assert!(s.execute("home.accounting", json!({"range": "A3", "symbol": "₩"})).is_err());
    let wb = &s.active().unwrap().wb;
    assert_eq!(display::format(&Value::Number(1_000_000.0), VND_ACCOUNTING, wb).text.trim(), "1,000,000 ₫");
    assert_eq!(display::format(&Value::Number(-2500.0), VND_ACCOUNTING, wb).text.trim(), "-2,500 ₫");
    assert_eq!(display::format(&Value::Number(1500.5), "#,##0.00 [$₫-42A]", wb).text, "1,500.50 ₫");
    assert_eq!(Region::ViVn.currency_symbol(), "₫");
}

#[test]
fn edit_text_reads_back_in_the_region() {
    let mut s = workbook();
    set(&mut s, "A1", "1234.5", None);
    set(&mut s, "A2", "19/10/2026", Some("vi-VN"));
    set(&mut s, "A3", "19/10/2026 18:30", Some("vi-VN"));
    set(&mut s, "A4", "3/4/2026", Some("vi-VN"));
    let d = s.active().unwrap();
    let sh = d.wb.active().unwrap();
    let vi = |a1| display::edit_text_in(&d.wb, sh, CellRef::parse(a1).unwrap(), Region::ViVn);
    assert_eq!(vi("A1"), "1234,5");
    assert_eq!(vi("A2"), "19/10/2026");
    assert_eq!(vi("A3"), "19/10/2026 18:30:00");
    assert_eq!(vi("A4"), "03/04/2026", "3 April, never 4 March");
    assert_eq!(display::edit_text(&d.wb, sh, CellRef::parse("A1").unwrap()), "1234.5");
    assert_eq!(display::edit_text(&d.wb, sh, CellRef::parse("A4").unwrap()), "4/3/2026");
}

#[test]
fn xlsx_keeps_invariant_values_formulas_and_codes() {
    let mut s = workbook();
    set(&mut s, "A1", "1.234,5", Some("vi-VN"));
    set(&mut s, "A2", "1.000.000 ₫", Some("vi-VN"));
    set(&mut s, "A3", "19/10/2026", Some("vi-VN"));
    // What the cell editor sends for `=SUM(A1;1,5)` typed in vi-VN.
    set(&mut s, "A4", "=SUM(A1,1.5)", None);
    s.execute("home.accounting", json!({"range": "A5", "symbol": "₫"})).unwrap();
    set(&mut s, "A5", "2500", None);
    let before: Vec<(Value, String)> = ["A1", "A2", "A3", "A4", "A5"].iter().map(|c| at(&s, c)).collect();
    let saved = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap();
    let bytes = gridcraft_engine::io::base64_decode(saved["base64"].as_str().unwrap()).unwrap();
    let sheet = zip_part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<f>SUM(A1,1.5)</f>"), "{sheet}");
    assert!(sheet.contains("<v>1234.5</v>") && sheet.contains("<v>1000000</v>") && sheet.contains("<v>46314</v>"), "{sheet}");
    let styles = zip_part(&bytes, "xl/styles.xml");
    assert!(styles.contains("formatCode=\"#,##0 [$₫-42A]\""), "{styles}");
    assert!(styles.contains("formatCode=\"_-* #,##0 [$₫-42A]_-;-* #,##0 [$₫-42A]_-;_-* &quot;-&quot; [$₫-42A]_-;_-@_-\""), "{styles}");
    s.execute("file.open", json!({"name": "vi.xlsx", "base64": saved["base64"]})).unwrap();
    let after: Vec<(Value, String)> = ["A1", "A2", "A3", "A4", "A5"].iter().map(|c| at(&s, c)).collect();
    assert_eq!(after, before);
    let formula = s.active().unwrap().wb.active().unwrap().cell(CellRef::parse("A4").unwrap()).unwrap().formula.clone().unwrap();
    assert_eq!(&*formula.text, "SUM(A1,1.5)");
}
