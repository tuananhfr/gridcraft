//! The regional format: a preference of its own that defaults from the interface language and
//! decides how typed constants and formulas read, never what is stored.
use gridcraft_engine::core::{CellRef, Region, Value, region::VND_ACCOUNTING};
use gridcraft_ui_egui::{SheetApp, UiState, i18n::Language};
use serde_json::json;

fn app(language: Language) -> SheetApp {
    let mut session = gridcraft_engine::Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.language = language;
    app
}

fn enter(app: &mut SheetApp, cell: &str, text: &str) -> bool {
    app.run("selection.set", json!({"range": cell})).unwrap();
    app.begin_edit(Some(text.into()), false);
    app.commit_edit(0, 0, false, false)
}

fn edit_text(app: &mut SheetApp, cell: &str) -> String {
    app.run("selection.set", json!({"range": cell})).unwrap();
    app.begin_edit(None, true);
    let text = app.editor.as_ref().unwrap().text.clone();
    app.cancel_edit();
    text
}

fn cell(app: &SheetApp, a1: &str) -> (Value, String, Option<String>) {
    let d = app.session.active().unwrap();
    let sh = d.wb.active().unwrap();
    let at = CellRef::parse(a1).unwrap();
    let formula = sh.cell(at).and_then(|c| c.formula.as_ref()).map(|f| f.text.to_string());
    (sh.value(at), d.wb.styles.get(sh.style_id(at)).num_fmt.as_str().to_string(), formula)
}

#[test]
fn region_defaults_from_the_interface_language_until_chosen() {
    let mut app = app(Language::En);
    assert_eq!(app.ui.region(), Region::EnUs);
    for (language, region) in [(Language::Vi, Region::ViVn), (Language::Es, Region::EsEs), (Language::De, Region::EnUs), (Language::En, Region::EnUs)]
    {
        app.run("app.language.set", json!({"language": language.code()})).unwrap();
        assert_eq!(app.ui.region(), region, "{language:?}");
    }
    app.run("app.language.set", json!({"language": "vi"})).unwrap();
    assert_eq!(app.run("app.region.set", json!({"region": "en-US"})), Ok(json!({"region": "en-US", "auto": false})));
    assert_eq!(app.ui.region(), Region::EnUs, "a chosen region outlives the language");
    app.run("app.language.set", json!({"language": "es"})).unwrap();
    assert_eq!(app.ui.region(), Region::EnUs);
    assert!(app.run("app.region.set", json!({"region": "xx"})).is_err());
    assert!(app.run("app.region.set", json!({})).is_err(), "no region is an error, not a dialog");
    assert_eq!(app.ui.region(), Region::EnUs);
}

