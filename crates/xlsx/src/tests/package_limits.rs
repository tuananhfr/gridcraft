//! Decompression limits: ratio, per-entry and package caps enforced on what inflates, not on
//! what the zip headers declare.

use super::{make_zip, minimal};
use crate::limits::{LIMITS, Limits};
use crate::package::Package;
use crate::{IoError, read_xlsx, write_xlsx};

const MIB: usize = 1024 * 1024;

/// Bytes that deflate barely compresses (xorshift), so only the size caps can stop them.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// Rewrites the uncompressed size `name` declares, in its local header and the central directory.
fn declare_size(zip: &mut [u8], name: &str, size: u32) {
    let mut patched = 0;
    for i in 0..zip.len().saturating_sub(46 + name.len()) {
        let (name_at, size_at) = match zip[i..i + 4] {
            [0x50, 0x4b, 0x03, 0x04] => (30, 22),
            [0x50, 0x4b, 0x01, 0x02] => (46, 24),
            _ => continue,
        };
        if zip.get(i + name_at..i + name_at + name.len()) == Some(name.as_bytes()) {
            zip[i + size_at..i + size_at + 4].copy_from_slice(&size.to_le_bytes());
            patched += 1;
        }
    }
    assert_eq!(patched, 2, "{name} not found in both headers");
}

fn small(entry: usize, total: usize) -> Limits {
    Limits { entry: entry as u64, total: total as u64, entries: 100, ratio: LIMITS.ratio, ratio_floor: LIMITS.ratio_floor }
}

fn refused(r: Result<Option<Vec<u8>>, IoError>) -> String {
    match r {
        Err(IoError::TooLarge(m)) => m,
        other => panic!("expected a refusal, got {:?}", other.map(|o| o.map(|b| b.len()))),
    }
}

#[test]
fn a_deflate_bomb_is_refused_by_its_ratio() {
    let bomb = format!("{}<sheetData/>", " ".repeat(64 * MIB));
    let bytes = minimal(&bomb, &[], "", "");
    assert!(bytes.len() < MIB, "the bomb is {} bytes", bytes.len());
    match read_xlsx(&bytes) {
        Err(IoError::TooLarge(m)) => assert!(m.contains("compressed more than 200:1"), "{m}"),
        other => panic!("expected a refusal, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn a_declared_small_entry_is_cut_off_while_streaming() {
    // Ratio bomb declaring 100 bytes: the declared size passes, the stream does not.
    let mut bytes = make_zip(&[("bomb.xml", &vec![b' '; 32 * MIB])]);
    declare_size(&mut bytes, "bomb.xml", 100);
    let mut p = Package::open(&bytes).unwrap();
    assert!(refused(p.read("bomb.xml")).contains("compressed more than"));
    // Incompressible data declaring 100 bytes stops at the entry cap.
    let mut bytes = make_zip(&[("big.bin", &noise(3 * MIB, 7)), ("small.xml", b"<a/>")]);
    declare_size(&mut bytes, "big.bin", 100);
    let mut p = Package::open_with(&bytes, small(MIB, 64 * MIB)).unwrap();
    assert!(refused(p.read("big.bin")).contains("expands to more than 1048576 bytes"));
    // The refusal sticks for the rest of the package.
    assert!(p.refusal().is_some());
    assert!(matches!(p.read("small.xml"), Err(IoError::TooLarge(_))));
}

#[test]
fn a_caller_limit_is_not_a_refusal_but_the_entry_cap_is() {
    let mut bytes = make_zip(&[("meta.xml", &noise(2 * MIB, 3)), ("big.bin", &noise(3 * MIB, 5))]);
    // Declared over the entry cap, the package is refused before anything is inflated.
    let mut p = Package::open_with(&bytes, small(2 * MIB + 1, 64 * MIB)).unwrap();
    assert!(refused(p.read("big.bin")).contains("declares"));
    assert!(p.refusal().is_some());
    // Declared over only a caller's limit, the part fails on its own.
    let mut p = Package::open_with(&bytes, small(2 * MIB + 1, 64 * MIB)).unwrap();
    assert!(matches!(p.read_limited("meta.xml", 1024), Err(IoError::TooLarge(_))));
    assert!(p.refusal().is_none());
    declare_size(&mut bytes, "big.bin", 100);
    let mut p = Package::open_with(&bytes, small(2 * MIB + 1, 64 * MIB)).unwrap();
    assert!(matches!(p.read_limited("meta.xml", 1024), Err(IoError::TooLarge(_))));
    assert!(p.refusal().is_none(), "a metadata part over its own limit is the caller's to skip");
    assert_eq!(p.read("meta.xml").unwrap().unwrap().len(), 2 * MIB);
    refused(p.read("big.bin"));
    assert!(p.refusal().is_some());
}

#[test]
fn a_required_part_declared_over_the_cap_is_refused_as_too_large() {
    // Honest headers on a part too large for this build: not a workbook "with no workbook part".
    let over = u32::try_from(LIMITS.entry + 1).unwrap();
    for part in ["[Content_Types].xml", "xl/workbook.xml"] {
        let mut bytes = minimal("<sheetData/>", &[], "", "");
        assert!(read_xlsx(&bytes).is_ok());
        declare_size(&mut bytes, part, over);
        match read_xlsx(&bytes) {
            Err(IoError::TooLarge(m)) => assert!(m.contains("declares"), "{part}: {m}"),
            Err(e) => panic!("{part}: refused for another reason: {e}"),
            Ok(_) => panic!("{part}: opened"),
        }
    }
}

#[test]
fn the_package_total_is_capped() {
    let a = noise(2 * MIB, 11);
    let b = noise(2 * MIB, 13);
    let mut bytes = make_zip(&[("a.bin", &a), ("b.bin", &b), ("c.xml", b"<c/>")]);
    declare_size(&mut bytes, "b.bin", 100);
    let mut p = Package::open_with(&bytes, small(4 * MIB, 3 * MIB)).unwrap();
    assert_eq!(p.read("a.bin").unwrap().unwrap(), a);
    assert!(refused(p.read("b.bin")).contains("package expands to more than"));
    assert!(matches!(p.read("c.xml"), Err(IoError::TooLarge(_))));
}

#[test]
fn too_many_entries_are_refused() {
    let names: Vec<String> = (0..101).map(|i| format!("p{i}.xml")).collect();
    let parts: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"<a/>"[..])).collect();
    let bytes = make_zip(&parts);
    assert!(matches!(Package::open_with(&bytes, small(MIB, MIB)), Err(IoError::TooLarge(_))));
    assert!(Package::open_with(&bytes[..], Limits { entries: 101, ..small(MIB, MIB) }).is_ok());
}

