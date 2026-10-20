//! External clipboard modes, regional formulas and their serialized XLSX representation.

use std::io::{Cursor, Read};

use gridcraft_core::{CellRef, RangeRef, Region, Value};
use gridcraft_engine::{Session, UiRequest};
use gridcraft_model::{Color, Style};
use serde_json::json;

const HTML: &str = r##"<table><tr><td colspan="2" style="font-weight:bold;background:#abcdef">Heading</td></tr>
    <tr><td x:num="1234.5" style='mso-number-format:"0.0000"'>1,234.5000</td><td x:str>0012</td></tr></table>"##;

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

fn cell(s: &Session, at: &str) -> (Value, Style) {
    let wb = &s.active().unwrap().wb;
    let sh = wb.active().unwrap();
    let at = CellRef::parse(at).unwrap();
    (sh.value(at), wb.styles.get(sh.style_id(at)).clone())
}

#[test]
fn external_match_destination_and_values_keep_destination_styles() {
    for what in ["matchDestination", "values"] {
        let mut s = session();
        s.execute("home.italic", json!({"range":"D4:E5"})).unwrap();
        s.execute("home.numberFormat", json!({"range":"D4:E5", "code":"0%"})).unwrap();
        let before = cell(&s, "D4").1;
        s.execute("edit.pasteSpecial", json!({"source":"external", "what":what, "at":"D4", "html":HTML, "text":"fallback"})).unwrap();
        assert_eq!(cell(&s, "D4"), (Value::text("Heading"), before.clone()));
        assert_eq!(cell(&s, "D5"), (Value::number(1234.5), before.clone()));
        assert_eq!(cell(&s, "E5"), (Value::text("0012"), before));
        assert!(s.active().unwrap().wb.active().unwrap().merges.is_empty());
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(cell(&s, "D4").0, Value::Empty);
        assert!(cell(&s, "D4").1.font.italic);
    }
}

