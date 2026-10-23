use gridcraft_engine::{
    DocState, Session,
    core::{CellRef, RangeRef, Value},
    model::{Cell, Formula, Sheet, Style, Workbook},
    vn_legacy::{Encoding, encoding},
};
use serde_json::json;
use std::sync::Arc;

fn fixture() -> Session {
    let mut wb = Workbook::new();
    wb.cell_styles.push(("Normal".into(), Style::default()));
    wb.sheets.push(Arc::new(Sheet::new("Other")));
    let legacy = wb.styles.derive(Default::default(), |st| {
        st.font.name = ".VnTime".into();
        st.font.size = 17.0;
        st.font.bold = true;
        st.font.italic = true;
        st.font.color = gridcraft_engine::model::Color::Rgb(0x123456);
    });
    for sheet in &mut wb.sheets {
        let sh = Arc::make_mut(sheet);
        sh.set_value(CellRef::new(0, 0), Value::from("b\u{b6}ng c\u{ad}\u{ea}ng \u{ae}\u{e9}"));
        sh.set_style(CellRef::new(0, 0), legacy);
        sh.set_value(CellRef::new(1, 0), Value::from("Ti\u{1ebf}ng Vi\u{1ec7}t"));
        sh.set_cell(CellRef::new(2, 0), Cell::formula(Formula::new("=1+2")));
        sh.set_style(CellRef::new(2, 0), legacy);
        sh.set_value(CellRef::new(3, 0), Value::Number(45000.0));
        sh.set_style(CellRef::new(3, 0), legacy);
    }
    let mut session = Session::new();
    session.add_document(DocState::new(wb, None, "Legacy".into())).unwrap();
    session
}

#[test]
fn detects_font_families_and_capital_variants() {
    for name in [".VnTime", " .vntime ", "TCVN3", "ABC", "abc-serif", "ABC Serif"] {
        assert_eq!(encoding(name), Some(Encoding::Tcvn3));
    }
    for name in [".VnTimeH", ".VNARIALH"] {
        assert_eq!(encoding(name), Some(Encoding::Tcvn3Upper));
    }
    for name in ["VNI-Times", " vni Times "] {
        assert_eq!(encoding(name), Some(Encoding::Vni));
    }
    for name in ["Calibri", "Arial", "Noto Sans", "Vietnamese", "ABCDEF"] {
        assert_eq!(encoding(name), None);
    }
}

#[test]
fn automatic_workbook_conversion_roundtrip_and_one_undo() {
    let mut s = fixture();
    let original = s.doc().unwrap().wb.clone();
    let result = s.execute("review.convertVietnameseLegacy", json!({})).unwrap();
    assert_eq!(result["convertedCells"], 2);
    assert_eq!(result["skippedFormulaCells"], 2);
    let wb = &s.doc().unwrap().wb;
    for sheet in &wb.sheets {
        assert_eq!(sheet.value(CellRef::new(0, 0)), Value::from("b\u{1ea3}ng c\u{1b0}\u{1edd}ng \u{111}\u{1ed9}"));
        assert_eq!(sheet.value(CellRef::new(1, 0)), Value::from("Ti\u{1ebf}ng Vi\u{1ec7}t"));
        assert!(sheet.cell(CellRef::new(2, 0)).unwrap().formula.is_some());
        assert_eq!(sheet.value(CellRef::new(3, 0)), Value::Number(45000.0));
        let mut expected = original.styles.get(original.sheets[0].style_id(CellRef::new(0, 0))).clone();
        expected.font.name = gridcraft_engine::model::DEFAULT_FONT.into();
        assert_eq!(wb.styles.get(sheet.style_id(CellRef::new(0, 0))), &expected);
    }
    let bytes = gridcraft_xlsx::write_xlsx(wb).unwrap();
    let (back, _) = gridcraft_xlsx::read_xlsx(&bytes).unwrap();
    for sheet in &back.sheets {
        assert_eq!(sheet.value(CellRef::new(0, 0)), Value::from("b\u{1ea3}ng c\u{1b0}\u{1edd}ng \u{111}\u{1ed9}"));
        assert_eq!(back.styles.get(sheet.style_id(CellRef::new(0, 0))).font.name, gridcraft_engine::model::DEFAULT_FONT);
    }
    assert_eq!(s.doc().unwrap().undo.len(), 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().wb.as_ref(), original.as_ref());
}

