//! Regional formats: the notation numbers, dates and currency are typed in. Like Excel's system
//! region, it is a setting of its own, separate from the interface language, and never changes
//! what is stored: files and the engine API keep the invariant (en-US) notation.

/// A regional format GridCraft can read typed input in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Region {
    #[default]
    EnUs,
    /// Spanish formula syntax. Typed constants keep the behaviour from before regions existed:
    /// a decimal comma for plain numbers, everything else read as en-US.
    EsEs,
    ViVn,
}

/// Which part of a numeric date comes first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateOrder {
    MonthDay,
    DayMonth,
}

/// The ₫ accounting format: symbol after the amount, no decimals (Excel's Vietnamese locale tag 42A).
pub const VND_ACCOUNTING: &str = "_-* #,##0 [$₫-42A]_-;-* #,##0 [$₫-42A]_-;_-* \"-\" [$₫-42A]_-;_-@_-";

impl Region {
    pub const ALL: [Region; 3] = [Region::EnUs, Region::EsEs, Region::ViVn];

    /// BCP 47 tag, as saved in preferences and taken by `cell.set`'s `region`.
    pub const fn code(self) -> &'static str {
        match self {
            Region::EnUs => "en-US",
            Region::EsEs => "es-ES",
            Region::ViVn => "vi-VN",
        }
    }

    /// Reads a tag leniently: `vi`, `vi-VN`, `vi_vn` all name Vietnamese.
    pub fn parse(code: &str) -> Option<Region> {
        let mut parts = code.trim().split(['-', '_']);
        let language = parts.next()?.to_ascii_lowercase();
        let country = parts.next().map(str::to_ascii_uppercase);
        let region = match language.as_str() {
            "en" => Region::EnUs,
            "es" => Region::EsEs,
            "vi" => Region::ViVn,
            _ => return None,
        };
        match country.as_deref() {
            None => Some(region),
            Some(c) if region.code().ends_with(c) => Some(region),
            Some(_) => None,
        }
    }

    pub const fn decimal_separator(self) -> char {
        match self {
            Region::EnUs => '.',
            Region::EsEs | Region::ViVn => ',',
        }
    }

    pub const fn group_separator(self) -> char {
        match self {
            Region::EnUs => ',',
            Region::EsEs | Region::ViVn => '.',
        }
    }

    /// The order numeric dates are typed in (Spanish input still reads month first; see [`Region::EsEs`]).
    pub const fn date_order(self) -> DateOrder {
        match self {
            Region::EnUs | Region::EsEs => DateOrder::MonthDay,
            Region::ViVn => DateOrder::DayMonth,
        }
    }

    /// The symbol the Accounting Number Format button applies by default.
    pub const fn currency_symbol(self) -> &'static str {
        match self {
            Region::EnUs | Region::EsEs => "$",
            Region::ViVn => "₫",
        }
    }

    /// The format a typed amount in this region's currency gets, like `"$"#,##0` for `$1,000`.
    pub const fn currency_format(self, decimals: bool) -> &'static str {
        match (self, decimals) {
            (Region::EnUs | Region::EsEs, false) => "\"$\"#,##0",
            (Region::EnUs | Region::EsEs, true) => "\"$\"#,##0.00",
            (Region::ViVn, false) => "#,##0 [$₫-42A]",
            (Region::ViVn, true) => "#,##0.00 [$₫-42A]",
        }
    }

    /// Canonical number text (`1234.5`, `1.5E+21`) as it is typed in this region, for the cell
    /// editor. Spanish keeps the canonical text, which its input still reads.
    pub fn number_text(self, canonical: &str) -> String {
        match self {
            Region::ViVn => canonical.replace('.', ","),
            Region::EnUs | Region::EsEs => canonical.to_string(),
        }
    }

    /// How a date (and time) is typed in this region, for the cell editor; `None` where the
    /// editor's ISO fallback already reads back.
    pub const fn edit_date_format(self, date: bool, time: bool) -> Option<&'static str> {
        match (self, date, time) {
            (Region::ViVn, true, false) => Some("dd/mm/yyyy"),
            (Region::ViVn, true, true) => Some("dd/mm/yyyy hh:mm:ss"),
            (Region::ViVn, false, _) => Some("hh:mm:ss"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_parse_leniently() {
        for r in Region::ALL {
            assert_eq!(Region::parse(r.code()), Some(r));
        }
        assert_eq!(Region::parse("vi"), Some(Region::ViVn));
        assert_eq!(Region::parse("VI_vn"), Some(Region::ViVn));
        assert_eq!(Region::parse(" es "), Some(Region::EsEs));
        assert_eq!(Region::parse("en-GB"), None);
        assert_eq!(Region::parse("fr-FR"), None);
        assert_eq!(Region::parse(""), None);
    }

    #[test]
    fn separators_and_currency() {
        assert_eq!((Region::EnUs.decimal_separator(), Region::EnUs.group_separator()), ('.', ','));
        assert_eq!((Region::ViVn.decimal_separator(), Region::ViVn.group_separator()), (',', '.'));
        assert_eq!(Region::ViVn.date_order(), DateOrder::DayMonth);
        assert_eq!(Region::ViVn.currency_symbol(), "₫");
        assert_eq!(Region::EsEs.currency_symbol(), "$");
        assert_eq!(Region::ViVn.number_text("-1234.5"), "-1234,5");
        assert_eq!(Region::EnUs.number_text("1234.5"), "1234.5");
    }
}
