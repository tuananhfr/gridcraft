use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::{PivotLayout, PivotTable, Sheet};

use crate::i18n::Language;

pub fn translate<'a>(sheet: &Sheet, cell: CellRef, text: &'a str, language: Language) -> Option<&'a str> {
    if !matches!(text, "Row Labels" | "Column Labels" | "Grand Total" | "Values" | "Total") {
        return None;
    }
    sheet.pivots.iter().any(|pivot| is_caption(pivot, cell, text)).then(|| language.tr(text))
}

fn is_caption(pivot: &PivotTable, cell: CellRef, text: &str) -> bool {
    let Some(range) = pivot.last_range.filter(|range| range.contains(cell) && range.start.col == pivot.anchor.col) else { return false };
    let row = cell.row.saturating_sub(range.start.row) as usize;
    let col = cell.col.saturating_sub(range.start.col) as usize;
    let compact = pivot.layout == PivotLayout::Compact;
    let on_rows = pivot.values_on_rows && pivot.values.len() > 1;
    let row_levels = pivot.rows.len().saturating_add(usize::from(on_rows));
    let label_cols = if row_levels == 0 && pivot.columns.is_empty() { 0 } else if compact { 1 } else { row_levels.max(1) };
    let filters = pivot
        .filters
        .iter()
        .filter(|filter| !pivot.rows.iter().chain(&pivot.columns).any(|field| field.source_col.eq_ignore_ascii_case(&filter.source_col)))
        .count();
    let body = if pivot.anchor.row > range.start.row {
        pivot.anchor.row.saturating_sub(range.start.row) as usize
    } else {
        filters.saturating_add(usize::from(filters > 0))
    };
    let row_header = if pivot.columns.is_empty() {
        body
    } else {
        body.saturating_add(pivot.columns.len()).saturating_add(usize::from(pivot.values.len() > 1 && !on_rows))
    };
    // Coordinates distinguish generated captions from identically named source fields and items.
    match text {
        "Row Labels" => pivot.show_headers && compact && row_levels > 0 && row == row_header && col == 0,
        "Column Labels" => pivot.show_headers && compact && !pivot.columns.is_empty() && row == body && col == label_cols,
        "Values" => {
            pivot.show_headers
                && !compact
                && on_rows
                && row == row_header
                && col == pivot.values_position.unwrap_or(pivot.rows.len()).min(pivot.rows.len())
        }
        "Total" => pivot.columns.is_empty() && on_rows && row == body && col == label_cols,
        "Grand Total" => {
            (pivot.grand_totals_cols && !pivot.rows.is_empty() && !on_rows && cell.row == range.end.row && col == 0)
                || (pivot.grand_totals_rows
                    && !pivot.columns.is_empty()
                    && (pivot.values.len() <= 1 || on_rows)
                    && row == body.saturating_add(1)
                    && cell.col == range.end.col)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use gridcraft_engine::{Session, core::Value};
    use serde_json::json;

    use super::*;

    #[test]
    fn captions_follow_language_and_leave_source_items_and_saved_text_unchanged() {
        let mut session = Session::new();
        session.new_workbook();
        session
            .execute(
                "range.setValues",
                json!({"range": "A1", "values": [["Region", "Product", "Sales"], ["Grand Total", "Column Labels", 10], ["Row Labels", "Values", 20]]}),
            )
            .unwrap();
        session.execute("selection.set", json!({"range": "A1"})).unwrap();
        session.execute("insert.pivotTable", json!({"rows": ["Region"], "columns": ["Product"], "values": ["Sales"]})).unwrap();
        let wb = &session.doc().unwrap().wb;
        let sheet = wb.active().unwrap();
        let before = gridcraft_engine::io::save_bytes(wb, "book.xlsx").unwrap();
        let mut translated = Vec::new();
        for (cell, value) in sheet.cells.iter() {
            let Value::Text(text) = &value.value else { continue };
            if let Some(shown) = translate(sheet, cell, text, Language::Vi) {
                translated.push(shown);
                assert_eq!(translate(sheet, cell, text, Language::En), Some(text.as_ref()));
            }
        }
        assert_eq!(translated.iter().filter(|text| **text == "Tổng cộng").count(), 2);
        assert!(translated.contains(&"Nhãn hàng") && translated.contains(&"Nhãn cột"));
        assert_eq!(translated.len(), 4, "source items with the same names must not be translated");
        assert_eq!(gridcraft_engine::io::save_bytes(wb, "book.xlsx").unwrap(), before);
    }

    #[test]
    fn values_and_total_captions_translate_in_tabular_layout_with_filters() {
        let mut session = Session::new();
        session.new_workbook();
        session
            .execute(
                "range.setValues",
                json!({"range": "A1", "values": [["Region", "Product", "Sales", "Units"], ["East", "Pears", 10, 2], ["West", "Apples", 20, 3]]}),
            )
            .unwrap();
        session.execute("selection.set", json!({"range": "A1"})).unwrap();
        session.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales", "Units"]})).unwrap();
        session.execute("pivot.addField", json!({"field": "Product", "area": "filters"})).unwrap();
        session.execute("pivot.moveField", json!({"field": "Σ Values", "from": "columns", "to": "rows"})).unwrap();
        session.execute("pivot.layout", json!({"layout": "tabular"})).unwrap();
        let sheet = session.doc().unwrap().wb.active().unwrap();
        let translations: Vec<_> = sheet
            .cells
            .iter()
            .filter_map(|(cell, value)| match &value.value {
                Value::Text(text) => translate(sheet, cell, text, Language::Vi),
                _ => None,
            })
            .collect();
        assert!(translations.contains(&"Giá trị") && translations.contains(&"Tổng cộng"), "{translations:?}");
        assert_eq!(translations.len(), 2);
    }
}
