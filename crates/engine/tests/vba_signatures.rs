use gridcraft_engine::model::{VbaSignaturePart, VbaSignatures, Workbook};
use gridcraft_engine::{Session, io};
use serde_json::json;
use std::io::{Cursor, Write};

fn legacy() -> Vec<u8> {
    let rec = |id: u16, p: &[u8]| [&id.to_le_bytes()[..], &(p.len() as u16).to_le_bytes(), p].concat();
    let bof = |kind: u16| rec(0x809, &[&0x600u16.to_le_bytes()[..], &kind.to_le_bytes(), &[0; 12]].concat());
    let global = |offset: u32| [bof(5), rec(0x85, &[&offset.to_le_bytes()[..], &[0, 0, 1, 0], b"S"].concat()), rec(10, &[])].concat();
    let stream = [global(global(0).len() as u32), bof(0x10), rec(10, &[])].concat();
    let mut cf = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    cf.create_stream("/Workbook").unwrap().write_all(&stream).unwrap();
    cf.create_stream("/\u{5}DigitalSignature").unwrap().write_all(b"signature").unwrap();
    cf.create_storage_all("/_VBA_PROJECT_CUR/VBA").unwrap();
    cf.create_stream("/_VBA_PROJECT_CUR/PROJECT").unwrap().write_all(b"synthetic").unwrap();
    cf.into_inner().into_inner()
}

#[test]
fn vba_signature_legacy_open_and_every_save_export_report_loss() {
    let mut s = Session::new();
    let result = s.execute("file.open", json!({"name": "old.xls", "base64": io::base64_encode(&legacy())})).unwrap();
    assert!(result["warnings"].as_array().unwrap().iter().any(|w| w == gridcraft_xlsx::LEGACY_SIGNATURE));
    assert!(s.ui_requests.iter().any(|r| matches!(r, gridcraft_engine::UiRequest::Message(m) if m.contains(gridcraft_xlsx::LEGACY_SIGNATURE))));
    for format in ["xlsm", "xltm", "xlsx", "csv", "tsv", "html"] {
        let result = s.execute_headless("file.saveBytes", json!({"format": format})).unwrap();
        let expected = if ["xlsm", "xltm"].contains(&format) { gridcraft_xlsx::LEGACY_SIGNATURE } else { io::REMOVED_SIGNED_MACROS };
        assert_eq!(result["warnings"], json!([expected]), "{format}");
    }
    let json = s.execute_headless("file.saveBytes", json!({"format": "json"})).unwrap();
    assert_eq!(json["warnings"], json!([gridcraft_xlsx::LEGACY_SIGNATURE]));
    let dir = std::env::temp_dir().join(format!("grid03-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (command, ext) in
        [("file.save", "xlsm"), ("file.saveAs", "xltm"), ("file.exportCsv", "csv"), ("file.exportHtml", "html"), ("file.exportPdf", "pdf")]
    {
        let path = dir.join(format!("output.{ext}"));
        let expected = if ["xlsm", "xltm"].contains(&ext) { gridcraft_xlsx::LEGACY_SIGNATURE } else { io::REMOVED_SIGNED_MACROS };
        for _ in 0..2 {
            let result = s.execute_headless(command, json!({"path": path})).unwrap();
            assert_eq!(result["warnings"], json!([expected]), "{command}");
        }
    }
    let wb = &s.doc().unwrap().wb;
    let draft = io::save_xlsx_fast(wb, "old.xlsm").unwrap();
    let back = io::open_bytes("old.xlsm", &draft).unwrap();
    assert!(back.0.legacy_vba_signature);
    assert!(back.1.iter().any(|w| w == gridcraft_xlsx::LEGACY_SIGNATURE));
    assert_eq!(io::save_warnings(&back.0, "xlsm"), io::save_warnings(wb, "xlsm"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn vba_signature_stale_guard_warns_for_save_and_drafts_but_json_keeps_metadata() {
    let mut wb = Workbook::new();
    wb.vba_project = Some(b"changed".to_vec());
    wb.vba_signatures = Some(VbaSignatures {
        project_part: "xl/vbaProject.bin".into(),
        project_sha256: vec![0; 32],
        relationships: b"<Relationships/>".to_vec(),
        parts: vec![VbaSignaturePart {
            name: "xl/vbaProjectSignature.bin".into(),
            relationship_type: "urn:signature".into(),
            content_type: "application/signature".into(),
            data: vec![1, 2, 3],
        }],
    });
    let output = io::save_with_warnings(&wb, "signed.xlsm").unwrap();
    assert_eq!(output.warnings, vec![io::UNSIGNED_MACROS]);
    let mut zip = zip::ZipArchive::new(Cursor::new(output.bytes)).unwrap();
    assert!(zip.by_name("xl/vbaProjectSignature.bin").is_err());
    assert_eq!(io::save_warnings(&wb, "signed.xlsx"), vec![io::REMOVED_SIGNED_MACROS]);
    assert!(io::save_warnings(&wb, "signed.json").is_empty());
    let back = io::open_bytes("draft.xlsm", &io::save_xlsx_fast(&wb, "draft.xlsm").unwrap()).unwrap();
    assert!(back.0.vba_signature_loss);
    assert!(back.1.iter().any(|w| w == io::UNSIGNED_MACROS));
}
