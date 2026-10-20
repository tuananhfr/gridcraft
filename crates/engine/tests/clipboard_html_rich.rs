//! Synthetic clipboard tables cover the supported spreadsheet interchange vocabulary.

use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_engine::Session;
use gridcraft_model::{BorderStyle, Color, HAlign, Style, Underline, VAlign};
use serde_json::json;

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
fn excel_classes_inline_styles_typed_values_and_merges() {
    let html = r##"<html><head><style>
        td {font-family:'Arial',sans-serif;font-size:12pt;color:#123456}
        .xl65 {font-weight:700;font-style:italic;text-decoration:underline line-through;
          background:#abcdef;text-align:center;vertical-align:middle;white-space:normal;
          border-top:1pt solid #112233;border-right:2pt dashed #445566;
          border-bottom:3pt double #778899;border-left:1pt dotted #aabbcc;
          mso-number-format:'0.00%'}
        </style></head><body><table><tr>
        <td class=xl65 style='color:rgb(9, 8, 7)' rowspan=2 colspan=2 x:num='0.125'>12.50%</td>
        <td x:str>0012</td></tr><tr><td x:fmla='=1+1' x:num='42'>42.00</td></tr></table></body></html>"##;
    let mut s = session();
    s.execute("edit.paste", json!({"at":"C3", "html":html})).unwrap();
    let (value, st) = cell(&s, "C3");
    assert_eq!(value, Value::number(0.125));
    assert!(st.font.bold && st.font.italic && st.font.strike);
    assert_eq!(st.font.underline, Underline::Single);
    assert_eq!(st.font.name, "Arial");
    assert_eq!(st.font.size, 12.0);
    assert_eq!(st.font.color, Color::Rgb(0x090807));
    assert_eq!(st.fill.fg, Color::Rgb(0xabcdef));
    assert_eq!(st.align.h, HAlign::Center);
    assert_eq!(st.align.v, VAlign::Center);
    assert!(st.align.wrap);
    assert_eq!(st.num_fmt.as_str(), "0.00%");
    assert_eq!(st.borders.top.style, BorderStyle::Thin);
    assert_eq!(st.borders.top.color, Color::Rgb(0x112233));
    assert_eq!(st.borders.right.style, BorderStyle::MediumDashed);
    assert_eq!(st.borders.right.color, Color::Rgb(0x445566));
    assert_eq!(st.borders.bottom.style, BorderStyle::Double);
    assert_eq!(st.borders.left.style, BorderStyle::Dotted);
    assert_eq!(cell(&s, "E3").0, Value::text("0012"));
    assert_eq!(cell(&s, "E4").0, Value::number(42.0));
    let sh = s.active().unwrap().wb.active().unwrap();
    assert!(sh.cell(CellRef::parse("E4").unwrap()).unwrap().formula.is_none());
    assert_eq!(sh.merges, [RangeRef::parse("C3:D4").unwrap()]);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().wb.active().unwrap().merges.is_empty());
    assert_eq!(cell(&s, "C3").0, Value::Empty);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(cell(&s, "C3").0, Value::number(0.125));
    assert_eq!(s.active().unwrap().wb.active().unwrap().merges, [RangeRef::parse("C3:D4").unwrap()]);
}

#[test]
fn libreoffice_numeric_attribute_overrides_a_parseable_rounded_display() {
    let html = r##"<table><tr><td sdval='12.345' sdnum='1033;0;0.00'
        style='font-family:"Liberation Sans",serif;font-size:16px;text-align:right;
        vertical-align:top;border-bottom-width:2px;border-bottom-style:solid;border-bottom-color:#f00'><span style='font-weight:bold'>12.35</span></td>
        <td sdval='2.5' style='mso-number-format:unrecognised'>2.5</td></tr></table>"##;
    let mut s = session();
    s.execute("edit.paste", json!({"html":html})).unwrap();
    let (value, st) = cell(&s, "A1");
    assert_eq!(value, Value::number(12.345));
    assert_eq!(st.num_fmt.as_str(), "0.00");
    assert_eq!(st.font.name, "Liberation Sans");
    assert_eq!(st.font.size, 12.0);
    assert_eq!(st.align.v, VAlign::Top);
    assert_eq!(st.borders.bottom.style, BorderStyle::Medium);
    assert_eq!(st.borders.bottom.color, Color::Rgb(0xff0000));
    assert_eq!(cell(&s, "B1").0, Value::number(2.5));
    assert_eq!(cell(&s, "B1").1.num_fmt.as_str(), "General");
}