#[test]
fn the_choice_persists_with_the_other_preferences() {
    let mut app = app(Language::Vi);
    app.run("app.region.set", json!({"region": "en-US"})).unwrap();
    let saved = serde_json::to_value(&app.ui).unwrap();
    assert_eq!(saved["region"], "en-US");
    let restored: UiState = serde_json::from_value(saved).unwrap();
    assert_eq!((restored.region, restored.region()), (Some(Region::EnUs), Region::EnUs));
    app.run("app.region.set", json!({"region": "auto"})).unwrap();
    let saved = serde_json::to_value(&app.ui).unwrap();
    assert!(saved.get("region").is_none(), "following the language saves nothing");
    let restored: UiState = serde_json::from_value(saved).unwrap();
    assert_eq!((restored.region, restored.region()), (None, Region::ViVn));
    // A value from a later version (or hand-edited) follows the language, keeping the rest.
    let restored: UiState = serde_json::from_str(r#"{"dark": true, "language": "vi", "region": "xx-YY"}"#).unwrap();
    assert!(restored.dark);
    assert_eq!((restored.region, restored.region()), (None, Region::ViVn));
    let restored: UiState = serde_json::from_str(r#"{"region": 5}"#).unwrap();
    assert_eq!(restored.region, None);
}

#[test]
fn vietnamese_formulas_enter_and_redisplay_in_the_formula_bar() {
    let mut app = app(Language::Vi);
    assert!(enter(&mut app, "A1", "=SUM(1,5;2,25)"));
    assert_eq!(cell(&app, "A1").2.as_deref(), Some("SUM(1.5,2.25)"));
    assert_eq!(cell(&app, "A1").0, Value::Number(3.75));
    assert!(enter(&mut app, "A2", "=IF(A1>1;ROUND(A1*0,5;1);{1\\2;3\\4})"));
    assert_eq!(cell(&app, "A2").2.as_deref(), Some("IF(A1>1,ROUND(A1*0.5,1),{1,2;3,4})"));
    assert_eq!(edit_text(&mut app, "A1"), "=SUM(1,5;2,25)");
    assert_eq!(edit_text(&mut app, "A2"), "=IF(A1>1;ROUND(A1*0,5;1);{1\\2;3\\4})");
    // Committing the redisplayed text unchanged keeps the stored formula.
    app.begin_edit(None, true);
    assert!(app.commit_edit(0, 0, false, false));
    assert_eq!(cell(&app, "A2").2.as_deref(), Some("IF(A1>1,ROUND(A1*0.5,1),{1,2;3,4})"));
    // English separators are not Vietnamese syntax; nothing is entered.
    assert!(!enter(&mut app, "A3", "=SUM(1.5,2)"));
    app.cancel_edit();
    // Changing the region redraws the same stored formula in the new notation.
    app.run("app.region.set", json!({"region": "en-US"})).unwrap();
    assert_eq!(edit_text(&mut app, "A1"), "=SUM(1.5,2.25)");
    assert_eq!(cell(&app, "A1").2.as_deref(), Some("SUM(1.5,2.25)"));
    app.run("app.region.set", json!({"region": "es-ES"})).unwrap();
    assert_eq!(edit_text(&mut app, "A1"), "=SUMA(1,5;2,25)");
}

#[test]
fn vietnamese_constants_read_and_edit_in_vietnamese_notation() {
    let mut app = app(Language::Vi);
    for (at, typed, value, format, shown) in [
        ("B1", "1.234,5", Value::Number(1234.5), "#,##0.00", "1234,5"),
        ("B2", "1.234", Value::Number(1234.0), "#,##0", "1234"),
        ("B3", "-1.234", Value::Number(-1234.0), "#,##0", "-1234"),
        ("B4", "1234,5", Value::Number(1234.5), "General", "1234,5"),
        ("B5", "50%", Value::Number(0.5), "0%", "0,5"),
        ("B6", "1.000.000 ₫", Value::Number(1_000_000.0), "#,##0 [$₫-42A]", "1000000"),
        ("B7", "19/10/2026", Value::Number(46314.0), "m/d/yyyy", "19/10/2026"),
        ("B8", "14:30", Value::Number(14.5 / 24.0), "h:mm", "14:30"),
        ("B9", "1.23", Value::text("1.23"), "General", "1.23"),
    ] {
        assert!(enter(&mut app, at, typed), "{typed}");
        assert_eq!(cell(&app, at), (value.clone(), format.to_string(), None), "{typed}");
        assert_eq!(edit_text(&mut app, at), shown, "{typed}");
        // F2 then Enter keeps the value.
        assert!(enter(&mut app, at, shown));
        assert_eq!(cell(&app, at).0, value, "{typed} after re-entering {shown}");
    }
    // en-US is untouched.
    app.run("app.region.set", json!({"region": "en-US"})).unwrap();
    assert!(enter(&mut app, "C1", "1.234"));
    assert_eq!(cell(&app, "C1").0, Value::Number(1.234));
    assert!(enter(&mut app, "C2", "$1,000"));
    assert_eq!(cell(&app, "C2"), (Value::Number(1000.0), "\"$\"#,##0".into(), None));
    assert_eq!(edit_text(&mut app, "B1"), "1234.5");
}

#[test]
fn validation_reads_the_entry_in_the_region() {
    let mut app = app(Language::Vi);
    app.run("data.validation", json!({"range": "A1", "type": "decimal", "operator": "greater", "formula1": "1000"})).unwrap();
    assert!(enter(&mut app, "A1", "1.234"), "1.234 is 1234 in vi-VN");
    assert!(!enter(&mut app, "A1", "1,5"), "1,5 is 1.5 in vi-VN");
}

#[test]
fn the_accounting_button_applies_the_regions_currency() {
    let mut app = app(Language::Vi);
    assert_eq!(app.accounting_params(), json!({"symbol": "₫"}));
    app.run("selection.set", json!({"range": "A1"})).unwrap();
    let params = app.accounting_params();
    app.run("home.accounting", params).unwrap();
    assert_eq!(cell(&app, "A1").1, VND_ACCOUNTING);
    app.run("app.region.set", json!({"region": "en-US"})).unwrap();
    assert_eq!(app.accounting_params(), json!({}));
    app.run("app.region.set", json!({"region": "es-ES"})).unwrap();
    assert_eq!(app.accounting_params(), json!({}), "Spanish keeps the dollar it had");
}

#[test]
fn xlsx_round_trip_is_invariant() {
    let mut app = app(Language::Vi);
    assert!(enter(&mut app, "A1", "1.234,5"));
    assert!(enter(&mut app, "A2", "1.000.000 ₫"));
    assert!(enter(&mut app, "A3", "19/10/2026"));
    assert!(enter(&mut app, "A4", "=SUM(A1;1,5)"));
    app.run("selection.set", json!({"range": "A5"})).unwrap();
    let params = app.accounting_params();
    app.run("home.accounting", params).unwrap();
    assert!(enter(&mut app, "A5", "2.500"));
    let cells = ["A1", "A2", "A3", "A4", "A5"];
    let before: Vec<_> = cells.iter().map(|c| cell(&app, c)).collect();
    assert_eq!(before[3].2.as_deref(), Some("SUM(A1,1.5)"));
    let saved = app.run("file.save", json!({"format": "xlsx"})).unwrap();
    app.run("file.open", json!({"name": "vung.xlsx", "base64": saved["base64"]})).unwrap();
    let after: Vec<_> = cells.iter().map(|c| cell(&app, c)).collect();
    assert_eq!(after, before);
    assert_eq!(edit_text(&mut app, "A4"), "=SUM(A1;1,5)");
    assert_eq!(edit_text(&mut app, "A3"), "19/10/2026");
    // The same file in en-US: same numbers, English notation.
    app.run("app.region.set", json!({"region": "en-US"})).unwrap();
    assert_eq!(edit_text(&mut app, "A4"), "=SUM(A1,1.5)");
    assert_eq!(cell(&app, "A4").0, Value::Number(1236.0));
}
