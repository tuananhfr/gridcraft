//! Parsing what people type into a cell (and numeric text in formulas).
//!
//! Recognises numbers with thousands separators, percentages, currency, scientific notation,
//! fractions (`1 1/2`), dates (`3/4/2026`, `2026-03-04`, `4-Mar-2026`, `Mar 4, 2026`), times
//! (`10:30`, `10:30:15 PM`) and date-times, in en-US notation or in a [`Region`]'s.

use crate::date::{DateSystem, MONTHS, serial_from_ymd, time_fraction};
use crate::region::{DateOrder, Region};
use crate::value::{CellError, Value};

/// A parsed entry: its value and the number format Excel would apply automatically.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub value: Value,
    pub format: Option<&'static str>,
}

/// Interprets typed cell input (not formulas). Text that isn't a number, date, boolean or error
/// stays text. A leading apostrophe forces text.
pub fn parse_input(s: &str, sys: DateSystem) -> Parsed {
    parse_entry(s, |t| parse_numeric(t, sys))
}

/// Like [`parse_input`], reading numbers, dates and currency in `region`'s notation.
pub fn parse_input_in(s: &str, sys: DateSystem, region: Region) -> Parsed {
    match region {
        Region::EnUs => parse_input(s, sys),
        Region::EsEs => {
            // Pre-region Spanish entry: a decimal comma only for plain numbers.
            let normalized = s.trim().replace(',', ".");
            let number = normalized.strip_suffix('%').unwrap_or(&normalized);
            if number.parse::<f64>().is_ok_and(f64::is_finite) { parse_input(&normalized, sys) } else { parse_input(s, sys) }
        }
        Region::ViVn => parse_entry(s, |t| parse_numeric_vi(t, sys)),
    }
}

fn parse_entry(s: &str, numeric: impl Fn(&str) -> Option<(f64, Option<&'static str>)>) -> Parsed {
    if s.is_empty() {
        return Parsed { value: Value::Empty, format: None };
    }
    if let Some(rest) = s.strip_prefix('\'') {
        return Parsed { value: Value::text(rest), format: None };
    }
    let t = s.trim();
    if t.eq_ignore_ascii_case("TRUE") {
        return Parsed { value: Value::Bool(true), format: None };
    }
    if t.eq_ignore_ascii_case("FALSE") {
        return Parsed { value: Value::Bool(false), format: None };
    }
    if let Some(e) = CellError::parse(t) {
        return Parsed { value: Value::Error(e), format: None };
    }
    if let Some((n, f)) = numeric(t) {
        return Parsed { value: Value::number(n), format: f };
    }
    Parsed { value: Value::text(s), format: None }
}

/// Numeric text as arithmetic coerces it ("1,000", "50%", "$3", "1/2/2020", "10:00").
pub fn parse_number_text(s: &str) -> Option<f64> {
    parse_number_text_in(s, DateSystem::D1900)
}

/// Like [`parse_number_text`], with date text read in the given workbook date system.
pub fn parse_number_text_in(s: &str, sys: DateSystem) -> Option<f64> {
    parse_numeric(s.trim(), sys).map(|(n, _)| n)
}

fn parse_numeric(t: &str, sys: DateSystem) -> Option<(f64, Option<&'static str>)> {
    if t.is_empty() {
        return None;
    }
    // Parenthesised negatives: (100) = -100.
    if let Some(inner) = t.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        return parse_numeric(inner.trim(), sys).map(|(n, f)| (-n, f));
    }
    if let Some(r) = parse_plain(t) {
        return Some(r);
    }
    if let Some(r) = parse_fraction(t) {
        return Some((r, Some("# ?/?")));
    }
    if let Some(r) = parse_datetime(t, sys, &EN_DATES) {
        return Some(r);
    }
    None
}