#[test]
fn sheets_classes_skip_hidden_content_and_preserve_column_positions() {
    let html = r##"<style>.s0 {font-weight:bold;background-color:rgb(255, 238, 0);white-space:nowrap}
        .gone {display:none} table td {color:red} #s0 {font-style:italic}
        </style><table><tr class=gone><td>hidden row</td></tr>
        <tr><td hidden>hidden cell</td><td class=s0 rowspan=2>Label</td><td>7</td></tr>
        <tr><td class=gone>hidden cell</td><td style='text-align:left'>8</td></tr></table>"##;
    let mut s = session();
    s.execute("edit.paste", json!({"html":html})).unwrap();
    let (value, st) = cell(&s, "A1");
    assert_eq!(value, Value::text("Label"));
    assert!(st.font.bold);
    assert!(!st.font.italic);
    assert!(!st.align.wrap);
    assert_eq!(st.fill.fg, Color::Rgb(0xffee00));
    assert_eq!(st.font.color, Color::Auto);
    assert_eq!(cell(&s, "B1").0, Value::number(7.0));
    assert_eq!(cell(&s, "B2").0, Value::number(8.0));
    assert_eq!(s.active().unwrap().wb.active().unwrap().merges, [RangeRef::parse("A1:A2").unwrap()]);
}

#[test]
fn copy_html_round_trips_styles_values_merges_and_multiline_text() {
    let html = r##"<table><tr><td rowspan=2 colspan=2 x:num=12.345 style='font-weight:bold;
        font-style:italic;text-decoration:underline line-through;font-family:Arial;font-size:14pt;
        color:#123456;background:#abcdef;text-align:right;vertical-align:middle;white-space:normal;
        border-left:1px dotted #112233;border-right:2px dashed #445566;
        border-top:3px solid #778899;border-bottom:3px double #aabbcc;mso-number-format:"0.00"'>12.35</td>
        <td x:str style='white-space:pre-wrap'>0012<br>&lt;next&gt;&amp;</td></tr>
        <tr><td x:str>=1+1</td></tr></table>"##;
    let mut source = session();
    source.execute("edit.paste", json!({"html":html})).unwrap();
    let copy = source.execute("edit.copy", json!({"range":"A1:C2", "html":true})).unwrap();
    let copied_html = copy["html"].as_str().unwrap();
    assert!(copied_html.contains("x:num=\"12.345\""));
    assert!(copied_html.contains("rowspan=\"2\" colspan=\"2\""));
    assert!(!copied_html.contains("border:1px solid #d4d4d4"));
    let mut dest = session();
    dest.execute("edit.paste", json!({"html":copied_html, "text":copy["text"]})).unwrap();
    for at in ["A1", "B1", "C1", "A2", "B2", "C2"] {
        assert_eq!(cell(&source, at), cell(&dest, at), "{at}");
    }
    assert_eq!(cell(&dest, "C2").0, Value::text("=1+1"));
    assert_eq!(source.active().unwrap().wb.active().unwrap().merges, dest.active().unwrap().wb.active().unwrap().merges);
}

