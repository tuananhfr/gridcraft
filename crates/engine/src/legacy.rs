//! Legacy workbook detection and open-time messages shared with frontends.

pub const OPENED: &str = "The legacy .xls file has been opened. When saved, it will be saved as .xlsx.";
pub const FAILED_TITLE: &str = "Could not open the legacy .xls format";
pub const FAILED: &str = "This legacy Excel file (.xls) could not be opened. Open it in Excel or LibreOffice, save it as .xlsx and try again.";

/// Encrypted OOXML also uses OLE2, so it must keep the password workflow.
pub fn is_legacy_workbook(name: &str, bytes: &[u8]) -> bool {
    if gridcraft_xlsx::sniff(bytes) == gridcraft_xlsx::Format::Encrypted {
        return false;
    }
    bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
        || crate::io::FileKind::from_path(name) == Some(crate::io::FileKind::Xls)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_ole2_regardless_of_extension_and_legacy_names() {
        let magic = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        for name in ["book.xls", "book.xlsx", "book.csv", "book"] {
            assert!(is_legacy_workbook(name, &magic));
        }
        assert!(is_legacy_workbook("BOOK.XLS", b""));
        assert!(!is_legacy_workbook("book", &magic[..7]));
    }

    #[test]
    fn excludes_xlsx_zip_csv_and_encrypted_ooxml() {
        let xlsx = crate::io::save_bytes(&gridcraft_model::Workbook::new(), "book.xlsx").unwrap();
        assert!(!is_legacy_workbook("book.xlsx", &xlsx));
        assert!(!is_legacy_workbook("book.csv", b"a,b\n1,2\n"));
        let encrypted = gridcraft_xlsx::testing::encrypt_workbook(&xlsx, "pw");
        assert!(!is_legacy_workbook("secret.xlsx", &encrypted));
    }
}
