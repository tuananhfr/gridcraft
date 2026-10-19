//! Values displayed in a regional format: what the user sees (cell text, clipboard, Find, filter
//! and slicer captions) follows the session's region, while files and agent-facing data keep the
//! invariant en-US notation.
use gridcraft_core::{CellRef, Region};
use gridcraft_engine::{Session, display};
use serde_json::json;

/// A sheet with a number, a grouped amount, ₫ amounts, a percent and dates, as typed in vi-VN.
fn vietnamese_sheet() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    for (cell, input) in [
        ("A1", "Khoản"),
        ("B1", "Số tiền"),
        ("A2", "1234,5"),
        ("B2", "1.234,5"),
        ("A3", "1.000.000 ₫"),
        ("B3", "12,5%"),
        ("A4", "19/10/2026"),
        ("B4", "19/10/2026 13:30"),
    ] {
        s.execute("cell.set", json!({"cell": cell, "input": input, "region": "vi-VN"})).unwrap();
    }
    s.execute("home.accounting", json!({"range": "A5", "symbol": "₫"})).unwrap();
    s.execute("cell.set", json!({"cell": "A5", "input": "2500000"})).unwrap();
    s
}

fn text_in(s: &Session, a1: &str, region: Region) -> String {
    let d = s.active().unwrap();
    display::cell_text_in(&d.wb, d.wb.active().unwrap(), CellRef::parse(a1).unwrap(), region)
}

#[test]
fn cells_show_in_the_region_and_stay_en_us_elsewhere() {
    let s = vietnamese_sheet();
    for (cell, en, vi) in [
        ("A2", "1234.5", "1234,5"),
        ("B2", "1,234.50", "1.234,50"),
        ("A3", "1,000,000 ₫", "1.000.000 ₫"),
        ("B3", "12.50%", "12,50%"),
        ("A4", "10/19/2026", "19/10/2026"),
        ("B4", "10/19/2026 13:30", "19/10/2026 13:30"),
        ("A5", "  2,500,000 ₫ ", "  2.500.000 ₫ "),
        ("A1", "Khoản", "Khoản"),
    ] {
        assert_eq!(text_in(&s, cell, Region::EnUs), en, "{cell} en-US");
        assert_eq!(text_in(&s, cell, Region::EsEs), en, "{cell} es-ES");
        assert_eq!(text_in(&s, cell, Region::ViVn), vi, "{cell} vi-VN");
        let d = s.active().unwrap();
        assert_eq!(display::cell_text(&d.wb, d.wb.active().unwrap(), CellRef::parse(cell).unwrap()), en, "{cell} plain");
    }
    let d = s.active().unwrap();
    let a3 = d.wb.active().unwrap().style_id(CellRef::parse("A3").unwrap());
    assert_eq!(d.wb.styles.get(a3).num_fmt.as_str(), "#,##0 [$₫-42A]", "files keep the code");
}

#[test]
fn the_cell_editor_starts_from_text_that_reads_back() {
    let s = vietnamese_sheet();
    let d = s.active().unwrap();
    let sh = d.wb.active().unwrap();
    let edit = |a1: &str| display::edit_text_in(&d.wb, sh, CellRef::parse(a1).unwrap(), Region::ViVn);
    assert_eq!(edit("A4"), "19/10/2026");
    assert_eq!(edit("B2"), "1234,5");
}

#[test]
fn copy_and_paste_text_follow_the_session_region() {
    let mut s = vietnamese_sheet();
    let en = s.execute("edit.copy", json!({"range": "A2:B3"})).unwrap();
    assert_eq!(en["text"], "1234.5\t1,234.50\n1,000,000 ₫\t12.50%\n");
    s.region = Region::ViVn;
    let vi = s.execute("edit.copy", json!({"range": "A2:B3", "html": true})).unwrap();
    assert_eq!(vi["text"], "1234,5\t1.234,50\n1.000.000 ₫\t12,50%\n");
    assert!(vi["html"].as_str().unwrap().contains("1.234,50"));
    // An explicit region wins over the session's.
    let param = s.execute("edit.copy", json!({"range": "A2", "region": "en-US"})).unwrap();
    assert_eq!(param["text"], "1234.5\n");
    // Text from another app in the vi-VN notation pastes as numbers and dates, as typing does.
    s.execute("edit.paste", json!({"at": "D1", "text": "1.234,5\t19/10/2026\n12,5%\t1.000.000 ₫\n"})).unwrap();
    let d = s.active().unwrap();
    let sh = d.wb.active().unwrap();
    let v = |a1: &str| sh.value(CellRef::parse(a1).unwrap());
    assert_eq!(v("D1").as_f64(), Some(1234.5));
    assert_eq!(v("E1").as_f64(), Some(46314.0));
    assert_eq!(v("D2").as_f64(), Some(0.125));
    assert_eq!(v("E2").as_f64(), Some(1_000_000.0));
    assert_eq!(text_in(&s, "E1", Region::ViVn), "19/10/2026");
}

