use gridcraft_core::region::VND_ACCOUNTING;

use super::*;

const S: DateSystem = DateSystem::D1900;
/// Monday 19 October 2026.
const OCT_19: f64 = 46314.0;
/// Sunday 4 January 2026.
const JAN_4: f64 = 46026.0;

fn show(code: &str, v: Value, region: Region) -> String {
    format_value_in(&v, &NumberFormat::parse(code), S, region).text
}

/// (code, value, en-US, vi-VN); es-ES keeps the en-US display.
const CASES: &[(&str, f64, &str, &str)] = &[
    ("General", 1234.5, "1234.5", "1234,5"),
    ("General", -0.25, "-0.25", "-0,25"),
    ("General", 1.23456789e20, "1.23457E+20", "1,23457E+20"),
    ("General", 1e-10, "1E-10", "1E-10"),
    ("0", 1234.5, "1235", "1235"),
    ("0.00", 1234.567, "1234.57", "1234,57"),
    ("#,##0", 1234567.0, "1,234,567", "1.234.567"),
    ("#,##0", -1234567.89, "-1,234,568", "-1.234.568"),
    ("#,##0.00", 1234.5, "1,234.50", "1.234,50"),
    ("#,##0.00", 0.0, "0.00", "0,00"),
    ("#,##0,", 1234567.0, "1,235", "1.235"),
    ("#.##", 0.5, ".5", ",5"),
    ("0%", 0.256, "26%", "26%"),
    ("0.0%", 0.125, "12.5%", "12,5%"),
    ("0.00E+00", 12345.0, "1.23E+04", "1,23E+04"),
    ("##0.0E+0", 0.000123, "123.0E-6", "123,0E-6"),
    ("# ?/?", 1.5, "1 1/2", "1 1/2"),
    ("#,##0 ?/?", 12345.5, "12,345 1/2", "12.345 1/2"),
    ("#,##0.00;[Red](#,##0.00);\"zero\"", -1234.5, "(1,234.50)", "(1.234,50)"),
    ("#,##0.00;[Red](#,##0.00);\"zero\"", 0.0, "zero", "zero"),
    ("0.0;-0.0;\"-\";@", -2.25, "-2.3", "-2,3"),
    ("@", 1234.5, "1234.5", "1234,5"),
    ("\"v.\"0.0", 3.0, "v.3.0", "v.3,0"),
    ("#,##0 [$₫-42A]", 1_000_000.0, "1,000,000 ₫", "1.000.000 ₫"),
    ("#,##0.00 [$₫-42A]", 1234.5, "1,234.50 ₫", "1.234,50 ₫"),
    ("\"$\"#,##0.00", 1234.5, "$1,234.50", "$1.234,50"),
    (VND_ACCOUNTING, 1_000_000.0, " 1,000,000 ₫ ", " 1.000.000 ₫ "),
    (VND_ACCOUNTING, -1_000_000.0, "-1,000,000 ₫ ", "-1.000.000 ₫ "),
    (VND_ACCOUNTING, 0.0, " - ₫ ", " - ₫ "),
    // Built-in 14 and 22 follow the region's short date; an explicit code is shown as written.
    ("m/d/yyyy", OCT_19, "10/19/2026", "19/10/2026"),
    ("M/D/YYYY", OCT_19, "10/19/2026", "19/10/2026"),
    ("m/d/yyyy h:mm", OCT_19 + 0.5625, "10/19/2026 13:30", "19/10/2026 13:30"),
    ("mm/dd/yyyy", OCT_19, "10/19/2026", "10/19/2026"),
    ("d/m/yyyy", OCT_19, "19/10/2026", "19/10/2026"),
    ("dd.mm.yyyy", OCT_19, "19.10.2026", "19.10.2026"),
    ("[$-F800]dddd, mmmm dd, yyyy", OCT_19, "Monday, October 19, 2026", "19 tháng 10 2026"),
    ("[$-F400]h:mm:ss AM/PM", 0.5625, "1:30:00 PM", "1:30:00 PM"),
    ("d-mmm-yy", OCT_19, "19-Oct-26", "19-thg 10-26"),
    ("d-mmm", JAN_4, "4-Jan", "4-thg 1"),
    ("mmm-yy", OCT_19, "Oct-26", "thg 10-26"),
    ("mmmm", OCT_19, "October", "tháng 10"),
    ("mmmmm", OCT_19, "O", "10"),
    ("ddd", OCT_19, "Mon", "T2"),
    ("ddd", JAN_4, "Sun", "CN"),
    ("dddd, d mmmm yyyy", OCT_19, "Monday, 19 October 2026", "Thứ Hai, 19 tháng 10 2026"),
    ("dddd", JAN_4, "Sunday", "Chủ Nhật"),
    ("dddd", JAN_4 + 6.0, "Saturday", "Thứ Bảy"),
    ("h:mm", 0.5625, "13:30", "13:30"),
    ("h:mm AM/PM", 0.5625, "1:30 PM", "1:30 PM"),
    ("mm:ss.0", 1.5 / 86_400.0, "00:01.5", "00:01,5"),
    ("[h]:mm:ss", 1.5, "36:00:00", "36:00:00"),
];

