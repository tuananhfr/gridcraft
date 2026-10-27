//! What logs may say about the user's files: a document only by its extension and size, a
//! settings, font or log folder with the home folder shown as `~`. Logs never carry document
//! names, paths or URLs.

use std::path::{Path, PathBuf};

/// A document as logs name it, such as `.xlsx (12345 bytes)`, or `.xlsx` when the size is unknown.
pub fn file_label(name: &str, size: Option<usize>) -> String {
    // A long or odd "extension" is part of a name (or a URL's query), not a file type.
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty() && e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()));
    let kind = ext.map_or_else(|| "file".to_string(), |e| format!(".{}", e.to_ascii_lowercase()));
    match size {
        Some(n) => format!("{kind} ({n} bytes)"),
        None => kind,
    }
}

/// `path` with the user's home folder shown as `~`, which hides the account name it contains.
pub fn home_relative(path: &Path) -> String {
    home_relative_to(path, home_dir().as_deref())
}

fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).filter(|h| !h.is_empty()).map(PathBuf::from)
}

fn home_relative_to(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_is_named_by_its_extension_and_size_only() {
        assert_eq!(file_label("Báo cáo quý 3.XLSX", Some(12345)), ".xlsx (12345 bytes)");
        assert_eq!(file_label("/home/an/Tài liệu/budget.xlsm", None), ".xlsm");
        assert_eq!(file_label("C:\\Users\\an\\Desktop\\Book1.csv", Some(0)), ".csv (0 bytes)");
        assert_eq!(file_label("no extension", Some(7)), "file (7 bytes)");
        assert_eq!(file_label("https://example.com/f.xlsx?token=secret", Some(3)), "file (3 bytes)");
        assert_eq!(file_label("report.final version", None), "file");
        assert_eq!(file_label("", None), "file");
    }

    #[test]
    fn the_home_folder_shows_as_a_tilde() {
        let home = Path::new("/home/an");
        let shown = home_relative_to(Path::new("/home/an/.config/ERPCons/BangTinh/gridcraft.log"), Some(home));
        assert_eq!(shown, format!("~{0}.config{0}ERPCons{0}BangTinh{0}gridcraft.log", std::path::MAIN_SEPARATOR));
        assert_eq!(home_relative_to(home, Some(home)), "~");
        assert_eq!(home_relative_to(Path::new("/usr/share/fonts/a.ttf"), Some(home)), "/usr/share/fonts/a.ttf");
        assert_eq!(home_relative_to(Path::new("/home/anna/x"), Some(home)), "/home/anna/x", "only whole components match");
        assert_eq!(home_relative_to(Path::new("/home/an/x"), None), "/home/an/x");
    }
}
