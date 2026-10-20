//! A zip bomb opened from disk shows the translated "can't open" message and opens nothing.
use gridcraft_engine::Session;
use gridcraft_engine::io::FILE_TOO_LARGE;
use gridcraft_ui_egui::i18n::Language;
use gridcraft_ui_egui::{Services, SheetApp};

fn bomb() -> Vec<u8> {
    let types = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#;
    gridcraft_xlsx::testing::zip_package(&[("[Content_Types].xml", types), ("xl/workbook.xml", &vec![b' '; 64 << 20])])
}

#[test]
fn a_zip_bomb_shows_the_translated_reason_and_opens_nothing() {
    let dir = std::env::temp_dir().join(format!("gridcraft-bomb-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Bomb.xlsx");
    std::fs::write(&path, bomb()).unwrap();
    let mut session = Session::new();
    session.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, Services::default()),
    );
    h.run_steps(2);
    h.state_mut().open_path(&path.to_string_lossy());
    h.run_steps(2);
    assert_eq!(h.state().message.as_ref().map(|(_, m)| m.as_str()), Some(FILE_TOO_LARGE));
    assert_eq!(h.state().session.documents().len(), 1, "only the blank workbook");
    assert_eq!(Language::Vi.tr(FILE_TOO_LARGE), "Tệp quá lớn hoặc bị nén bất thường nên không mở được.");
    let _ = std::fs::remove_dir_all(&dir);
}

fn no_workbook_part() -> Vec<u8> {
    let types = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#;
    gridcraft_xlsx::testing::zip_package(&[("[Content_Types].xml", types)])
}

#[test]
fn a_spreadsheet_missing_its_workbook_part_says_so_in_vietnamese() {
    let dir = std::env::temp_dir().join(format!("gridcraft-no-workbook-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Báo cáo.xlsx");
    std::fs::write(&path, no_workbook_part()).unwrap();
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Services::default());
    app.ui.language = Language::Vi;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(2);
    h.state_mut().open_path(&path.to_string_lossy());
    h.run_steps(2);
    let msg = h.state().message.as_ref().map(|(_, m)| m.clone()).unwrap_or_default();
    assert!(msg.starts_with("We can't open '"), "the engine's message stays English for agents: {msg}");
    use egui_kittest::kittest::Queryable;
    assert!(
        h.query_by_label_contains("Báo cáo.xlsx': tệp không phải là bảng tính hợp lệ: thiếu phần sổ làm việc")
            .is_some_and(|n| n.value().is_some_and(|v| v.starts_with("Không mở được '"))),
        "the message box shows it in Vietnamese"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every refusal `gridcraft_engine::io` gives for a broken file reads in Vietnamese, so the
/// templates in `i18n/messages.rs` keep up with the engine's wording.
#[test]
fn open_errors_for_broken_files_translate_whole() {
    use gridcraft_engine::io::{open_bytes, open_bytes_with_password};
    use gridcraft_xlsx::testing::{compound_file, encrypt_workbook, zip_package};
    let refusal = |r: gridcraft_engine::Result<(gridcraft_engine::model::Workbook, Vec<String>)>| r.err().map(|e| e.to_string()).unwrap_or_default();
    let types: &[u8] = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#;
    let deep = "<workbook>".repeat(300);
    let bad_xml = zip_package(&[("[Content_Types].xml", types), ("xl/workbook.xml", deep.as_bytes())]);
    let wrong_root = zip_package(&[("[Content_Types].xml", types), ("xl/workbook.xml", b"<worksheet/>")]);
    let ods = zip_package(&[
        ("mimetype", b"application/vnd.oasis.opendocument.spreadsheet"),
        ("content.xml", br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"><office:body><office:spreadsheet/></office:body></office:document-content>"#),
    ]);
    let twice = encrypt_workbook(&encrypt_workbook(&no_workbook_part(), "pw"), "pw");
    let cases = [
        (refusal(open_bytes("a.xlsx", &no_workbook_part())), "Không mở được 'a.xlsx': tệp không phải là bảng tính hợp lệ: thiếu phần sổ làm việc"),
        (refusal(open_bytes("b.xlsx", &bad_xml)), "Không mở được 'b.xlsx': tệp chứa XML không hợp lệ: "),
        (
            refusal(open_bytes("c.xlsx", &wrong_root)),
            "Không mở được 'c.xlsx': tệp không phải là bảng tính hợp lệ: phần sổ làm việc có phần tử gốc <worksheet> không hợp lệ",
        ),
        (refusal(open_bytes("d.ods", &ods)), "Không nhập được 'd.ods': tệp không phải là bảng tính hợp lệ: tệp ODS không có trang tính nào"),
        (
            refusal(open_bytes("e.xls", &compound_file(&[("Data", b"x")]))),
            "'e.xls' là tệp nhị phân cũ nhưng không phải sổ làm việc Excel (không có luồng Workbook). Bảng tính không mở được tệp này.",
        ),
        (
            refusal(open_bytes_with_password("f.xlsx", &twice, Some("pw"))),
            "Không mở được 'f.xlsx': tệp mã hóa bị hỏng (bên trong lại là một tệp mã hóa khác).",
        ),
        (
            refusal(open_bytes_with_password("g.xlsx", &encrypt_workbook(&no_workbook_part(), "pw"), Some("nope"))),
            "Mật khẩu không đúng. Hãy kiểm tra phím Caps Lock đã tắt chưa rồi thử lại.",
        ),
        (refusal(open_bytes("h.json", b"{")), "không phải sổ làm việc JSON của Bảng tính: "),
    ];
    for (english, vietnamese) in cases {
        assert!(!english.is_empty(), "the file was refused ({vietnamese})");
        let shown = Language::Vi.tr_message(&english);
        assert!(shown.starts_with(vietnamese), "{english}\n→ {shown}");
    }
}