#[test]
fn automatic_selection_stays_on_selected_cells_and_sheet() {
    let mut s = fixture();
    s.doc_mut().unwrap().selection = gridcraft_engine::Selection::range(RangeRef::parse("A1:A2").unwrap());
    let result = s.execute("review.convertVietnameseLegacy", json!({})).unwrap();
    assert_eq!(result["convertedCells"], 1);
    assert_eq!(result["skippedFormulaCells"], 0);
    assert_eq!(s.doc().unwrap().wb.sheets[1].value(CellRef::new(0, 0)), Value::from("b\u{b6}ng c\u{ad}\u{ea}ng \u{ae}\u{e9}"));
}

#[test]
fn forced_commands_convert_only_selection_regardless_of_font() {
    for (command, source, expected) in
        [("review.convertTcvn3", "b\u{b6}ng", "b\u{1ea3}ng"), ("review.convertVni", "Tie\u{e1}ng Vie\u{e4}t", "Ti\u{1ebf}ng Vi\u{1ec7}t")]
    {
        let mut s = fixture();
        let d = s.doc_mut().unwrap();
        let wb = Arc::make_mut(&mut d.wb);
        Arc::make_mut(wb.sheets.first_mut().unwrap()).set_value(CellRef::new(1, 0), Value::from(source));
        d.selection = gridcraft_engine::Selection::at(CellRef::new(1, 0));
        s.execute(command, json!({})).unwrap();
        assert_eq!(s.doc().unwrap().wb.sheets[0].value(CellRef::new(1, 0)), Value::from(expected));
        assert_eq!(s.doc().unwrap().wb.sheets[1].value(CellRef::new(0, 0)), Value::from("b\u{b6}ng c\u{ad}\u{ea}ng \u{ae}\u{e9}"));
    }
}

#[test]
fn forced_conversion_requires_selection_and_protection_rolls_back_all_sheets() {
    let mut s = fixture();
    s.doc_mut().unwrap().selection.ranges.clear();
    assert!(s.execute("review.convertTcvn3", json!({})).is_err());
    s.doc_mut().unwrap().selection = Default::default();
    s.execute("review.protectSheet", json!({})).unwrap();
    let before = s.doc().unwrap().wb.clone();
    let history = s.doc().unwrap().undo.len();
    assert!(s.execute("review.convertVietnameseLegacy", json!({})).is_err());
    assert_eq!(s.doc().unwrap().wb.as_ref(), before.as_ref());
    assert_eq!(s.doc().unwrap().undo.len(), history);
}

#[test]
fn uppercase_font_and_named_normal_font_are_used() {
    let mut s = fixture();
    let wb = Arc::make_mut(&mut s.doc_mut().unwrap().wb);
    wb.cell_styles.first_mut().unwrap().1.font.name = "Noto Serif".into();
    let style = wb.styles.derive(Default::default(), |s| s.font.name = ".VnTimeH".into());
    let sheet = Arc::make_mut(wb.sheets.first_mut().unwrap());
    sheet.set_value(CellRef::new(0, 0), Value::from("b\u{b6}ng"));
    sheet.set_style(CellRef::new(0, 0), style);
    s.execute("review.convertVietnameseLegacy", json!({})).unwrap();
    let wb = &s.doc().unwrap().wb;
    assert_eq!(wb.sheets[0].value(CellRef::new(0, 0)), Value::from("B\u{1ea2}NG"));
    assert_eq!(wb.styles.get(wb.sheets[0].style_id(CellRef::new(0, 0))).font.name, "Noto Serif");
}
