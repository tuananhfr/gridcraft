//! Hostile packages (zip bombs, too many entries) are refused with one translatable message and
//! open nothing; large legitimate workbooks still open.
use std::io::Write;

use gridcraft_engine::io::{FILE_TOO_LARGE, base64_encode, open_bytes, open_bytes_with_password, save_bytes};
use gridcraft_engine::{Session, core::CellRef, core::Value};
use gridcraft_xlsx::limits::{self, LIMITS};
use serde_json::json;

const CT: &str = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/></Types>"#;
const ROOT_RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
const WORKBOOK: &str = r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
const SHEET_REL: &str = r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#;
const CUSTOM_XML_REL: &str = r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/>"#;

fn zip(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in parts {
        z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// A one-sheet package; `sheet_lead` goes before `<sheetData/>`, `wb_rels` after the sheet's.
fn package(sheet_lead: &str, wb_rels: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let sheet = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{sheet_lead}<sheetData><row r="1"><c r="A1"><v>7</v></c></row></sheetData></worksheet>"#
    );
    let rels = format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{SHEET_REL}{wb_rels}</Relationships>"#);
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", CT.as_bytes()),
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("xl/workbook.xml", WORKBOOK.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
    ];
    parts.extend_from_slice(extra);
    zip(&parts)
}

fn bomb() -> Vec<u8> {
    package(&" ".repeat(64 << 20), "", &[])
}

fn refusal(r: gridcraft_engine::Result<(gridcraft_engine::model::Workbook, Vec<String>)>) -> String {
    match r {
        Ok(_) => panic!("the hostile package opened"),
        Err(e) => e.to_string(),
    }
}

/// Rewrites the inflated size `name` declares, in its local header and the central directory.
fn declare_size(zip: &mut [u8], name: &str, size: u32) {
    for i in 0..zip.len().saturating_sub(4) {
        let (name_at, size_at) = match zip[i..i + 4] {
            [0x50, 0x4b, 0x03, 0x04] => (30, 22),
            [0x50, 0x4b, 0x01, 0x02] => (46, 24),
            _ => continue,
        };
        if zip.get(i + name_at..i + name_at + name.len()) == Some(name.as_bytes()) {
            zip[i + size_at..i + size_at + 4].copy_from_slice(&size.to_le_bytes());
        }
    }
}

#[test]
fn a_required_part_declared_over_the_cap_says_too_large_not_corrupt() {
    let over = u32::try_from(LIMITS.entry + 1).unwrap();
    for (name, part) in [("bomb.xlsx", "[Content_Types].xml"), ("bomb.xlsx", "xl/workbook.xml"), ("sheet.xlsx", "xl/worksheets/sheet1.xml")] {
        let mut bytes = package("", "", &[]);
        declare_size(&mut bytes, part, over);
        assert_eq!(refusal(open_bytes(name, &bytes)), FILE_TOO_LARGE, "{part}");
    }
}

#[test]
fn a_zip_bomb_is_refused_with_the_translatable_message_and_opens_nothing() {
    let bytes = bomb();
    assert!(bytes.len() < 1 << 20);
    assert_eq!(refusal(open_bytes("bomb.xlsx", &bytes)), FILE_TOO_LARGE);
    let mut s = Session::new();
    s.new_workbook();
    let before = s.documents().len();
    let err = s.run("file.open", json!({"name": "bomb.xlsx", "base64": base64_encode(&bytes)})).unwrap_err();
    assert!(err.ends_with(FILE_TOO_LARGE), "{err}");
    assert_eq!(s.documents().len(), before, "no document, not even a partial one");
}

#[test]
fn a_bomb_in_an_optional_part_still_refuses_the_file() {
    let item = format!("<root>{}</root>", " ".repeat(16 << 20));
    let bytes = package("", CUSTOM_XML_REL, &[("customXml/item1.xml", item.as_bytes())]);
    assert_eq!(refusal(open_bytes("custom.xlsx", &bytes)), FILE_TOO_LARGE);
}

#[test]
fn a_bomb_nested_inside_an_encrypted_package_is_refused() {
    let encrypted = gridcraft_xlsx::testing::encrypt_workbook(&bomb(), "pw");
    assert_eq!(refusal(open_bytes_with_password("secret.xlsx", &encrypted, Some("pw"))), FILE_TOO_LARGE);
}

#[test]
fn a_package_nested_in_a_part_is_never_inflated_and_its_layers_are_checked() {
    // Zip-in-zip with the inner bomb stored: the outer entry is what inflates, and is refused.
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    z.start_file("bomb.xml", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
    z.write_all(&vec![b' '; 32 << 20]).unwrap();
    let stored_inner = z.finish().unwrap().into_inner();
    let bytes = package("", CUSTOM_XML_REL, &[("customXml/item1.xml", &stored_inner)]);
    assert_eq!(refusal(open_bytes("nested.xlsx", &bytes)), FILE_TOO_LARGE);
    // The inner bomb deflated: kept as opaque bytes, never opened, so the workbook opens.
    let bytes = package("", CUSTOM_XML_REL, &[("customXml/item1.xml", &bomb())]);
    let (wb, _) = open_bytes("nested.xlsx", &bytes).unwrap();
    assert_eq!(wb.sheets[0].value(CellRef::parse("A1").unwrap()), Value::Number(7.0));
}

#[test]
fn too_many_zip_entries_are_refused() {
    let names: Vec<String> = (0..=LIMITS.entries).map(|i| format!("customXml/i{i}.xml")).collect();
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for name in &names {
        z.start_file(name.as_str(), stored).unwrap();
    }
    for (name, data) in [("[Content_Types].xml", CT), ("_rels/.rels", ROOT_RELS), ("xl/workbook.xml", WORKBOOK)] {
        z.start_file(name, stored).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    let bytes = z.finish().unwrap().into_inner();
    assert_eq!(refusal(open_bytes("many.xlsx", &bytes)), FILE_TOO_LARGE);
}

#[test]
fn a_large_legitimate_workbook_still_opens() {
    let mut s = Session::new();
    s.new_workbook();
    let rows = 50_000;
    let values: Vec<Vec<serde_json::Value>> =
        (0..rows).map(|r| vec![json!(r), json!(format!("Khách hàng {}", r % 977)), json!(r as f64 * 1.5), json!("Hà Nội")]).collect();
    s.run("range.setValues", json!({"range": format!("A1:D{rows}"), "values": values})).unwrap();
    let bytes = save_bytes(&s.active().unwrap().wb, "big.xlsx").unwrap();
    let (wb, _) = open_bytes("big.xlsx", &bytes).unwrap();
    let last = CellRef::parse(&format!("C{rows}")).unwrap();
    assert_eq!(wb.sheets[0].value(last), Value::Number((rows - 1) as f64 * 1.5));
    // A highly repetitive but real sheet: 300,000 formatted empty rows.
    let rows: String = (1..=300_000).map(|r| format!(r#"<row r="{r}" s="0" ht="15" customHeight="1"/>"#)).collect();
    let sheet = format!(r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#);
    let rels = format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{SHEET_REL}</Relationships>"#);
    let bytes = zip(&[
        ("[Content_Types].xml", CT.as_bytes()),
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("xl/workbook.xml", WORKBOOK.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
    ]);
    assert!(open_bytes("rows.xlsx", &bytes).is_ok());
}

#[test]
fn the_web_build_selects_the_smaller_limits() {
    assert_eq!(limits::for_target(true), limits::WEB);
    assert_eq!(limits::for_target(false), limits::NATIVE);
    assert_eq!(LIMITS, limits::for_target(cfg!(target_arch = "wasm32")));
    assert_eq!(limits::WEB.entry, 256 << 20);
    assert_eq!(limits::WEB.total, 512 << 20);
    assert_eq!(limits::NATIVE.entry, 512 << 20);
    assert_eq!(limits::NATIVE.total, 1 << 30);
}