#[test]
fn clipboard_and_tsv_export_leave_covered_merge_cells_empty() {
    let mut s = session();
    s.execute("edit.paste", json!({"html":"<table><tr><td rowspan=2 colspan=2>M</td><td>X</td></tr><tr><td>Y</td></tr></table>"})).unwrap();
    s.execute("cell.set", json!({"cell":"B1", "input":"covered one"})).unwrap();
    s.execute("cell.set", json!({"cell":"A2", "input":"covered two"})).unwrap();
    s.execute("cell.set", json!({"cell":"B2", "input":"covered three"})).unwrap();
    let copy = s.execute("edit.copy", json!({"range":"A1:C2"})).unwrap();
    assert_eq!(copy["text"], "M\t\tX\n\t\tY\n");
    let clipped = s.execute("edit.copy", json!({"range":"B1:C2", "html":true})).unwrap();
    assert!(!clipped["html"].as_str().unwrap().contains("covered"));
    let bytes = gridcraft_engine::io::save_bytes(&s.active().unwrap().wb, "book.tsv").unwrap();
    let tsv = String::from_utf8(bytes).unwrap();
    assert!(!tsv.contains("covered"));
    assert_eq!(tsv.trim_start_matches('\u{feff}').replace("\r\n", "\n"), "M\t\tX\n\t\tY\n");
}

#[test]
fn hostile_html_falls_back_before_pasting_any_cells() {
    let mut s = session();
    let cases = [
        format!("<table><tr><td>{}x{}</td></tr></table>", "<span>".repeat(65), "</span>".repeat(65)),
        format!("<table><tr>{}</tr></table>", "<td>x</td>".repeat(100_000)),
        format!("<table><tr><td style='{}'>x</td></tr></table>", " ".repeat(8193)),
        format!("<table><tr><td>x</td></tr></table>{}", " ".repeat(8 * 1024 * 1024)),
        "<table><tr><td rowspan=20000 colspan=2>x</td></tr></table>".into(),
    ];
    for html in cases {
        s.execute("edit.paste", json!({"text":"fallback", "html":html})).unwrap();
        assert_eq!(cell(&s, "A1").0, Value::text("fallback"));
        assert_eq!(cell(&s, "B1").0, Value::Empty);
        assert!(s.active().unwrap().wb.active().unwrap().merges.is_empty());
    }
}

#[test]
fn unsafe_css_and_formula_attributes_are_ignored() {
    let html = r##"<style>@import url(https://invalid.example/a.css); .remote {color:red}
        .safe {font-weight:bold}</style><table><tr>
        <td class=safe style='background:url(https://invalid.example/image);font-style:italic' x:fmla='=1+1'>text</td>
        <td x:fmla='=INVALID(' x:str>0012</td></tr></table>"##;
    let mut s = session();
    s.execute("edit.paste", json!({"html":html})).unwrap();
    let (value, st) = cell(&s, "A1");
    assert_eq!(value, Value::text("text"));
    assert!(st.font.bold);
    assert!(!st.font.italic);
    assert_eq!(st.fill, gridcraft_model::Fill::default());
    assert_eq!(cell(&s, "B1").0, Value::text("0012"));
    assert!(s.active().unwrap().wb.active().unwrap().cell(CellRef::parse("A1").unwrap()).unwrap().formula.is_none());
}

#[test]
fn all_hidden_html_does_not_paste_the_plain_text_flavour() {
    let mut s = session();
    s.execute("edit.paste", json!({"text":"secret", "html":"<table><tr hidden><td>secret</td></tr></table>"})).unwrap();
    assert_eq!(cell(&s, "A1").0, Value::Empty);
}

#[test]
fn displayed_formulas_still_parse_but_x_str_forces_text() {
    let mut s = session();
    s.execute("edit.paste", json!({"html":"<table><tr><td>=1+1</td><td x:str>=1+1</td></tr></table>"})).unwrap();
    assert_eq!(cell(&s, "A1").0, Value::number(2.0));
    assert_eq!(cell(&s, "B1").0, Value::text("=1+1"));
}

#[test]
fn preformatted_spaces_tabs_and_literal_text_survive_copy_html() {
    let mut source = session();
    let text = "  first\tsecond  \nlast\u{e000}  ";
    source.execute("range.setValues", json!({"range":"A1", "values":[[text]]})).unwrap();
    let copied = source.execute("edit.copy", json!({"range":"A1", "html":true})).unwrap();
    let mut dest = session();
    dest.execute("edit.paste", json!({"html":copied["html"]})).unwrap();
    assert_eq!(cell(&dest, "A1").0, Value::text(text));
    assert_eq!(cell(&dest, "A1").1, cell(&source, "A1").1);
}