/// vi-VN: `.` groups thousands (`1.234` is 1234, never 1.234), `,` is the decimal point, the
/// currency follows the amount (`1.000 ₫`, `1.000đ`, `1.000 VND`) and dates are day first.
fn parse_numeric_vi(t: &str, sys: DateSystem) -> Option<(f64, Option<&'static str>)> {
    if t.is_empty() {
        return None;
    }
    // One level only: `((1))` stays text, and hostile input can't recurse.
    if let Some(inner) = t.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        return parse_plain_vi(inner.trim()).map(|(n, f)| (-n, f));
    }
    if let Some(r) = parse_plain_vi(t) {
        return Some(r);
    }
    if let Some(r) = parse_fraction(t) {
        return Some((r, Some("# ?/?")));
    }
    parse_datetime(t, sys, &VI_DATES)
}

fn parse_plain_vi(t: &str) -> Option<(f64, Option<&'static str>)> {
    let mut s = t;
    let mut neg = false;
    if let Some(r) = s.strip_prefix('-') {
        neg = true;
        s = r.trim_start();
    } else if let Some(r) = s.strip_prefix('+') {
        s = r.trim_start();
    }
    let lower = s.to_lowercase();
    let symbol = ["₫", "vnđ", "vnd", "đ"].into_iter().find(|sym| lower.ends_with(sym));
    if let Some(sym) = symbol {
        // Lowercasing keeps these suffixes' byte lengths, so the cut is on a char boundary.
        s = s.get(..s.len().saturating_sub(sym.len()))?.trim_end();
    }
    let mut percent = false;
    if symbol.is_none()
        && let Some(r) = s.strip_suffix('%')
    {
        percent = true;
        s = r.trim_end();
    }
    if s.is_empty() {
        return None;
    }
    let (int, frac) = s.split_once(',').map_or((s, None), |(a, b)| (a, Some(b)));
    let has_sep = int.contains('.');
    let mut digits = if has_sep {
        let groups: Vec<&str> = int.split('.').collect();
        let first = groups.first()?;
        if first.is_empty() || first.len() > 3 || groups.iter().skip(1).any(|g| g.len() != 3) {
            return None;
        }
        groups.concat()
    } else {
        int.to_string()
    };
    if let Some(f) = frac {
        digits.push('.');
        digits.push_str(f);
    }
    let (n, sci) = plain_number(&digits)?;
    let decimals = frac.is_some();
    let n = if percent { n / 100.0 } else { n };
    let n = if neg { -n } else { n };
    let fmt = if symbol.is_some() {
        Some(Region::ViVn.currency_format(decimals))
    } else if percent {
        Some(if decimals { "0.00%" } else { "0%" })
    } else if sci {
        Some("0.00E+00")
    } else if has_sep {
        Some(if decimals { "#,##0.00" } else { "#,##0" })
    } else {
        None
    };
    Some((n, fmt))
}

/// Canonical digits (`1234.5`, `1.5e3`) to a finite number, and whether it had an exponent.
fn plain_number(digits: &str) -> Option<(f64, bool)> {
    let b = digits.as_bytes();
    if !b.iter().all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')) {
        return None;
    }
    if !b.first().is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
        return None;
    }
    let n: f64 = digits.parse().ok()?;
    n.is_finite().then(|| (n, digits.contains(['e', 'E'])))
}

/// Signs, currency, thousands separators, percent, exponent.
fn parse_plain(t: &str) -> Option<(f64, Option<&'static str>)> {
    let mut s = t;
    let mut neg = false;
    let mut currency = false;
    let mut percent = false;
    if let Some(r) = s.strip_prefix('-') {
        neg = true;
        s = r.trim_start();
    } else if let Some(r) = s.strip_prefix('+') {
        s = r.trim_start();
    }
    if let Some(r) = s.strip_prefix('$') {
        currency = true;
        s = r.trim_start();
        if !neg && let Some(r2) = s.strip_prefix('-') {
            neg = true;
            s = r2;
        }
    }
    if let Some(r) = s.strip_suffix('%') {
        percent = true;
        s = r.trim_end();
    }
    if s.is_empty() {
        return None;
    }
    let has_sep = s.contains(',');
    let digits: String = if has_sep {
        // Thousands separators must be in groups of three before the decimal point.
        let (int, frac) = s.split_once('.').map_or((s, None), |(a, b)| (a, Some(b)));
        let groups: Vec<&str> = int.split(',').collect();
        let first = groups.first()?;
        if first.is_empty() || first.len() > 3 || groups.iter().skip(1).any(|g| g.len() != 3) {
            return None;
        }
        let mut d = groups.concat();
        if let Some(f) = frac {
            d.push('.');
            d.push_str(f);
        }
        d
    } else {
        s.to_string()
    };
    let (mut n, sci) = plain_number(&digits)?;
    if percent {
        n /= 100.0;
    }
    if neg {
        n = -n;
    }
    let fmt = if currency {
        Some(Region::EnUs.currency_format(digits.contains('.')))
    } else if percent {
        Some(if digits.contains('.') { "0.00%" } else { "0%" })
    } else if sci {
        Some("0.00E+00")
    } else if has_sep {
        Some(if digits.contains('.') { "#,##0.00" } else { "#,##0" })
    } else {
        None
    };
    Some((n, fmt))
}

