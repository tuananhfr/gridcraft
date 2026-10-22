use super::{make_zip, minimal};
use crate::package::Package;
use crate::{XlsxKind, read_xlsx, write_xlsx_as};
use gridcraft_model::Workbook;

fn signed(variant: &str, year: &str, extra: &str) -> (Vec<u8>, Vec<u8>) {
    let rels = format!(
        r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="sig1" Type="http://schemas.microsoft.com/office/{year}/relationships/{variant}" Target="{variant}.bin"/>{extra}</Relationships>"#
    );
    let package = minimal(
        "<sheetData/>",
        &[],
        r#"<Relationship Id="vba" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/>"#,
        "",
    );
    let mut zip = Package::open(&package).unwrap();
    let types = format!(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/{variant}.bin" ContentType="application/vnd.ms-office.{variant}"/></Types>"#
    );
    let mut parts = vec![("[Content_Types].xml".to_string(), types.into_bytes())];
    for name in ["_rels/.rels", "xl/workbook.xml", "xl/_rels/workbook.xml.rels", "xl/worksheets/sheet1.xml"] {
        parts.push((name.to_string(), zip.read(name).unwrap().unwrap()));
    }
    parts.push(("xl/vbaProject.bin".into(), b"synthetic project".to_vec()));
    parts.push(("xl/_rels/vbaProject.bin.rels".into(), rels.as_bytes().to_vec()));
    parts.push((format!("xl/{variant}.bin"), vec![0, 255, 128, 42]));
    (make_zip(&parts.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect::<Vec<_>>()), rels.into_bytes())
}

#[test]
fn vba_signature_variants_keep_identical_parts_rels_and_content_types() {
    for (variant, year) in [("vbaProjectSignature", "2006"), ("vbaProjectSignatureAgile", "2014"), ("vbaProjectSignatureV3", "2020/07")] {
        let (bytes, rels) = signed(variant, year, "");
        let (wb, _) = read_xlsx(&bytes).unwrap();
        let sig = wb.vba_signatures.as_ref().unwrap();
        assert!(sig.matches(wb.vba_project.as_deref()));
        assert_eq!(sig.parts[0].relationship_type, format!("http://schemas.microsoft.com/office/{year}/relationships/{variant}"));
        for kind in [XlsxKind::MacroWorkbook, XlsxKind::MacroTemplate] {
            let saved = write_xlsx_as(&wb, kind).unwrap();
            let mut zip = Package::open(&saved).unwrap();
            assert_eq!(zip.read(&format!("xl/{variant}.bin")).unwrap().unwrap(), [0, 255, 128, 42]);
            assert_eq!(zip.read("xl/_rels/vbaProject.bin.rels").unwrap().unwrap(), rels);
            let types = zip.read_xml("[Content_Types].xml").unwrap().unwrap();
            assert!(types.kids("Override").any(|e| e.attr("PartName") == Some(format!("/xl/{variant}.bin").as_str())
                && e.attr("ContentType") == Some(format!("application/vnd.ms-office.{variant}").as_str())));
            assert_eq!(read_xlsx(&saved).unwrap().0.vba_signatures, wb.vba_signatures);
        }
    }
}

#[test]
fn vba_signature_is_never_written_after_project_change_or_to_xlsx() {
    let (bytes, _) = signed("vbaProjectSignature", "2006", "");
    let (mut wb, _) = read_xlsx(&bytes).unwrap();
    let plain = write_xlsx_as(&wb, XlsxKind::Workbook).unwrap();
    assert!(!Package::open(&plain).unwrap().has("xl/vbaProjectSignature.bin"));
    wb.vba_project = Some(b"changed".to_vec());
    assert!(!wb.vba_signatures.as_ref().unwrap().matches(wb.vba_project.as_deref()));
    let changed = write_xlsx_as(&wb, XlsxKind::MacroWorkbook).unwrap();
    let mut zip = Package::open(&changed).unwrap();
    assert!(!zip.has("xl/vbaProjectSignature.bin"));
    assert!(!zip.has("xl/_rels/vbaProject.bin.rels"));
    assert_eq!(zip.read("xl/vbaProject.bin").unwrap().unwrap(), b"changed");
}

#[test]
fn vba_signature_unknown_leaf_relationship_is_kept_and_external_is_warned() {
    let (bytes, _) = signed(
        "customSignature",
        "2099",
        r#"<Relationship Id="external" Type="urn:unknown" Target="https://example.invalid/part" TargetMode="External"/>"#,
    );
    let (wb, report) = read_xlsx(&bytes).unwrap();
    assert!(report.warnings.iter().any(|w| w == crate::DROPPED_SIGNATURE));
    assert!(wb.vba_signature_loss);
    assert_eq!(wb.vba_signatures.as_ref().unwrap().parts.len(), 1);
    let bytes = write_xlsx_as(&wb, XlsxKind::MacroWorkbook).unwrap();
    let mut zip = Package::open(&bytes).unwrap();
    assert!(zip.has("xl/customSignature.bin"));
    let rels = zip.read_xml("xl/_rels/vbaProject.bin.rels").unwrap().unwrap();
    assert_eq!(rels.kids("Relationship").count(), 1);
}

#[test]
fn vba_signature_empty_workbook_has_no_signature_metadata() {
    let bytes = write_xlsx_as(&Workbook::new(), XlsxKind::Workbook).unwrap();
    assert!(read_xlsx(&bytes).unwrap().0.vba_signatures.is_none());
}
