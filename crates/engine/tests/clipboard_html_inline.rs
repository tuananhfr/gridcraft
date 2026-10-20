//! Synthetic HTML covers formatting that can be represented by a whole-cell style.

use gridcraft_core::{CellRef, Value};
use gridcraft_engine::Session;
use gridcraft_model::{Color, HAlign, Style, Underline};
use serde_json::json;

fn pasted_cell(markup: &str) -> (Value, Style) {
    let mut session = Session::new();
    session.new_workbook();
    let html = format!("<table><tr>{markup}</tr></table>");
    session.execute("edit.paste", json!({"html":html})).unwrap();
    let wb = &session.active().unwrap().wb;
    let sheet = wb.active().unwrap();
    let at = CellRef::parse("A1").unwrap();
    (sheet.value(at), wb.styles.get(sheet.style_id(at)).clone())
}

#[test]
fn inline_bold_tags_style_the_entire_cell() {
    for tag in ["b", "strong", "B", "STRONG"] {
        let (value, style) = pasted_cell(&format!("<td><{tag}>x</{tag}></td>"));
        assert_eq!(value, Value::text("x"), "{tag}");
        assert!(style.font.bold, "{tag}");
    }
}

#[test]
fn nested_inline_emphasis_combines_bold_and_italic() {
    let (value, style) = pasted_cell("<td><strong><em>x</em></strong></td>");
    assert_eq!(value, Value::text("x"));
    assert!(style.font.bold);
    assert!(style.font.italic);
}

#[test]
fn inline_italic_underline_and_strike_aliases_style_the_cell() {
    for tag in ["i", "em"] {
        assert!(pasted_cell(&format!("<td><{tag}>x</{tag}></td>")).1.font.italic, "{tag}");
    }
    assert_eq!(pasted_cell("<td><u>x</u></td>").1.font.underline, Underline::Single);
    for tag in ["s", "strike", "del"] {
        assert!(pasted_cell(&format!("<td><{tag}>x</{tag}></td>")).1.font.strike, "{tag}");
    }
    let (_, style) = pasted_cell("<td><u><s>x</s></u></td>");
    assert_eq!(style.font.underline, Underline::Single);
    assert!(style.font.strike);
}

