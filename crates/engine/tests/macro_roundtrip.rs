//! A macro-enabled workbook (.xlsm) keeps its VBA project byte for byte through open, edit and
//! save, and opening it never runs anything.

use std::io::Read as _;

use gridcraft_engine::Session;
use gridcraft_engine::io::{base64_decode, base64_encode, drops_macros, save_bytes, workbook_ext};
use gridcraft_engine::model::Workbook;
use serde_json::json;

/// Not a real VBA project: arbitrary bytes, including non-UTF-8 ones, that must come back as is.
const VBA: &[u8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1\x00\xff\xfe synthetic vbaProject \x00\x01\x02\x80";

/// A session holding `macro.xlsm` as the browser opens it (bytes, no path), with A1 = 42 and an
/// `Auto_Open` name, the hook Excel would run on open.
fn opened_xlsm() -> Session {
    let mut author = Session::new();
    author.new_workbook();
    author.execute("cell.set", json!({"cell": "A1", "input": "42"})).unwrap();
    author.execute("formulas.defineName", json!({"name": "Auto_Open", "refersTo": "=Sheet1!$A$1"})).unwrap();
    let mut wb: Workbook = (*author.doc().unwrap().wb).clone();
    wb.vba_project = Some(VBA.to_vec());
    let package = save_bytes(&wb, "macro.xlsm").unwrap();
    let mut s = Session::new();
    s.execute("file.open", json!({"name": "macro.xlsm", "base64": base64_encode(&package)})).unwrap();
    s
}

fn part(package: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(package)).unwrap();
    let mut file = zip.by_name(name).ok()?;
    let mut out = Vec::new();
    file.read_to_end(&mut out).unwrap();
    Some(out)
}

#[test]
fn an_opened_xlsm_saves_as_xlsm_with_the_identical_vba_project() {
    let mut s = opened_xlsm();
    s.execute("cell.set", json!({"cell": "B2", "input": "edited"})).unwrap();
    let format = workbook_ext(&s.doc().unwrap().display_title());
    assert_eq!(format, "xlsm", "the document keeps the kind it was opened as");
    let saved = s.execute("file.saveBytes", json!({"format": format})).unwrap();
    assert_eq!(saved["name"], "macro.xlsm");
    let package = base64_decode(saved["base64"].as_str().unwrap()).unwrap();
    assert_eq!(part(&package, "xl/vbaProject.bin").as_deref(), Some(VBA), "the VBA project is written back unchanged");
    let types = String::from_utf8(part(&package, "[Content_Types].xml").unwrap()).unwrap();
    assert!(types.contains("application/vnd.ms-excel.sheet.macroEnabled.main+xml"), "{types}");
    let (back, _) = gridcraft_engine::io::open_bytes("macro.xlsm", &package).unwrap();
    assert_eq!(back.vba_project.as_deref(), Some(VBA));
    assert_eq!(back.sheet(0).unwrap().value(gridcraft_engine::core::CellRef::new(1, 1)).display(), "edited");
}

#[test]
fn an_explicit_xlsx_still_leaves_the_vba_project_out() {
    let mut s = opened_xlsm();
    let saved = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap();
    assert_eq!(saved["name"], "macro.xlsx");
    let package = base64_decode(saved["base64"].as_str().unwrap()).unwrap();
    assert!(part(&package, "xl/vbaProject.bin").is_none());
    let wb = &s.doc().unwrap().wb;
    assert!(drops_macros(wb, "macro.xlsx") && drops_macros(wb, "Report.XLTX"));
    assert!(!drops_macros(wb, "macro.xlsm") && !drops_macros(wb, "t.xltm"), "macro-enabled kinds keep it");
    assert!(!drops_macros(wb, "macro.csv") && !drops_macros(wb, "macro.pdf"), "exports are copies");
    assert!(!drops_macros(&Workbook::new(), "plain.xlsx"), "nothing to drop");
}

#[test]
fn opening_a_macro_workbook_runs_nothing() {
    let s = opened_xlsm();
    let d = s.doc().unwrap();
    assert!(!d.is_dirty() && d.undo.is_empty(), "opening changed nothing");
    assert_eq!(d.wb.vba_project.as_deref(), Some(VBA));
    assert_eq!(d.wb.sheet(0).unwrap().value(gridcraft_engine::core::CellRef::new(0, 0)).display(), "42");
    assert!(s.journal.iter().all(|(id, _)| id == "file.open"), "no command ran besides the open: {:?}", s.journal);
}

#[test]
fn workbook_ext_keeps_only_the_macro_enabled_kinds() {
    assert_eq!(workbook_ext("macro.xlsm"), "xlsm");
    assert_eq!(workbook_ext("MACRO.XLSM"), "xlsm");
    assert_eq!(workbook_ext("t.xltm"), "xltm");
    for plain in ["Book1", "a.xlsx", "t.xltx", "data.csv", "x (imported).xlsx"] {
        assert_eq!(workbook_ext(plain), "xlsx", "{plain}");
    }
}

#[test]
fn drafts_of_a_macro_workbook_keep_the_vba_project() {
    let s = opened_xlsm();
    let wb = &s.doc().unwrap().wb;
    let draft = gridcraft_engine::io::save_xlsx_fast(wb, "macro.xlsm").unwrap();
    assert_eq!(part(&draft, "xl/vbaProject.bin").as_deref(), Some(VBA));
    let plain = gridcraft_engine::io::save_xlsx_fast(wb, "macro.xlsx").unwrap();
    assert!(part(&plain, "xl/vbaProject.bin").is_none());
    let types = String::from_utf8(part(&plain, "[Content_Types].xml").unwrap()).unwrap();
    assert!(types.contains("spreadsheetml.sheet.main+xml"), "a template name still drafts a workbook: {types}");
}
