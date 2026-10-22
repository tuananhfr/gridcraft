use gridcraft_calc::Calc;
use gridcraft_core::{CellError, CellRef, RangeRef, Value};
use gridcraft_model::{CalcMode, Cell, Formula, Sheet, Workbook};
use std::sync::Arc;

fn set(wb: &mut Workbook, sheet: usize, at: &str, input: &str) {
    let cell = match input.strip_prefix('=') {
        Some(formula) => Cell::formula(Formula::new(formula)),
        None => Cell::value(Value::Number(input.parse().unwrap())),
    };
    wb.sheet_mut(sheet).unwrap().cells.set(CellRef::parse(at).unwrap(), cell);
}

#[test]
fn selection_reads_spill_precedent_and_leaves_other_dependents_cached() {
    let mut wb = Workbook::new();
    wb.sheets.push(Arc::new(Sheet::new("Other")));
    set(&mut wb, 1, "B1", "10");
    set(&mut wb, 1, "A2", "=SEQUENCE(2,1,B1)");
    set(&mut wb, 1, "C1", "=B1*2");
    set(&mut wb, 0, "D1", "=Other!A3+1");
    let mut calc = Calc::new();
    calc.recalc_all(&mut wb);
    wb.calc.mode = CalcMode::Manual;
    set(&mut wb, 1, "B1", "30");
    calc.cells_changed(&mut wb, &[(1, CellRef::parse("B1").unwrap())]);
    calc.recalc_selection(&mut wb, 0, &[RangeRef::parse("D1").unwrap()]);
    assert_eq!(wb.sheet(0).unwrap().value(CellRef::parse("D1").unwrap()), Value::Number(32.0));
    assert_eq!(wb.sheet(1).unwrap().value(CellRef::parse("C1").unwrap()), Value::Number(20.0));
    assert!(calc.stale.contains(&(1, CellRef::parse("C1").unwrap())));
}

#[test]
fn selection_cannot_overwrite_an_unselected_spill() {
    let mut wb = Workbook::new();
    set(&mut wb, 0, "E1", "=SEQUENCE(3)");
    let mut calc = Calc::new();
    calc.recalc_all(&mut wb);
    wb.calc.mode = CalcMode::Manual;
    set(&mut wb, 0, "A2", "=SEQUENCE(1,5)");
    set(&mut wb, 0, "A5", "=SUM(A2:E2)");
    calc.cells_changed(&mut wb, &[(0, CellRef::parse("A2").unwrap()), (0, CellRef::parse("A5").unwrap())]);
    calc.recalc_selection(&mut wb, 0, &[RangeRef::parse("A5").unwrap()]);
    assert_eq!(wb.sheet(0).unwrap().value(CellRef::parse("A2").unwrap()), Value::Error(CellError::Spill));
    assert_eq!(wb.sheet(0).unwrap().value(CellRef::parse("E2").unwrap()), Value::Number(2.0));
}

#[test]
fn selection_recalculates_spill_precedent_outside_aggregate_range() {
    let mut wb = Workbook::new();
    set(&mut wb, 0, "B1", "10");
    set(&mut wb, 0, "A2", "=SEQUENCE(2,1,B1)");
    set(&mut wb, 0, "D1", "=SUM(A3:A4)");
    let mut calc = Calc::new();
    calc.recalc_all(&mut wb);
    wb.calc.mode = CalcMode::Manual;
    set(&mut wb, 0, "B1", "30");
    calc.cells_changed(&mut wb, &[(0, CellRef::parse("B1").unwrap())]);
    calc.recalc_selection(&mut wb, 0, &[RangeRef::parse("D1").unwrap()]);
    assert_eq!(wb.sheet(0).unwrap().value(CellRef::parse("D1").unwrap()), Value::Number(31.0));
}
