//! Cloud comment marks on the grid: drawn for the threads shown, a click on a marked cell names
//! its thread (and still selects the cell), and nothing about the workbook changes. Without
//! threads (the desktop app) a click names nothing.

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::comment_marks::MarkThread;
use gridcraft_ui_egui::{SheetApp, grid::Geo};
use serde_json::json;

fn harness() -> egui_kittest::Harness<'static, SheetApp> {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("cell.set", json!({"cell": "C3", "input": "note me"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(4);
    h
}

fn centre(h: &egui_kittest::Harness<'_, SheetApp>, cell: CellRef) -> Pos2 {
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    let geo = Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll);
    geo.cell_rect(sheet, cell).center()
}

fn click(h: &mut egui_kittest::Harness<'_, SheetApp>, cell: CellRef) {
    let pos = centre(h, cell);
    for pressed in [true, false] {
        h.input_mut()
            .events
            .extend([Event::PointerMoved(pos), Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
        h.step();
    }
    h.run_steps(2);
}

fn show_on(h: &mut egui_kittest::Harness<'_, SheetApp>, range: &str) {
    let app = h.state_mut();
    app.session.execute("selection.set", json!({"range": range})).unwrap();
    let doc = app.session.active().unwrap().uid;
    let anchor = app.session.comment_anchor(doc).unwrap();
    app.session.execute("selection.set", json!({"range": "A1"})).unwrap();
    app.comment_marks.show(doc, vec![MarkThread { id: "t1".into(), anchor }]);
}

#[test]
fn a_click_on_a_marked_cell_names_its_thread_and_still_selects() {
    let mut h = harness();
    show_on(&mut h, "C3");
    h.run_steps(2);
    let wb = h.state().session.active().unwrap().wb.clone();
    click(&mut h, CellRef::new(2, 2));
    assert_eq!(h.state_mut().comment_marks.take_clicked(), ["t1"]);
    assert_eq!(h.state().session.active().unwrap().selection.a1(), "C3");
    click(&mut h, CellRef::new(5, 5));
    assert!(h.state_mut().comment_marks.take_clicked().is_empty());
    let d = h.state().session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&d.wb, &wb), "marks and clicks leave the workbook alone");
}

#[test]
fn without_threads_a_click_names_nothing() {
    let mut h = harness();
    click(&mut h, CellRef::new(2, 2));
    assert!(h.state().comment_marks.is_empty());
    assert!(h.state_mut().comment_marks.take_clicked().is_empty());
}
