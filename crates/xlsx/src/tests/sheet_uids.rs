//! Sheet uids survive save and open; files without them, or with bad ones, open without them.

use std::io::Read;
use std::sync::Arc;

use gridcraft_model::{Sheet, Workbook};

use crate::{read_xlsx, write_xlsx};

fn workbook_xml(bytes: &[u8]) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name("xl/workbook.xml").unwrap().read_to_string(&mut xml).unwrap();
    xml
}

#[test]
fn sheet_uids_survive_save_and_open() {
    let mut wb = Workbook::new();
    wb.sheets = ["One", "Two", "Three"].iter().map(|n| Arc::new(Sheet::new(*n))).collect();
    wb.sheet_mut(0).unwrap().uid = "f1".into();
    wb.sheet_mut(2).unwrap().uid = "a1b2c3-7".into();
    let bytes = write_xlsx(&wb).unwrap();
    let xml = workbook_xml(&bytes);
    // `sheetId` stays the small number Office requires.
    assert!(xml.contains(r#"sheetId="3""#), "{xml}");
    let (back, _) = read_xlsx(&bytes).unwrap();
    let uids: Vec<_> = back.sheets.iter().map(|s| s.uid.as_str()).collect();
    assert_eq!(uids, ["f1", "", "a1b2c3-7"]);
    // Saving again writes the extension once, not a kept copy beside a new one.
    let again = write_xlsx(&back).unwrap();
    assert_eq!(workbook_xml(&again).matches("sheetUids>").count(), 2);
    assert_eq!(read_xlsx(&again).unwrap().0.sheets[2].uid, "a1b2c3-7");
}

#[test]
fn bad_or_unknown_sheet_uids_are_ignored() {
    let ext = |uid: &str, rid: &str| {
        format!(
            r#"<extLst><ext uri="{{8A0C7A52-3E44-4B6B-9F1D-5C2E7B9D4A61}}" xmlns:ecid="https://office.erpcons.ai/ns/gridcraft/2026/ids"><ecid:sheetUids><ecid:sheet rid="{rid}" uid="{uid}"/></ecid:sheetUids></ext></extLst>"#
        )
    };
    for (uid, rid, expected) in [("c0ffee-2", "rId1", "c0ffee-2"), ("NOT VALID", "rId1", ""), ("c0ffee-2", "rId9", ""), (&"a".repeat(40), "rId1", "")]
    {
        let bytes = super::minimal("<sheetData/>", &[], "", &ext(uid, rid));
        let (wb, _) = read_xlsx(&bytes).unwrap();
        assert_eq!(wb.sheets[0].uid, expected, "{uid} {rid}");
    }
}