#[test]
fn legacy_font_attributes_style_the_entire_cell() {
    let (value, style) = pasted_cell(r##"<td><font color="#ff0000" face="Example &amp; Sans,serif" size="4">x</font></td>"##);
    assert_eq!(value, Value::text("x"));
    assert_eq!(style.font.color, Color::Rgb(0xff0000));
    assert_eq!(style.font.name, "Example & Sans");
    assert_eq!(style.font.size, 13.5);
    assert_eq!(pasted_cell(r##"<td><font color="#ff0000">x</font></td>"##).1.font.color, Color::Rgb(0xff0000));
}

#[test]
fn partial_inline_styles_do_not_spread_to_unwrapped_text() {
    for (markup, text) in [
        ("<td>a <b>b</b></td>", "a b"),
        ("<td><b>a</b> b</td>", "a b"),
        ("<td><b>a</b><b>b</b></td>", "ab"),
        ("<td><b></b>x</td>", "x"),
        ("<td>x<em></em></td>", "x"),
        ("<td>a <span style='font-weight:bold;color:red'>b</span></td>", "a b"),
        ("<td><font color='red' face='Example Sans' size='7'>a</font> b</td>", "a b"),
        ("<td><i>a</i> b</td>", "a b"),
        ("<td>a <u>b</u></td>", "a b"),
        ("<td><del>a</del> b</td>", "a b"),
    ] {
        let (value, style) = pasted_cell(markup);
        assert_eq!(value, Value::text(text), "{markup}");
        assert_eq!(style.font, Style::default().font, "{markup}");
    }
}

#[test]
fn only_common_wrappers_contribute_nested_styles() {
    for markup in ["<td><b>x<em>y</em></b></td>", "<td><b><em>x</em> y</b></td>", "<td><b><em>x</em><em>y</em></b></td>"] {
        let (_, style) = pasted_cell(markup);
        assert!(style.font.bold, "{markup}");
        assert!(!style.font.italic, "{markup}");
    }
}

#[test]
fn surrounding_whitespace_comments_and_hidden_text_do_not_break_wrappers() {
    for markup in [
        "<td> \n <b>x</b> \t </td>",
        "<td>&nbsp;&#32;<b>&#120;</b>&#x20;</td>",
        "<td><!--before--><b>x<!--inside--></b><!--after--></td>",
        "<td><span hidden>ignored</span><b>x</b><span style='display:none'>ignored</span></td>",
    ] {
        assert!(pasted_cell(markup).1.font.bold, "{markup}");
    }
}

#[test]
fn full_cell_span_css_and_classes_are_preserved() {
    let (_, style) = pasted_cell("<td><span style='font-weight:bold;font-style:italic;color:blue'>x</span></td>");
    assert!(style.font.bold && style.font.italic);
    assert_eq!(style.font.color, Color::Rgb(0x0000ff));
    let (_, style) = pasted_cell("<td><style>.emphasis {font-weight:bold}</style><span class=emphasis>x</span></td>");
    assert!(style.font.bold);
}

#[test]
fn whole_cell_emphasis_preserves_line_breaks() {
    let (value, style) = pasted_cell("<td><b>x<br>y</b></td>");
    assert_eq!(value, Value::text("x\ny"));
    assert!(style.font.bold);
}

#[test]
fn cell_inline_css_wins_over_descendant_styles() {
    assert!(!pasted_cell("<td style='font-weight:normal'><b>x</b></td>").1.font.bold);
    let (_, style) = pasted_cell(
        "<td style='font-weight:normal;font-style:normal;text-decoration:none;color:blue;font-family:CellFont;font-size:20pt'>\
        <b><em><u><del><font color=red face=ChildFont size=7>x</font></del></u></em></b></td>",
    );
    assert!(!style.font.bold && !style.font.italic && !style.font.strike);
    assert_eq!(style.font.underline, Underline::None);
    assert_eq!(style.font.color, Color::Rgb(0x0000ff));
    assert_eq!(style.font.name, "CellFont");
    assert_eq!(style.font.size, 20.0);
    let (_, style) = pasted_cell("<td style='color:blue'><span style='color:red;font-weight:bold'>x</span></td>");
    assert_eq!(style.font.color, Color::Rgb(0x0000ff));
    assert!(style.font.bold);
}

#[test]
fn inline_emphasis_preserves_libreoffice_number_format_precedence() {
    let (value, style) = pasted_cell("<td sdval='1.25' sdnum='1033;0;0.00' style='mso-number-format:0%'><b>1.25</b></td>");
    assert_eq!(value, Value::number(1.25));
    assert_eq!(style.num_fmt.as_str(), "0.00");
    assert!(style.font.bold);
}

#[test]
fn table_headers_default_to_bold_and_centered_with_css_overrides() {
    let (value, style) = pasted_cell("<th>x</th>");
    assert_eq!(value, Value::text("x"));
    assert!(style.font.bold);
    assert_eq!(style.align.h, HAlign::Center);
    let (_, style) = pasted_cell("<th style='font-weight:normal;text-align:left'>x</th>");
    assert!(!style.font.bold);
    assert_eq!(style.align.h, HAlign::Left);
}

#[test]
fn subscript_and_superscript_tags_keep_plain_text_and_inherited_emphasis() {
    for tag in ["sub", "sup"] {
        let (value, style) = pasted_cell(&format!("<td><{tag}>x</{tag}></td>"));
        assert_eq!(value, Value::text("x"));
        assert_eq!(style.font, Style::default().font);
        assert!(pasted_cell(&format!("<td><b>a<{tag}>b</{tag}></b></td>")).1.font.bold);
    }
}

#[test]
fn captions_are_ignored_and_cell_styles_do_not_leak() {
    let html = "<table><caption><b>ignored</b><table><tr><td>also ignored</td></tr></table></caption>\
        <tr><th>header</th><td><b>from</b></td><td>another app</td></tr><caption>ignored too</caption></table>";
    let mut session = Session::new();
    session.new_workbook();
    session.execute("edit.paste", json!({"html":html})).unwrap();
    let wb = &session.active().unwrap().wb;
    let sheet = wb.active().unwrap();
    for (at, text, bold) in [("A1", "header", true), ("B1", "from", true), ("C1", "another app", false)] {
        let at = CellRef::parse(at).unwrap();
        assert_eq!(sheet.value(at), Value::text(text));
        assert_eq!(wb.styles.get(sheet.style_id(at)).font.bold, bold);
    }
    assert_eq!(sheet.value(CellRef::parse("A2").unwrap()), Value::Empty);
    assert_eq!(sheet.value(CellRef::parse("D1").unwrap()), Value::Empty);
}