#[test]
fn find_matches_values_as_shown() {
    let mut s = vietnamese_sheet();
    assert!(s.execute("edit.find", json!({"what": "1.234,50"})).is_err());
    s.region = Region::ViVn;
    let r = s.execute("edit.find", json!({"what": "1.234,50", "all": true})).unwrap();
    assert_eq!(r["results"][0]["cell"], "B2");
    assert_eq!(r["results"][0]["value"], "1.234,50");
    let r = s.execute("edit.find", json!({"what": "19/10/2026", "all": true})).unwrap();
    assert_eq!(r["count"], 2);
}

#[test]
fn filter_and_slicer_captions_are_regional_while_values_stay_en_us() {
    let mut s = vietnamese_sheet();
    s.execute("data.filter", json!({"range": "A1:B4"})).unwrap();
    let d = s.active().unwrap();
    let labeled = gridcraft_engine::cmd::data::filter_values_in(&d.wb, 0, 1, Region::ViVn);
    let pairs: Vec<(&str, &str)> = labeled.iter().map(|(v, l, _)| (v.as_str(), l.as_str())).collect();
    assert_eq!(pairs, [("12.50%", "12,50%"), ("1,234.50", "1.234,50"), ("10/19/2026 13:30", "19/10/2026 13:30")]);
    let plain = gridcraft_engine::cmd::data::filter_values(&d.wb, 0, 1);
    assert_eq!(plain.iter().map(|(v, _)| v.as_str()).collect::<Vec<_>>(), ["12.50%", "1,234.50", "10/19/2026 13:30"]);
    // The value a caption stands for filters the column.
    s.execute("data.filterBy", json!({"column": "B", "values": ["1,234.50"]})).unwrap();
    let sh = s.active().unwrap().wb.active().unwrap();
    assert!(!sh.is_row_hidden(1) && sh.is_row_hidden(2) && sh.is_row_hidden(3));

    let mut s = vietnamese_sheet();
    s.execute("insert.table", json!({"range": "A1:B4", "header": true})).unwrap();
    s.execute("insert.slicer", json!({"column": "Số tiền"})).unwrap();
    let d = s.active().unwrap();
    let sh = d.wb.active().unwrap();
    let sl = sh.slicers.first().unwrap();
    let tiles = gridcraft_engine::cmd::slicer::slicer_items_in(&d.wb, 0, sl, Region::ViVn);
    let tiles: Vec<(&str, &str)> = tiles.iter().map(|(v, l, _)| (v.as_str(), l.as_str())).collect();
    assert_eq!(tiles, [("1,234.50", "1.234,50"), ("12.50%", "12,50%"), ("10/19/2026 13:30", "19/10/2026 13:30")]);
}

#[test]
fn csv_and_inspect_stay_en_us() {
    let mut s = vietnamese_sheet();
    s.region = Region::ViVn;
    let d = s.active().unwrap();
    let sh = d.wb.active().unwrap();
    assert_eq!(display::value_text(&d.wb, sh, CellRef::parse("B2").unwrap()), "1,234.50");
    let r = s.execute("cell.get", json!({"cell": "B2"})).unwrap();
    assert_eq!(r["text"], "1,234.50");
}

#[test]
fn ctrl_semicolon_texts_read_back_in_the_region() {
    // Ctrl+; and Ctrl+Shift+; type today's date and the time with these codes.
    let s = vietnamese_sheet();
    let wb = &s.active().unwrap().wb;
    for (n, code) in [(46314.0, "m/d/yyyy"), (0.5625, "h:mm AM/PM")] {
        let text = display::format_in(&gridcraft_core::Value::Number(n), code, wb, Region::ViVn).text;
        let read = gridcraft_core::parse::parse_input_in(&text, wb.date_system, Region::ViVn).value;
        assert_eq!(read.as_f64(), Some(n), "{text}");
    }
}