#[test]
fn a_bomb_in_an_optional_part_refuses_the_whole_workbook() {
    // Kept custom XML is optional: unreadable, it would just be dropped with a warning.
    let rel = r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/>"#;
    let item = format!("<root>{}</root>", " ".repeat(16 * MIB));
    let bytes = minimal("<sheetData/>", &[("customXml/item1.xml", &item)], rel, "");
    assert!(matches!(read_xlsx(&bytes), Err(IoError::TooLarge(_))));
    // The same part, small enough to be kept, opens.
    let bytes = minimal("<sheetData/>", &[("customXml/item1.xml", "<root/>")], rel, "");
    assert!(read_xlsx(&bytes).is_ok());
}

#[test]
fn a_small_highly_compressed_part_is_under_the_ratio_floor() {
    let sheet = format!("{}<sheetData/>", " ".repeat(LIMITS.ratio_floor as usize - 4096));
    assert!(read_xlsx(&minimal(&sheet, &[], "", "")).is_ok());
}

#[test]
fn a_large_legitimate_workbook_still_opens() {
    use gridcraft_core::{CellRef, Value};
    use gridcraft_model::Workbook;
    let mut wb = Workbook::new();
    let sheet = std::sync::Arc::make_mut(&mut wb.sheets[0]);
    for r in 0..50_000u32 {
        for c in 0..6u32 {
            let v = if c % 3 == 2 { Value::text(format!("Khách hàng {}", r % 977)) } else { Value::Number(f64::from(r * 7 + c)) };
            sheet.set_value(CellRef::new(r, c), v);
        }
    }
    let bytes = write_xlsx(&wb).unwrap();
    let (back, _) = read_xlsx(&bytes).unwrap();
    assert_eq!(back.sheets[0].value(CellRef::new(49_999, 1)), Value::Number(f64::from(49_999 * 7 + 1)));
    // A sheet of identical styled rows, more repetitive than real data, stays under the ratio.
    let rows: String = (1..=300_000).map(|r| format!(r#"<row r="{r}" s="1" customFormat="1" ht="15" customHeight="1"/>"#)).collect();
    let bytes = minimal(&format!("<sheetData>{rows}</sheetData>"), &[], "", "");
    assert!(read_xlsx(&bytes).is_ok());
}