/// `1/2` (only with a whole part, else it's a date), `1 1/2`, `0 3/4`.
fn parse_fraction(t: &str) -> Option<f64> {
    let (whole, frac) = t.split_once(' ')?;
    let neg = whole.starts_with('-');
    let w: f64 = whole.trim_start_matches('-').parse().ok()?;
    let (a, b) = frac.trim().split_once('/')?;
    let a: f64 = a.parse().ok()?;
    let b: f64 = b.parse().ok()?;
    if b == 0.0 || !w.is_finite() {
        return None;
    }
    let v = w + a / b;
    Some(if neg { -v } else { v })
}

fn month_from_name(s: &str) -> Option<u32> {
    let l = s.to_ascii_lowercase();
    if l.len() < 3 {
        return None;
    }
    MONTHS.iter().position(|m| m.to_ascii_lowercase().starts_with(&l)).map(|i| i as u32 + 1)
}

/// Time part: `h:mm`, `h:mm:ss`, `h:mm:ss.000`, `h AM`, with optional AM/PM (or the notation's
/// own names for them, such as vi-VN `SA`/`CH`).
fn parse_time(t: &str, notation: &DateNotation) -> Option<(f64, bool, bool)> {
    let t = t.trim();
    let lower = t.to_ascii_lowercase();
    let local = notation.day_periods.iter().find_map(|(name, pm)| lower.strip_suffix(name).map(|b| (b.trim().to_string(), Some(*pm))));
    let (body, ampm) = if let Some(local) = local {
        local
    } else if let Some(b) = lower.strip_suffix("am").or_else(|| lower.strip_suffix('a')) {
        (b.trim().to_string(), Some(false))
    } else if let Some(b) = lower.strip_suffix("pm").or_else(|| lower.strip_suffix('p')) {
        (b.trim().to_string(), Some(true))
    } else {
        (lower.clone(), None)
    };
    let parts: Vec<&str> = body.split(':').collect();
    if parts.len() == 1 && ampm.is_none() {
        return None;
    }
    if parts.len() > 3 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let mut h: f64 = parts.first()?.parse().ok()?;
    let m: f64 = parts.get(1).map_or(Some(0.0), |p| p.parse().ok())?;
    let s: f64 = parts.get(2).map_or(Some(0.0), |p| p.parse().ok())?;
    if m >= 60.0 || s >= 60.0 || h < 0.0 || m < 0.0 || s < 0.0 {
        return None;
    }
    if let Some(pm) = ampm {
        if !(0.0..=12.0).contains(&h) {
            return None;
        }
        if h == 12.0 {
            h = 0.0;
        }
        if pm {
            h += 12.0;
        }
    } else if h >= 10000.0 {
        return None;
    }
    Some((time_fraction(h, m, s), parts.len() == 3, ampm.is_some()))
}

/// How numeric dates are typed: which part comes first and what may separate the parts.
struct DateNotation {
    order: DateOrder,
    separators: &'static [char],
    /// Names for AM and PM besides `AM`/`PM` (lowercase, `true` for PM).
    day_periods: &'static [(&'static str, bool)],
}

const EN_DATES: DateNotation = DateNotation { order: DateOrder::MonthDay, separators: &['/', '-', '.'], day_periods: &[] };
/// `.` groups thousands in vi-VN, so it never separates date parts there. Times show `SA`/`CH`
/// there, so they read back.
const VI_DATES: DateNotation = DateNotation { order: DateOrder::DayMonth, separators: &['/', '-'], day_periods: &[("sa", false), ("ch", true)] };

