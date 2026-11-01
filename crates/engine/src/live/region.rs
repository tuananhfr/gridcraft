//! Region ids of region co-editing (`erpHost` v3): what one lock covers and one payload carries.
//!
//! - `cell:<sheetUid>:<A1>`: one cell's value, formula and style.
//! - `sheet:<sheetUid>`: the order lock of a sheet (rows/columns, sort, filter, multi-cell edits,
//!   anything else confined to that sheet). It clashes with every `cell:<sheetUid>:…`.
//! - `*`: the whole workbook (the sheet list, renames, workbook settings).

use gridcraft_core::CellRef;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Region {
    Cell { sheet: String, cell: CellRef },
    Sheet(String),
    Book,
}

impl Region {
    pub fn id(&self) -> String {
        match self {
            Region::Cell { sheet, cell } => format!("cell:{sheet}:{}", cell.a1()),
            Region::Sheet(sheet) => format!("sheet:{sheet}"),
            Region::Book => "*".into(),
        }
    }

    pub fn parse(id: &str) -> Option<Region> {
        if id == "*" {
            return Some(Region::Book);
        }
        if let Some(uid) = id.strip_prefix("sheet:") {
            return (!uid.is_empty() && !uid.contains(':')).then(|| Region::Sheet(uid.to_string()));
        }
        let (uid, a1) = id.strip_prefix("cell:")?.split_once(':')?;
        let cell = CellRef::parse(a1)?;
        (!uid.is_empty() && cell.is_valid()).then(|| Region::Cell { sheet: uid.to_string(), cell })
    }

    /// The sheet uid the region lies on (`None` for the whole workbook).
    pub fn sheet(&self) -> Option<&str> {
        match self {
            Region::Cell { sheet, .. } | Region::Sheet(sheet) => Some(sheet),
            Region::Book => None,
        }
    }

    /// Two members' locks on these regions can't both be held (the server's rule).
    pub fn clashes(&self, other: &Region) -> bool {
        match (self, other) {
            (Region::Book, _) | (_, Region::Book) => true,
            (Region::Sheet(a), Region::Sheet(b)) => a == b,
            (Region::Sheet(a), Region::Cell { sheet, .. }) | (Region::Cell { sheet, .. }, Region::Sheet(a)) => a == sheet,
            (a, b) => a == b,
        }
    }

    /// A lock on `self` lets its holder send updates of `other`.
    pub fn covers(&self, other: &Region) -> bool {
        match (self, other) {
            (Region::Book, _) => true,
            (Region::Sheet(a), Region::Cell { sheet, .. }) => a == sheet,
            (a, b) => a == b,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_clash_like_the_server() {
        let cell = Region::Cell { sheet: "f1".into(), cell: CellRef::new(4, 1) };
        assert_eq!(cell.id(), "cell:f1:B5");
        assert_eq!(Region::parse("cell:f1:B5"), Some(cell.clone()));
        assert_eq!(Region::parse("sheet:a1b2c3-4"), Some(Region::Sheet("a1b2c3-4".into())));
        assert_eq!(Region::parse("*"), Some(Region::Book));
        for bad in ["", "cell:f1", "cell::A1", "cell:f1:ZZZZ9", "sheet:", "para:1", "sheet:a:b"] {
            assert_eq!(Region::parse(bad), None, "{bad}");
        }
        let sheet = Region::Sheet("f1".into());
        let other_cell = Region::Cell { sheet: "f2".into(), cell: CellRef::new(4, 1) };
        assert!(sheet.clashes(&cell) && cell.clashes(&sheet) && !sheet.clashes(&other_cell));
        assert!(Region::Book.clashes(&other_cell) && cell.clashes(&cell.clone()));
        assert!(!cell.clashes(&Region::Cell { sheet: "f1".into(), cell: CellRef::new(4, 2) }));
        assert!(sheet.covers(&cell) && !cell.covers(&sheet) && Region::Book.covers(&sheet));
    }
}
