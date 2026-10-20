//! How a regional format displays formatted values: the characters a code's `.` and `,` stand
//! for, calendar names, and the built-in formats whose look follows the region. Spanish keeps the
//! en-US display it had before regions existed (see `Region::EsEs`).

use gridcraft_core::Region;
use gridcraft_core::date::{MONTHS, WEEKDAYS};

/// Short month names: CLDR `vi` abbreviated (format context). Windows NLS spells them
/// `Thg1`…`Thg12`.
const VI_MONTHS_SHORT: [&str; 12] = ["thg 1", "thg 2", "thg 3", "thg 4", "thg 5", "thg 6", "thg 7", "thg 8", "thg 9", "thg 10", "thg 11", "thg 12"];
/// Long month names: CLDR `vi` wide (format context, lowercase as inside a date). Windows NLS has
/// the traditional `Tháng Giêng`…`Tháng Mười Hai` instead.
const VI_MONTHS_LONG: [&str; 12] =
    ["tháng 1", "tháng 2", "tháng 3", "tháng 4", "tháng 5", "tháng 6", "tháng 7", "tháng 8", "tháng 9", "tháng 10", "tháng 11", "tháng 12"];
/// Short day names, Sunday first: CLDR `vi` short and Windows NLS abbreviated agree.
const VI_DAYS_SHORT: [&str; 7] = ["CN", "T2", "T3", "T4", "T5", "T6", "T7"];
/// Long day names, Sunday first: CLDR `vi` wide and Windows NLS agree.
const VI_DAYS_LONG: [&str; 7] = ["Chủ Nhật", "Thứ Hai", "Thứ Ba", "Thứ Tư", "Thứ Năm", "Thứ Sáu", "Thứ Bảy"];

/// A format whose look follows the region rather than its code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Regional {
    /// A locale-dependent built-in format (ECMA-376 §18.8.30), by id.
    Builtin(u32),
    /// A code tagged `[$-F800]`: Excel's "system long date", whatever the rest of the code says.
    SystemLongDate,
}

impl Regional {
    pub(crate) fn of(code: &str) -> Option<Regional> {
        if code.trim_start().get(..8).is_some_and(|tag| tag.eq_ignore_ascii_case("[$-F800]")) {
            return Some(Regional::SystemLongDate);
        }
        crate::builtin_id(code).filter(|id| matches!(id, 14..=22)).map(Regional::Builtin)
    }
}

/// The display conventions of one region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Locale {
    pub decimal: char,
    pub group: char,
    vietnamese: bool,
}

impl Locale {
    pub(crate) const EN_US: Locale = Locale { decimal: '.', group: ',', vietnamese: false };

    pub(crate) const fn of(region: Region) -> Locale {
        match region {
            Region::ViVn => Locale { decimal: region.decimal_separator(), group: region.group_separator(), vietnamese: true },
            Region::EnUs | Region::EsEs => Locale::EN_US,
        }
    }

    /// Canonical number text (`1234.5`, `1.2E+10`) with this region's decimal separator.
    pub(crate) fn number_text(self, canonical: String) -> String {
        if self.decimal == '.' { canonical } else { canonical.replace('.', self.decimal.encode_utf8(&mut [0; 4])) }
    }

    /// `mmm` (`short`) or `mmmm`; `month` is 1-based.
    pub(crate) fn month_name(self, month: u32, short: bool) -> String {
        let i = (month as usize).wrapping_sub(1);
        if self.vietnamese {
            let names = if short { &VI_MONTHS_SHORT } else { &VI_MONTHS_LONG };
            return names.get(i).copied().unwrap_or("").to_string();
        }
        let name = MONTHS.get(i).copied().unwrap_or("");
        if short { name.chars().take(3).collect() } else { name.to_string() }
    }

    /// `mmmmm`: the first letter in English; Vietnamese month names all start with `t`, so the
    /// month number stands in (CLDR `vi` narrow months).
    pub(crate) fn month_initial(self, month: u32) -> String {
        if self.vietnamese {
            return month.to_string();
        }
        MONTHS.get((month as usize).wrapping_sub(1)).and_then(|m| m.chars().next()).map(String::from).unwrap_or_default()
    }

    /// `ddd` (`short`) or `dddd`; `weekday` is 0 for Sunday.
    pub(crate) fn day_name(self, weekday: u32, short: bool) -> String {
        let i = weekday as usize;
        if self.vietnamese {
            let names = if short { &VI_DAYS_SHORT } else { &VI_DAYS_LONG };
            return names.get(i).copied().unwrap_or("").to_string();
        }
        let name = WEEKDAYS.get(i).copied().unwrap_or("");
        if short { name.chars().take(3).collect() } else { name.to_string() }
    }

    /// `AM/PM`: vi-VN's `SA`/`CH` (Windows NLS and CLDR `vi` agree). `A/P` keeps its typed letters.
    pub(crate) fn day_period(self, pm: bool) -> &'static str {
        match (self.vietnamese, pm) {
            (true, false) => "SA",
            (true, true) => "CH",
            (false, false) => "AM",
            (false, true) => "PM",
        }
    }

    /// The code a regional format shows with here, when it differs from its own. Like Excel, ids
    /// 14 and 22 follow the region's short date (vi-VN `dd/MM/yyyy`) and `[$-F800]` its long date
    /// (`dd MMMM yyyy`; both per Windows NLS). The other ids in 15–21 are spelled the same in
    /// vi-VN (month names still localize); 27–36 and 50–58 exist only for East Asian and Thai
    /// locales.
    pub(crate) fn code_for(self, format: Regional) -> Option<&'static str> {
        if !self.vietnamese {
            return None;
        }
        match format {
            Regional::Builtin(14) => Some("dd/mm/yyyy"),
            Regional::Builtin(22) => Some("dd/mm/yyyy h:mm"),
            Regional::SystemLongDate => Some("dd mmmm yyyy"),
            Regional::Builtin(_) => None,
        }
    }
}