fn parse_date(t: &str, sys: DateSystem, notation: &DateNotation) -> Option<(f64, &'static str)> {
    let t = t.trim().trim_end_matches(',');
    let num = |s: &str| s.trim().parse::<i64>().ok();
    let day_first = notation.order == DateOrder::DayMonth;
    // ISO yyyy-mm-dd or yyyy/mm/dd
    let parts: Vec<&str> = t.split(notation.separators).collect();
    if parts.len() == 3 {
        let (a, b, c) = (parts[0].trim(), parts[1].trim(), parts[2].trim());
        if a.len() == 4
            && let (Some(y), Some(m), Some(d)) = (num(a), num(b), num(c))
        {
            return valid_ymd(sys, y, m, d).map(|s| (s, "yyyy-mm-dd"));
        }
        if let (Some(first), Some(second), Some(y)) = (num(a), num(b), num(c)) {
            let (m, d) = if day_first { (second, first) } else { (first, second) };
            let y = if c.len() <= 2 { if y < 30 { 2000 + y } else { 1900 + y } } else { y };
            return valid_ymd(sys, y, m, d).map(|s| (s, "m/d/yyyy"));
        }
        // 4-Mar-2026
        if let (Some(d), Some(m), Some(y)) = (num(a), month_from_name(b), num(c)) {
            let y = if c.len() <= 2 { if y < 30 { 2000 + y } else { 1900 + y } } else { y };
            return valid_ymd(sys, y, m as i64, d).map(|s| (s, "d-mmm-yy"));
        }
    }
    if parts.len() == 2 {
        let (a, b) = (parts[0].trim(), parts[1].trim());
        // m/d in the current year is ambiguous without a clock; use month/day of 2026? Excel uses
        // the current year. We accept d-mmm / mmm-d only with month names, and m/d → current year.
        if let (Some(first), Some(second)) = (num(a), num(b)) {
            let y = current_year();
            let (m, d) = if day_first { (second, first) } else { (first, second) };
            if (1..=12).contains(&m) && (1..=31).contains(&d) {
                return valid_ymd(sys, y, m, d).map(|s| (s, "d-mmm"));
            }
            // m/yyyy, in either order
            if (1..=12).contains(&first) && b.len() == 4 {
                return valid_ymd(sys, second, first, 1).map(|s| (s, "mmm-yy"));
            }
        }
        if let (Some(d), Some(m)) = (num(a), month_from_name(b)) {
            return valid_ymd(sys, current_year(), m as i64, d).map(|s| (s, "d-mmm"));
        }
        if let (Some(m), Some(y)) = (month_from_name(a), num(b)) {
            let y = if b.len() <= 2 { if y < 30 { 2000 + y } else { 1900 + y } } else { y };
            return valid_ymd(sys, y, m as i64, 1).map(|s| (s, "mmm-yy"));
        }
    }
    // "Mar 4, 2026" / "March 4 2026" / "4 March 2026"
    let words: Vec<&str> = t.split([' ', ',']).filter(|w| !w.is_empty()).collect();
    if words.len() == 3 {
        // Two-digit years follow the same 2029 rule as the dashed forms above.
        let expand = |w: &str, y: i64| if w.len() <= 2 { if y < 30 { 2000 + y } else { 1900 + y } } else { y };
        if let (Some(m), Some(d), Some(y)) = (month_from_name(words[0]), num(words[1]), num(words[2])) {
            let y = expand(words[2], y);
            return valid_ymd(sys, y, m as i64, d).map(|s| (s, "d-mmm-yy"));
        }
        if let (Some(d), Some(m), Some(y)) = (num(words[0]), month_from_name(words[1]), num(words[2])) {
            let y = expand(words[2], y);
            return valid_ymd(sys, y, m as i64, d).map(|s| (s, "d-mmm-yy"));
        }
    }
    None
}

fn valid_ymd(sys: DateSystem, y: i64, m: i64, d: i64) -> Option<f64> {
    // Dates before 1900 stay text (`DATE()` would add 1900 to such a year).
    if !(1..=12).contains(&m) || d < 1 || y < 1900 {
        return None;
    }
    let max = crate::date::days_in_month_in(sys, y as i32, m as u32) as i64;
    if d > max {
        return None;
    }
    serial_from_ymd(sys, y, m, d)
}