#[test]
fn external_keep_source_preserves_html_and_does_not_use_internal_copy() {
    let mut s = session();
    s.execute("cell.set", json!({"input":"internal"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("edit.pasteSpecial", json!({"source":"external", "what":"keepSource", "at":"C3", "html":HTML})).unwrap();
    let (value, style) = cell(&s, "C3");
    assert_eq!(value, Value::text("Heading"));
    assert!(style.font.bold);
    assert_eq!(style.fill.fg, Color::Rgb(0xabcdef));
    assert_eq!(cell(&s, "C4").1.num_fmt.as_str(), "0.0000");
    assert_eq!(s.active().unwrap().wb.active().unwrap().merges, [RangeRef::parse("C3:D3").unwrap()]);
    assert_eq!(s.clipboard.as_ref().unwrap().text.trim(), "internal");
}

#[test]
fn external_values_from_plain_text_omit_automatic_formats_and_formulas() {
    let mut s = session();
    s.execute("edit.pasteSpecial", json!({"source":"external", "what":"values", "text":"12%\t=1+2"})).unwrap();
    assert_eq!(cell(&s, "A1"), (Value::number(0.12), Style::default()));
    assert_eq!(cell(&s, "B1"), (Value::number(3.0), Style::default()));
    assert!(s.active().unwrap().wb.active().unwrap().cell(CellRef::parse("B1").unwrap()).unwrap().formula.is_none());
}

#[test]
fn external_values_preserve_destination_merges() {
    let mut s = session();
    s.execute("home.mergeCells", json!({"range":"A1:B1"})).unwrap();
    s.execute("edit.pasteSpecial", json!({"source":"external", "what":"matchDestination", "html":HTML})).unwrap();
    assert_eq!(s.active().unwrap().wb.active().unwrap().merges, [RangeRef::parse("A1:B1").unwrap()]);
}

#[test]
fn external_special_rejects_missing_payload_and_unsupported_operations() {
    let mut s = session();
    for params in [
        json!({"source":"external", "what":"values"}),
        json!({"source":"external", "what":"formats", "text":"7"}),
        json!({"source":"external", "what":"values", "text":"7", "operation":"add"}),
        json!({"source":"external", "what":"values", "text":"7", "transpose":true}),
    ] {
        assert!(s.execute("edit.pasteSpecial", params).is_err());
        assert_eq!(cell(&s, "A1").0, Value::Empty);
    }
}

#[test]
fn vi_vn_paste_matches_typed_formula_canonicalization() {
    let mut s = session();
    s.region = Region::ViVn;
    s.execute("range.setValues", json!({"range":"A1", "values":[[2,3]]})).unwrap();
    for (input, at, expected) in [
        ("=SUM(A1;B1)", "C1", 5.0),
        ("=SUM(1,5;2,25)", "C2", 3.75),
        ("=SUM(1,5)", "C3", 1.5),
        ("=SUM(A1;B1", "C4", 5.0),
        ("=SUM(1.5,2.25)", "C5", 3.75),
    ] {
        s.execute("edit.paste", json!({"at":at, "text":input})).unwrap();
        assert_eq!(cell(&s, at).0, Value::number(expected), "{input}");
    }
    let sh = s.active().unwrap().wb.active().unwrap();
    assert_eq!(sh.cell(CellRef::parse("C1").unwrap()).unwrap().formula.as_ref().unwrap().text, "SUM(A1,B1)");
    // The typed editor canonicalizes before calling cell.set.
    s.execute("cell.set", json!({"cell":"D1", "input":"=SUM(A1,B1)", "region":"vi-VN"})).unwrap();
    let sh = s.active().unwrap().wb.active().unwrap();
    assert_eq!(sh.cell(CellRef::parse("C1").unwrap()).unwrap().formula, sh.cell(CellRef::parse("D1").unwrap()).unwrap().formula);
}

#[test]
fn invalid_vi_formula_stays_text_and_reports_existing_error() {
    let mut s = session();
    s.region = Region::ViVn;
    s.execute("edit.paste", json!({"text":"=SUM(1;*\t7"})).unwrap();
    assert_eq!(cell(&s, "A1").0, Value::text("=SUM(1;*"));
    assert_eq!(cell(&s, "B1").0, Value::number(7.0));
    assert!(s.ui_requests.iter().any(|r| matches!(r, UiRequest::Message(m) if m.contains("There's a problem with this formula"))));
}

#[test]
fn vi_formula_paste_respects_text_format_and_region_override() {
    let mut s = session();
    s.region = Region::ViVn;
    s.execute("home.numberFormat", json!({"range":"A1", "code":"@"})).unwrap();
    s.execute("edit.paste", json!({"text":"=SUM(1,5;2,25)"})).unwrap();
    assert_eq!(cell(&s, "A1").0, Value::text("=SUM(1,5;2,25)"));
    s.execute("edit.paste", json!({"at":"B1", "text":"=SUM(1,5)", "region":"en-US"})).unwrap();
    assert_eq!(cell(&s, "B1").0, Value::number(6.0));
}

#[test]
fn vi_html_formula_paste_keeps_literal_string_attributes() {
    let mut s = session();
    s.region = Region::ViVn;
    s.execute("edit.paste", json!({"html":"<table><tr><td>=SUM(1,5;2,25)</td><td x:str>=SUM(1,5;2,25)</td><td>=SUM(1;*</td></tr></table>"})).unwrap();
    assert_eq!(cell(&s, "A1").0, Value::number(3.75));
    assert_eq!(cell(&s, "B1").0, Value::text("=SUM(1,5;2,25)"));
    assert_eq!(cell(&s, "C1").0, Value::text("=SUM(1;*"));
    assert!(s.ui_requests.iter().any(|r| matches!(r, UiRequest::Message(_))));
}

fn zip_part(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
    xml
}

fn cell_xml<'a>(sheet: &'a str, at: &str) -> &'a str {
    let start = sheet.find(&format!("<c r=\"{at}\"")).unwrap();
    let tail = &sheet[start..];
    &tail[..tail.find("</c>").unwrap() + 4]
}

#[test]
fn rich_external_html_paste_has_golden_xlsx_parts() {
    let mut s = session();
    s.execute("edit.paste", json!({"text":"fallback", "html":HTML})).unwrap();
    let bytes = gridcraft_engine::io::save_bytes(&s.active().unwrap().wb, "clipboard.xlsx").unwrap();
    let sheet = zip_part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<mergeCell ref=\"A1:B1\"/>"), "{sheet}");
    assert!(cell_xml(&sheet, "A1").contains("t=\"s\""), "{sheet}");
    let number = cell_xml(&sheet, "A2");
    assert!(!number.contains("t=\"s\"") && !number.contains("t=\"inlineStr\""), "{number}");
    assert!(number.contains("<v>1234.5</v>"), "{number}");
    assert!(cell_xml(&sheet, "B2").contains("t=\"s\""), "{sheet}");
    let strings = zip_part(&bytes, "xl/sharedStrings.xml");
    assert!(strings.contains(">Heading</t>") && strings.contains(">0012</t>"), "{strings}");
    let styles = zip_part(&bytes, "xl/styles.xml");
    assert!(styles.contains("<b/>"), "{styles}");
    assert!(styles.contains("patternType=\"solid\"") && styles.contains("rgb=\"FFABCDEF\""), "{styles}");
    assert!(styles.contains("<numFmt ") && styles.contains("formatCode=\"0.0000\""), "{styles}");
}