#[test]
fn numbers_and_dates_per_region() {
    for &(code, n, en, vi) in CASES {
        assert_eq!(show(code, Value::Number(n), Region::EnUs), en, "en-US {code} {n}");
        assert_eq!(show(code, Value::Number(n), Region::EsEs), en, "es-ES {code} {n}");
        assert_eq!(show(code, Value::Number(n), Region::ViVn), vi, "vi-VN {code} {n}");
    }
}

#[test]
fn en_us_is_the_plain_format_value() {
    for &(code, n, _, _) in CASES {
        let fmt = NumberFormat::parse(code);
        assert_eq!(format_value_in(&Value::Number(n), &fmt, S, Region::EnUs), format_value(&Value::Number(n), &fmt, S), "{code}");
    }
}

#[test]
fn text_booleans_and_errors_look_the_same_everywhere() {
    for region in Region::ALL {
        assert_eq!(show("0.0;-0.0;0;\"[\"@\"]\"", Value::from("1.5"), region), "[1.5]");
        assert_eq!(show("General", Value::from("1.234,5"), region), "1.234,5");
        assert_eq!(show("#,##0.00", Value::Bool(true), region), "TRUE");
        assert_eq!(show("0.00", Value::Error(CellError::Div0), region), "#DIV/0!");
        assert_eq!(show("0.00", Value::Number(f64::INFINITY), region), "#NUM!");
    }
}

#[test]
fn colours_and_fill_survive_the_region() {
    let fmt = NumberFormat::parse("#,##0.00;[Red]-#,##0.00");
    let vi = format_value_in(&Value::Number(-1234.5), &fmt, S, Region::ViVn);
    assert_eq!((vi.text.as_str(), vi.color), ("-1.234,50", Some(FormatColor::Red)));
    let acc = format_value_in(&Value::Number(1234.0), &NumberFormat::parse(VND_ACCOUNTING), S, Region::ViVn);
    assert_eq!(acc.fill, Some((' ', 1)));
}

#[test]
fn general_fit_uses_the_decimal_comma() {
    assert_eq!(format_general_fit_in(1234.5678, 6, Region::ViVn).as_deref(), Some("1234,6"));
    assert_eq!(format_general_fit_in(1234.5678, 6, Region::EnUs).as_deref(), Some("1234.6"));
    assert_eq!(format_general_fit_in(1234.5678, 6, Region::EsEs).as_deref(), Some("1234.6"));
    assert_eq!(format_general_fit_in(1.5e30, 8, Region::ViVn).as_deref(), Some("1,5E+30"));
}

#[test]
fn text_function_stays_en_us() {
    assert_eq!(text_function(&Value::Number(1234.5), "#,##0.00", S).as_deref(), Ok("1,234.50"));
    assert_eq!(text_function(&Value::Number(OCT_19), "m/d/yyyy", S).as_deref(), Ok("10/19/2026"));
}
