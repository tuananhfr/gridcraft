use std::io::{Cursor, Write};

use gridcraft_engine::{Session, io, legacy};
use serde_json::json;

#[test]
fn legacy_vba_defaults_to_xlsm_and_xlsx_requires_the_macro_loss_warning() {
    let rec = |id: u16, payload: &[u8]| [&id.to_le_bytes()[..], &(payload.len() as u16).to_le_bytes(), payload].concat();
    let bof = |kind: u16| rec(0x0809, &[&0x0600u16.to_le_bytes()[..], &kind.to_le_bytes(), &[0; 12]].concat());
    let globals = |offset: u32| [bof(0x0005), rec(0x0085, &[&offset.to_le_bytes()[..], &[0, 0, 3, 0], b"Old"].concat()), rec(0x000A, &[])].concat();
    let workbook = [globals(globals(0).len() as u32), bof(0x0010), rec(0x000A, &[])].concat();
    let mut ole = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    ole.create_stream("/Workbook").unwrap().write_all(&workbook).unwrap();
    ole.create_storage_all("/_VBA_PROJECT_CUR/VBA").unwrap();
    ole.create_stream("/_VBA_PROJECT_CUR/PROJECT").unwrap().write_all(b"ID=synthetic").unwrap();
    let mut session = Session::new();
    session.execute("file.open", json!({"name": "Old.xls", "base64": io::base64_encode(&ole.into_inner().into_inner())})).unwrap();
    let doc = session.doc().unwrap();
    assert!(doc.display_title().ends_with(".xlsm"));
    assert!(doc.path.is_none(), "the source .xls must never be overwritten");
    assert_eq!(io::workbook_ext(&doc.display_title()), "xlsm");
    assert!(io::drops_macros(&doc.wb, "Old.xlsx"));
    assert!(!io::drops_macros(&doc.wb, "Old.xlsm"));
    assert!(session.ui_requests.iter().any(|r| matches!(r, gridcraft_engine::UiRequest::Message(text) if text == legacy::OPENED_MACROS)));
    let saved = session.execute("file.saveBytes", json!({})).unwrap();
    let bytes = io::base64_decode(saved["base64"].as_str().unwrap()).unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert!(zip.by_name("xl/vbaProject.bin").is_ok());
}
