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