/// The year used for dates typed without one, from the system clock (browser clock on wasm).
/// Falls back to 2026 only when no clock is available.
pub fn current_year() -> i64 {
    if let Some(ms) = crate::date::unix_now_ms() {
        let days = (ms / 86_400_000.0).floor() as i64;
        return crate::date::civil_from_days(days).0 as i64;
    }
    2026
}

fn parse_datetime(t: &str, sys: DateSystem, dates: &DateNotation) -> Option<(f64, Option<&'static str>)> {
    if let Some((tf, secs, ampm)) = parse_time(t, dates) {
        let f = match (secs, ampm) {
            // 24 hours or more: elapsed hours, so 25:00 shows as 25:00:00, not 1:00.
            _ if tf >= 1.0 => "[h]:mm:ss",
            (false, false) => "h:mm",
            (true, false) => "h:mm:ss",
            (false, true) => "h:mm AM/PM",
            (true, true) => "h:mm:ss AM/PM",
        };
        return Some((tf, Some(f)));
    }
    if let Some((d, f)) = parse_date(t, sys, dates) {
        return Some((d, Some(f)));
    }
    // date + time: split at the last space(s) before a time-looking tail.
    for (i, _) in t.match_indices(' ') {
        let (a, b) = (t.get(..i)?, t.get(i + 1..)?);
        if let (Some((d, _)), Some((tf, _, _))) = (parse_date(a, sys, dates), parse_time(b, dates)) {
            return Some((d + tf, Some("m/d/yyyy h:mm")));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Parsed {
        parse_input(s, DateSystem::D1900)
    }

    #[test]
    fn month_name_short_year() {
        for s in ["Mar 4, 26", "Mar 4 26", "March 4 26", "4 March 26", "Mar 4, 2026"] {
            assert_eq!(p(s).value, Value::Number(46085.0), "{s}");
        }
        assert_eq!(p("Mar 4, 99").value, p("Mar 4, 1999").value);
        assert_eq!(p("Mar 4, 1776").value, Value::text("Mar 4, 1776"));
        assert_eq!(p("Feb 30, 26").value, Value::text("Feb 30, 26"));
    }

    #[test]
    fn numbers() {
        assert_eq!(p("42").value, Value::Number(42.0));
        assert_eq!(p("-3.5").value, Value::Number(-3.5));
        assert_eq!(p("1,234").format, Some("#,##0"));
        assert_eq!(p("1,234").value, Value::Number(1234.0));
        assert_eq!(p("1,23").value, Value::text("1,23"));
        assert_eq!(p("12%").value, Value::Number(0.12));
        assert_eq!(p("$1,200.50").value, Value::Number(1200.5));
        assert_eq!(p("$1,200.50").format, Some("\"$\"#,##0.00"));
        assert_eq!(p("(100)").value, Value::Number(-100.0));
        assert_eq!(p("1e3").value, Value::Number(1000.0));
        assert_eq!(p("1 1/2").value, Value::Number(1.5));
        assert_eq!(p(".5").value, Value::Number(0.5));
    }

    #[test]
    fn texts_and_specials() {
        assert_eq!(p("hello").value, Value::text("hello"));
        assert_eq!(p("'123").value, Value::text("123"));
        assert_eq!(p("true").value, Value::Bool(true));
        assert_eq!(p("#N/A").value, Value::Error(CellError::NA));
        assert_eq!(p("e").value, Value::text("e"));
        assert_eq!(p("-").value, Value::text("-"));
        assert_eq!(p("1.2.3.4").value, Value::text("1.2.3.4"));
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(p("10/7/2026").value, Value::Number(46302.0));
        assert_eq!(p("2026-10-07").value, Value::Number(46302.0));
        assert_eq!(p("7-Oct-2026").value, Value::Number(46302.0));
        assert_eq!(p("Oct 7, 2026").value, Value::Number(46302.0));
        assert_eq!(p("2/30/2026").value, Value::text("2/30/2026"));
        assert_eq!(p("7/4/1776").value, Value::text("7/4/1776"));
        assert_eq!(p("1776-07-04").value, Value::text("1776-07-04"));
        assert_eq!(p("4-Jul-1776").value, Value::text("4-Jul-1776"));
        assert_eq!(p("12/31/1899").value, Value::text("12/31/1899"));
        assert_eq!(p("1/1/1900").value, Value::Number(1.0));
        assert_eq!(p("12:00").value, Value::Number(0.5));
        assert_eq!(p("6:00 PM").value, Value::Number(0.75));
        assert_eq!(p("12:00 AM").value, Value::Number(0.0));
        assert_eq!(p("10/7/2026 18:00").value, Value::Number(46302.75));
        assert_eq!(p("25:00").value, Value::Number(25.0 / 24.0));
        assert_eq!(p("25:00").format, Some("[h]:mm:ss"));
        assert_eq!(p("23:59").format, Some("h:mm"));
    }

    #[test]
    fn fictitious_1900_02_29() {
        // Excel accepts 29 February 1900 (serial 60) and keeps 1 March 1900 at 61.
        assert_eq!(p("2/29/1900").value, Value::Number(60.0));
        assert_eq!(p("1900-02-29").value, Value::Number(60.0));
        assert_eq!(p("29-Feb-1900").value, Value::Number(60.0));
        assert_eq!(p("2/28/1900").value, Value::Number(59.0));
        assert_eq!(p("3/1/1900").value, Value::Number(61.0));
        assert_eq!(p("2/29/1901").value, Value::text("2/29/1901"));
        assert_eq!(p("2/30/1900").value, Value::text("2/30/1900"));
        // The 1904 system starts after it.
        assert_eq!(parse_input("2/29/1900", DateSystem::D1904).value, Value::text("2/29/1900"));
        assert_eq!(parse_input("2/29/1904", DateSystem::D1904).value, Value::Number(59.0));
    }

    fn vi(s: &str) -> Parsed {
        parse_input_in(s, DateSystem::D1900, Region::ViVn)
    }

    #[test]
    fn vietnamese_numbers_group_with_dots_and_use_a_decimal_comma() {
        let n = |v: f64| Value::Number(v);
        for (input, value, format) in [
            ("1.234,5", n(1234.5), Some("#,##0.00")),
            ("1234,5", n(1234.5), None),
            ("-1.234", n(-1234.0), Some("#,##0")),
            ("(1.234)", n(-1234.0), Some("#,##0")),
            ("(1.000 ₫)", n(-1000.0), Some("#,##0 [$₫-42A]")),
            ("+1.234.567", n(1234567.0), Some("#,##0")),
            ("42", n(42.0), None),
            (",5", n(0.5), None),
            ("-0,25", n(-0.25), None),
            ("50%", n(0.5), Some("0%")),
            ("12,5%", n(0.125), Some("0.00%")),
            ("1,5E+3", n(1500.0), Some("0.00E+00")),
            ("1 1/2", n(1.5), Some("# ?/?")),
        ] {
            assert_eq!(vi(input), Parsed { value, format }, "{input}");
        }
    }

    #[test]
    fn vietnamese_ambiguities_are_explicit() {
        // A dot is a thousands separator, never a decimal point.
        assert_eq!(vi("1.234").value, Value::Number(1234.0));
        assert_eq!(p("1.234").value, Value::Number(1.234), "en-US keeps the decimal point");
        // A comma is the decimal point, never a thousands separator.
        assert_eq!(vi("1,234").value, Value::Number(1.234));
        assert_eq!(p("1,234").value, Value::Number(1234.0));
        // Dots that don't group by three, en-US notation and a dot date stay text.
        for text in ["1.23", "1234.5", "0.5", ".5", "1,234.5", "1.234,5,6", "$100", "1.2.3.4", "19.10.2026", "1.000 $", "((1))"] {
            assert_eq!(vi(text).value, Value::text(text), "{text}");
        }
    }

    #[test]
    fn vietnamese_currency_follows_the_amount() {
        let dong = Some("#,##0 [$₫-42A]");
        for input in ["1.000.000 ₫", "1.000.000₫", "1.000.000đ", "1.000.000 đ", "1.000.000 VND", "1.000.000 vnd", "1.000.000 VNĐ"] {
            assert_eq!(vi(input), Parsed { value: Value::Number(1_000_000.0), format: dong }, "{input}");
        }
        assert_eq!(vi("-1.500 ₫"), Parsed { value: Value::Number(-1500.0), format: dong });
        assert_eq!(vi("1.000,50 ₫"), Parsed { value: Value::Number(1000.5), format: Some("#,##0.00 [$₫-42A]") });
        assert_eq!(vi("₫").value, Value::text("₫"));
        assert_eq!(vi("đ").value, Value::text("đ"));
        assert_eq!(vi("ăn 1đ").value, Value::text("ăn 1đ"));
        // en-US doesn't read the dong.
        assert_eq!(p("1,000 ₫").value, Value::text("1,000 ₫"));
    }

    #[test]
    fn vietnamese_dates_are_day_first() {
        let day = |y, m, d| Value::Number(serial_from_ymd(DateSystem::D1900, y, m, d).unwrap());
        let year = current_year();
        for (input, value, format) in [
            ("19/10/2026", day(2026, 10, 19), "m/d/yyyy"),
            ("9/3/2026", day(2026, 3, 9), "m/d/yyyy"),
            ("09/03/26", day(2026, 3, 9), "m/d/yyyy"),
            ("19-10-2026", day(2026, 10, 19), "m/d/yyyy"),
            ("2026-10-19", day(2026, 10, 19), "yyyy-mm-dd"),
            ("19/10", day(year, 10, 19), "d-mmm"),
            ("1/2", day(year, 2, 1), "d-mmm"),
            ("10/2026", day(2026, 10, 1), "mmm-yy"),
        ] {
            assert_eq!(vi(input), Parsed { value, format: Some(format) }, "{input}");
        }
        assert_eq!(p("9/3/2026").value, day(2026, 9, 3), "en-US stays month first");
        assert_eq!(vi("10/19/2026").value, Value::text("10/19/2026"));
        assert_eq!(vi("31/2/2026").value, Value::text("31/2/2026"));
    }

    #[test]
    fn vietnamese_times_and_date_times() {
        assert_eq!(vi("14:30"), Parsed { value: Value::Number(14.5 / 24.0), format: Some("h:mm") });
        assert_eq!(vi("08:15:30").format, Some("h:mm:ss"));
        let at = vi("19/10/2026 18:00");
        assert_eq!(at.value, Value::Number(46314.75));
        assert_eq!(at.format, Some("m/d/yyyy h:mm"));
        // vi-VN shows AM/PM as SA/CH, so typed (and Ctrl+Shift+; inserted) times read back.
        assert_eq!(vi("1:30 CH"), Parsed { value: Value::Number(13.5 / 24.0), format: Some("h:mm AM/PM") });
        assert_eq!(vi("12:00:00 SA"), Parsed { value: Value::Number(0.0), format: Some("h:mm:ss AM/PM") });
        assert_eq!(vi("6 ch").value, Value::Number(0.75));
        assert_eq!(vi("1:30 PM").value, Value::Number(13.5 / 24.0), "AM/PM still read");
        assert_eq!(vi("19/10/2026 6:00 CH").value, Value::Number(46314.75));
        assert_eq!(p("1:30 CH").value, Value::text("1:30 CH"), "en-US doesn't read SA/CH");
    }

    #[test]
    fn spanish_keeps_its_decimal_comma_entry() {
        let es = |s: &str| parse_input_in(s, DateSystem::D1900, Region::EsEs);
        assert_eq!(es("-1,5").value, Value::Number(-1.5));
        assert_eq!(es("1,5%"), p("1.5%"));
        assert_eq!(es("1.5").value, Value::Number(1.5));
        assert_eq!(es("1.234,5").value, Value::text("1.234,5"));
        assert_eq!(es("3/4/2026"), p("3/4/2026"));
        for input in ["", "'1,5", "hola", "TRUE", "#N/A", "$1,200.50"] {
            assert_eq!(es(input), p(input), "{input}");
            assert_eq!(parse_input_in(input, DateSystem::D1900, Region::EnUs), p(input));
        }
    }
}
