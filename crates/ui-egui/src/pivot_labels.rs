//! PivotTable date-group labels in the regional format, at display only: the engine writes
//! `Qtr1`, `Jan` and `4-Jan` into the report cells, and those stay in the cells and the saved file
//! (as Excel keeps the labels it wrote). vi-VN shows them as `Quý 1`, `Thg1`, `4-Thg1`: the
//! months as Windows' vi-VN short names, which Excel's grouping uses.

use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::{PivotDateGroup, Sheet};

use crate::region::Region;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Whether `c` lies in the report of a PivotTable that groups a field by date.
pub fn in_date_grouped_pivot(sheet: &Sheet, c: CellRef) -> bool {
    sheet
        .pivots
        .iter()
        .any(|p| p.last_range.is_some_and(|r| r.contains(c)) && p.rows.iter().chain(&p.columns).any(|f| f.date_group != PivotDateGroup::None))
}

/// `label` as `region` shows it, when it is a quarter, month or day group label.
pub fn translate(label: &str, region: Region) -> Option<String> {
    if region != Region::ViVn {
        return None;
    }
    if let Some(q) = label.strip_prefix("Qtr").filter(|q| matches!(*q, "1" | "2" | "3" | "4")) {
        return Some(format!("Quý {q}"));
    }
    let month = |name: &str| MONTHS.iter().position(|m| *m == name).map(|i| format!("Thg{}", i + 1));
    if let Some(m) = month(label) {
        return Some(m);
    }
    let (day, name) = label.split_once('-')?;
    let day: u32 = day.parse().ok().filter(|d| (1..=31).contains(d))?;
    Some(format!("{day}-{}", month(name)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_month_and_day_labels_in_vietnamese() {
        let vi = |s| translate(s, Region::ViVn);
        assert_eq!(vi("Qtr1").as_deref(), Some("Quý 1"));
        assert_eq!(vi("Qtr4").as_deref(), Some("Quý 4"));
        assert_eq!(vi("Jan").as_deref(), Some("Thg1"));
        assert_eq!(vi("Dec").as_deref(), Some("Thg12"));
        assert_eq!(vi("4-Jan").as_deref(), Some("4-Thg1"));
        assert_eq!(vi("31-Oct").as_deref(), Some("31-Thg10"));
        for other in ["Qtr5", "Qtr", "January", "2026", "North", "0-Jan", "x-Jan", "4-Foo", "Grand Total", "Qtr1 Total"] {
            assert_eq!(vi(other), None, "{other}");
        }
        assert_eq!(translate("Qtr1", Region::EnUs), None);
    }
}
