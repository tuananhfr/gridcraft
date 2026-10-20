//! Vietnamese at display only: PivotTable date-group labels, SA/CH for AM/PM, the column width
//! tip's decimal comma, cell style names and undo labels. The workbook and its saved XLSX keep
//! the English the engine wrote.

use egui::{Event, Modifiers, PointerButton, Shape, pos2, vec2};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_ui_egui::grid::Geo;
use gridcraft_ui_egui::{SheetApp, i18n::Language};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

/// Sales by date, a PivotTable of it grouped by quarter on Sheet2 (active), and a time on Sheet1.
fn pivot_session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    let rows = json!([
        ["Region", "Date", "Sales"],
        ["East", "1/15/2026", 100],
        ["West", "5/5/2026", 70],
        ["East", "8/8/2026", 45],
        ["North", "11/11/2026", 120]
    ]);
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("cell.set", json!({"cell": "F1", "input": "1:30 PM"})).unwrap();
    s.execute("selection.set", json!({"range": "A1"})).unwrap();
    s.execute("insert.pivotTable", json!({"rows": ["Date"], "values": ["Sales"]})).unwrap();
    s.execute("pivot.group", json!({"field": "Date", "by": "quarters"})).unwrap();
    s
}

fn harness(s: Session, language: Language) -> Harness {
    let mut app = SheetApp::new(s, Default::default());
    app.ui.language = language;
    let mut h = egui_kittest::Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn painted(h: &Harness) -> Vec<String> {
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

fn xlsx(h: &Harness) -> Vec<u8> {
    gridcraft_engine::io::save_bytes(&h.state().session.doc().unwrap().wb, "book.xlsx").unwrap()
}

#[test]
fn pivot_quarter_and_month_labels_show_in_vietnamese_and_save_unchanged() {
    let english = harness(pivot_session(), Language::En);
    let mut h = harness(pivot_session(), Language::Vi);
    let shown = painted(&h);
    assert!(shown.iter().any(|t| t == "Quý 1") && shown.iter().any(|t| t == "Quý 4"), "{shown:?}");
    assert!(!shown.iter().any(|t| t == "Qtr1"));
    assert!(painted(&english).iter().any(|t| t == "Qtr1"), "English keeps Qtr1");
    let sheet = h.state().session.doc().unwrap().wb.active().unwrap().clone();
    assert_eq!(sheet.value(CellRef::parse("A4").unwrap()), Value::text("Qtr1"), "the cell keeps what the engine wrote");
    assert_eq!(xlsx(&h), xlsx(&english), "the saved XLSX doesn't depend on the display language");

    h.state_mut().run("pivot.group", json!({"field": "Date", "by": "months"})).unwrap();
    h.run_steps(3);
    let shown = painted(&h);
    assert!(shown.iter().any(|t| t == "Thg1") && shown.iter().any(|t| t == "Thg11"), "{shown:?}");
}

#[test]
fn text_outside_a_date_grouped_pivot_is_left_alone() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": [["Qtr1", "Jan"]]})).unwrap();
    let h = harness(s, Language::Vi);
    let shown = painted(&h);
    assert!(shown.iter().any(|t| t == "Qtr1") && shown.iter().any(|t| t == "Jan"), "typed text shows as typed");
}

#[test]
fn times_show_sa_ch_in_vi_vn_and_save_unchanged() {
    let first_sheet = || {
        let mut s = pivot_session();
        let data = s.doc().unwrap().wb.sheets.iter().position(|sh| sh.name == "Sheet1").unwrap();
        s.execute("sheet.activate", json!({"sheet": data})).unwrap();
        s
    };
    let english = harness(first_sheet(), Language::En);
    let h = harness(first_sheet(), Language::Vi);
    assert!(painted(&h).iter().any(|t| t == "1:30 CH"), "{:?}", painted(&h));
    assert!(painted(&english).iter().any(|t| t == "1:30 PM"));
    assert_eq!(xlsx(&h), xlsx(&english));
}

#[test]
fn ctrl_shift_semicolon_time_reads_back_as_a_time_in_vi_vn() {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = harness(s, Language::Vi);
    let ctrl_shift = Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: true };
    for pressed in [true, false] {
        h.input_mut().events.push(Event::Key {
            key: egui::Key::Semicolon,
            physical_key: Some(egui::Key::Semicolon),
            pressed,
            repeat: false,
            modifiers: ctrl_shift,
        });
    }
    h.run_steps(2);
    let typed = h.state().editor.as_ref().map(|e| e.text.clone()).unwrap();
    assert!(typed.ends_with(" SA") || typed.ends_with(" CH"), "{typed}");
    assert!(h.state_mut().commit_edit(0, 0, false, false));
    let v = h.state().session.doc().unwrap().wb.active().unwrap().value(CellRef::new(0, 0));
    assert!(matches!(v, Value::Number(n) if (0.0..1.0).contains(&n)), "a time, not text: {v:?}");
}

#[test]
fn the_column_width_tip_uses_the_decimal_comma_in_vi_vn() {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = harness(s, Language::Vi);
    let edge = {
        let app = h.state();
        let sheet = app.session.active().unwrap().wb.active().unwrap();
        let geo = Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll);
        pos2(geo.x(sheet, 0) + sheet.col_width(0) * geo.z, geo.rect.top() + geo.header_h * 0.5)
    };
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.input_mut().events.extend([Event::PointerMoved(edge), button(edge, true)]);
    h.step();
    for dx in [5.0, 10.0, 17.0] {
        h.input_mut().events.push(Event::PointerMoved(edge + vec2(dx, 0.0)));
        h.step();
    }
    let tip = h.state().toast.as_ref().map(|(t, _)| t.clone()).unwrap_or_default();
    h.input_mut().events.push(button(edge + vec2(17.0, 0.0), false));
    h.run_steps(2);
    assert!(tip.starts_with("Độ rộng: "), "{tip}");
    let width = tip.trim_start_matches("Độ rộng: ").split(' ').next().unwrap_or_default();
    assert!(width.contains(',') && !width.contains('.'), "decimal comma: {tip}");
}

#[test]
fn cell_style_names_and_undo_labels_have_vietnamese() {
    let missing: Vec<&str> = gridcraft_engine::command_specs()
        .iter()
        .filter(|c| c.undoable)
        .map(|c| c.label.trim_end_matches('…'))
        // Names Vietnamese Office keeps as they are.
        .filter(|l| Language::Vi.tr(l) == *l && !matches!(*l, "PivotChart" | "PivotTable" | "Solver" | "100%"))
        .collect();
    assert!(missing.is_empty(), "undo labels without Vietnamese: {missing:?}");
    for name in gridcraft_engine::cmd::format::CELL_STYLE_NAMES {
        let key = format!("Cell Style|{name}");
        assert_ne!(Language::Vi.tr(&key), *name, "{name}");
        assert_eq!(Language::En.tr(&key), *name);
    }
    assert_eq!(Language::Vi.tr("Cell Style|Normal"), "Bình thường");
    assert_eq!(Language::Vi.tr("Normal"), "Thường", "View › Normal keeps its own word");
}
