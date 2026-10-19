use gridcraft_core::Region;
use gridcraft_formula::{FormulaLocale, parse, parse_input, print, print_input};

const VI: FormulaLocale = FormulaLocale::Vi;

fn input(src: &str) -> String {
    print(&parse_input(src, VI, |_| false).unwrap())
}

#[test]
fn locale_table_rows() {
    let row = |l: FormulaLocale| (l.list_separator(), l.decimal_separator(), l.array_column_separator(), l.array_row_separator());
    assert_eq!(row(FormulaLocale::En), (',', '.', ',', ';'));
    assert_eq!(row(FormulaLocale::Es), (';', ',', '\\', ';'));
    assert_eq!(row(FormulaLocale::Vi), (';', ',', '\\', ';'));
    assert!(FormulaLocale::En.is_canonical());
    assert!(!FormulaLocale::Es.is_canonical() && !FormulaLocale::Vi.is_canonical());
    assert_eq!(FormulaLocale::Es.function_name("SUM"), "SUMA");
    assert_eq!(FormulaLocale::Vi.function_name("SUM"), "SUM", "Vietnamese Excel keeps English names");
    assert_eq!(FormulaLocale::Vi.canonical_name("SUMA"), "SUMA");
    assert_eq!(FormulaLocale::of(Region::EnUs), FormulaLocale::En);
    assert_eq!(FormulaLocale::of(Region::EsEs), FormulaLocale::Es);
    assert_eq!(FormulaLocale::of(Region::ViVn), FormulaLocale::Vi);
}

#[test]
fn vietnamese_entry_uses_semicolons_and_decimal_commas() {
    for (src, expected) in [
        ("=SUM(1,5;2,25)", "SUM(1.5,2.25)"),
        ("IF(A1>0;\"Có\";\"Không\")", "IF(A1>0,\"Có\",\"Không\")"),
        ("VLOOKUP(A2;D:E;2;FALSE)", "VLOOKUP(A2,D:E,2,FALSE)"),
        ("ROUND(1234,567;2)*0,5", "ROUND(1234.567,2)*0.5"),
        ("SUM({1\\2;3\\4};5)", "SUM({1,2;3,4},5)"),
        ("{1,5\\-2;TRUE\\\"a;b\"}", "{1.5,-2;TRUE,\"a;b\"}"),
        ("SUM((A1:A3;C1:C3))", "SUM((A1:A3,C1:C3))"),
        ("LET(x;1,5;x*2)", "LET(x,1.5,x*2)"),
        ("'Bảng 1'!A1+\"1,5;2\"", "'Bảng 1'!A1+\"1,5;2\""),
    ] {
        assert_eq!(input(src), expected, "{src}");
    }
    // English separators, Spanish names and dotted decimals are not Vietnamese syntax.
    for bad in ["SUM(1.5;2)", "SUM({1;2\\3})"] {
        assert!(parse_input(bad, VI, |_| false).is_err(), "{bad}");
    }
    assert_eq!(input("SUM(1,2)"), "SUM(1.2)", "a comma inside a call is a decimal point");
    assert_eq!(input("SUMA(1;2)"), "SUMA(1,2)", "Spanish names are not aliases here");
}

#[test]
fn vietnamese_display_round_trips_through_the_canonical_text() {
    for canonical in [
        "IF(TRUE,SUM(1.5,2.25),0)",
        "SUM({1.5,2.25;3,4})+SUM((A1:A3,C1:C3))",
        r#"IF(A1="a,b;c",CONCAT("1.5",",",";"),SUM(0.25,A1:A3 B2))"#,
        "LAMBDA(x,x+1)(2)",
        "-1E+21+0.001",
    ] {
        let expr = parse(canonical).unwrap();
        let shown = print_input(&expr, VI, |_| false).unwrap();
        assert_eq!(print(&parse_input(&shown, VI, |_| false).unwrap()), print(&expr), "{canonical} → {shown}");
    }
    let shown = |s: &str| print_input(&parse(s).unwrap(), VI, |_| false).unwrap();
    assert_eq!(shown("IF(TRUE,SUM(1.5,2.25),0)"), "IF(TRUE;SUM(1,5;2,25);0)");
    assert_eq!(shown("SUM({1.5,2;3,4})"), "SUM({1,5\\2;3\\4})");
    assert_eq!(shown(r#"CONCAT("1.5",",")"#), r#"CONCAT("1.5";",")"#);
}
