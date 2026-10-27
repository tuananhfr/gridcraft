//! `SheetApp::open_dropped` tells the web shell whether a dropped file was accepted, which it
//! reports to the office shell's `erpIntakeResult`.

use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

fn xlsx() -> Vec<u8> {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell": "A1", "input": "42"})).unwrap();
    gridcraft_engine::io::save_bytes(&s.active().unwrap().wb, "book.xlsx").unwrap()
}

fn app() -> SheetApp {
    let mut session = Session::new();
    session.new_workbook();
    SheetApp::new(session, Default::default())
}

#[test]
fn a_dropped_workbook_is_accepted_and_opens() {
    let mut app = app();
    assert!(app.open_dropped("Báo cáo.xlsx", &xlsx()));
    assert_eq!(app.session.active().unwrap().display_title(), "Báo cáo.xlsx");
}

#[test]
fn a_file_that_does_not_parse_is_refused() {
    let mut app = app();
    assert!(!app.open_dropped("broken.xlsx", b"PK\x03\x04 not really a zip"));
    assert!(!app.open_dropped("old.xls", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1 truncated"));
    assert!(app.message.is_some(), "the refusal is shown");
}

#[test]
fn the_document_limit_refuses_an_eleventh_drop() {
    let mut app = app();
    for i in 0..gridcraft_engine::MAX_DOCUMENTS {
        assert!(app.open_dropped(&format!("book{i}.xlsx"), &xlsx()), "document {i}");
    }
    assert_eq!(app.session.documents().len(), gridcraft_engine::MAX_DOCUMENTS);
    assert!(!app.open_dropped("one too many.xlsx", &xlsx()));
}

#[test]
fn a_password_protected_drop_is_accepted_while_it_asks() {
    let mut app = app();
    let encrypted = gridcraft_xlsx::testing::encrypt_workbook(&xlsx(), "synthetic-pw-3317");
    assert!(app.open_dropped("Secret.xlsx", &encrypted));
    assert_eq!(app.dialog.as_ref().map(|d| d.name()), Some("openPassword"));
}
