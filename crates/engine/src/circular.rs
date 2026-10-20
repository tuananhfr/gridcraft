use gridcraft_core::{CellError, Value};
use gridcraft_model::Workbook;

use crate::DocState;

pub fn has_circular_reference(wb: &Workbook) -> bool {
    wb.sheets.iter().any(|sheet| sheet.cells.iter().any(|(_, cell)| cell.formula.is_some() && cell.value == Value::Error(CellError::Circ)))
}

pub fn created_by_edit(before: &Workbook, after: &DocState) -> bool {
    after.calc.circular.iter().any(|(si, c)| {
        let Some(cell) = after.wb.sheet(*si).and_then(|sheet| sheet.cell(*c)) else { return false };
        if cell.formula.is_none() || cell.value != Value::Error(CellError::Circ) {
            return false;
        }
        let old = before.sheet(*si).and_then(|sheet| sheet.cell(*c));
        old.is_none_or(|old| old.value != cell.value || old.formula.as_ref().map(|f| &f.text) != cell.formula.as_ref().map(|f| &f.text))
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Session;

    #[test]
    fn detects_a_new_cycle_without_changing_its_error() {
        let mut session = Session::new();
        session.new_workbook();
        let before = session.doc().unwrap().wb.clone();
        session.execute("cell.set", json!({"cell": "A1", "input": "=A1"})).unwrap();
        let doc = session.doc().unwrap();
        assert!(created_by_edit(&before, doc));
        assert!(has_circular_reference(&doc.wb));
        assert_eq!(doc.wb.active().unwrap().value(gridcraft_core::CellRef::new(0, 0)).display(), "#CIRC!");
        assert!(!created_by_edit(&doc.wb, doc), "recalculation alone is not a new edit");
        session.execute("cell.set", json!({"cell": "A1", "input": "=1+2"})).unwrap();
        assert!(!has_circular_reference(&session.doc().unwrap().wb));
    }

    #[test]
    fn indirect_cycles_notify_but_unrelated_edits_do_not() {
        let mut session = Session::new();
        session.new_workbook();
        session.execute("cell.set", json!({"cell": "A1", "input": "=B1"})).unwrap();
        let before = session.doc().unwrap().wb.clone();
        session.execute("cell.set", json!({"cell": "B1", "input": "=A1"})).unwrap();
        assert!(created_by_edit(&before, session.doc().unwrap()));
        let before = session.doc().unwrap().wb.clone();
        session.execute("cell.set", json!({"cell": "C1", "input": "=1+2"})).unwrap();
        assert!(!created_by_edit(&before, session.doc().unwrap()));
        assert!(has_circular_reference(&session.doc().unwrap().wb), "an unrelated edit must not clear existing circular errors");
    }
}
