//! Touch on the grid: a finger pans the sheet, a tap selects, a double tap edits, a long press opens
//! the cell menu and the corner handle drags out a range. The mouse keeps its behaviour.

use egui::{Event, Modifiers, PointerButton, Pos2, TouchDeviceId, TouchId, TouchPhase, vec2};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_ui_egui::SheetApp;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = Harness::builder().with_size(vec2(390.0, 844.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(4);
    h
}

fn center(h: &Harness, a1: &str) -> Pos2 {
    corner_and_rect(h, a1).center()
}

fn corner_and_rect(h: &Harness, a1: &str) -> egui::Rect {
    let geo = h.state().grid.geo.clone().unwrap();
    let d = h.state().session.active().unwrap();
    geo.cell_rect(d.wb.active().unwrap(), CellRef::parse(a1).unwrap())
}

fn selection(h: &Harness) -> (RangeRef, CellRef) {
    let d = h.state().session.active().unwrap();
    (d.selection.current(), d.selection.active)
}

fn scroll(h: &Harness) -> egui::Vec2 {
    h.state().view().scroll
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn finger(id: u64, phase: TouchPhase, pos: Pos2) -> Event {
    Event::Touch { device_id: TouchDeviceId(1), id: TouchId(id), phase, pos, force: None }
}

/// What eframe's web runner sends for a finger: `pointerdown` alone, then `touchstart` (press +
/// touch), each `touchmove` (move + touch), and `touchend` (release, gone, touch).
fn touch_down(h: &mut Harness, at: Pos2) {
    h.input_mut().events.push(button(at, true));
    h.step();
    h.input_mut().events.extend([button(at, true), finger(0, TouchPhase::Start, at)]);
    h.step();
}

fn touch_move(h: &mut Harness, to: Pos2) {
    h.input_mut().events.extend([Event::PointerMoved(to), finger(0, TouchPhase::Move, to)]);
    h.step();
}

fn touch_up(h: &mut Harness, at: Pos2) {
    h.input_mut().events.extend([button(at, false), Event::PointerGone, finger(0, TouchPhase::End, at)]);
    h.step();
}

fn touch_drag(h: &mut Harness, from: Pos2, to: Pos2) {
    touch_down(h, from);
    for i in 1..=10 {
        touch_move(h, from + (to - from) * (i as f32 / 10.0));
    }
    touch_up(h, to);
    h.run_steps(2);
}

fn tap(h: &mut Harness, at: Pos2) {
    touch_down(h, at);
    touch_up(h, at);
}

fn mouse_drag(h: &mut Harness, from: Pos2, to: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(from));
    h.step();
    h.input_mut().events.push(button(from, true));
    h.step();
    for i in 1..=10 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (i as f32 / 10.0)));
        h.step();
    }
    h.input_mut().events.push(button(to, false));
    h.run_steps(2);
}

#[test]
fn a_finger_drag_pans_the_sheet_and_keeps_the_selection() {
    let mut h = harness();
    let before = selection(&h);
    let from = center(&h, "C12");
    touch_drag(&mut h, from, from - vec2(120.0, 250.0));
    let s = scroll(&h);
    assert!(s.y > 150.0 && s.x > 60.0, "the sheet followed the finger: {s:?}");
    assert_eq!(selection(&h), before, "panning selects nothing");
    assert!(h.state().editor.is_none());
}

#[test]
fn a_mouse_drag_still_selects_a_range() {
    let mut h = harness();
    let (from, to) = (center(&h, "B2"), center(&h, "C5"));
    mouse_drag(&mut h, from, to);
    assert_eq!(selection(&h).0, RangeRef::parse("B2:C5").unwrap());
    assert_eq!(scroll(&h), egui::Vec2::ZERO);
    assert!(!h.state().grid.touch.active);
}

#[test]
fn a_tap_selects_and_a_double_tap_edits() {
    let mut h = harness();
    let at = center(&h, "D4");
    tap(&mut h, at);
    h.run_steps(2);
    assert_eq!(selection(&h).1, CellRef::parse("D4").unwrap());
    assert!(h.state().editor.is_none());
    tap(&mut h, at);
    h.run_steps(2);
    assert!(h.state().editor.is_some(), "a second tap on the cell starts editing it");
}

#[test]
fn a_long_press_opens_the_cell_menu_on_that_cell() {
    let mut h = harness();
    let at = center(&h, "C6");
    touch_down(&mut h, at);
    h.run_steps(60);
    assert!(h.state().grid.context_menu.is_some(), "held for a second without moving");
    touch_up(&mut h, at);
    h.run_steps(2);
    assert!(h.state().grid.context_menu.is_some(), "lifting the finger leaves the menu open");
    assert_eq!(selection(&h).1, CellRef::parse("C6").unwrap());
    assert_eq!(scroll(&h), egui::Vec2::ZERO);
}

#[test]
fn the_corner_handle_drags_out_a_range() {
    let mut h = harness();
    let b2 = center(&h, "B2");
    tap(&mut h, b2);
    h.run_steps(2);
    assert!(h.state().grid.touch.active);
    let corner = corner_and_rect(&h, "B2").right_bottom() - vec2(2.0, 2.0);
    let to = center(&h, "D6");
    touch_drag(&mut h, corner, to);
    assert_eq!(selection(&h), (RangeRef::parse("B2:D6").unwrap(), CellRef::parse("B2").unwrap()));
    assert_eq!(scroll(&h), egui::Vec2::ZERO, "the handle selects instead of panning");
}

#[test]
fn a_finger_drag_over_an_unselected_chart_pans_instead_of_moving_it() {
    let mut h = harness();
    let st = h.state_mut();
    st.run("range.setValues", serde_json::json!({"range": "A1:B3", "values": [["x", "y"], ["a", 1], ["b", 2]]})).unwrap();
    let chart = st.run("insert.chart", serde_json::json!({"range": "A1:B3", "type": "column"})).unwrap()["chart"].clone();
    st.run("object.move", serde_json::json!({"kind": "chart", "id": chart, "at": "A6", "dx": 0, "dy": 0})).unwrap();
    st.selected_chart = None;
    h.run_steps(4);
    let anchor = |h: &Harness| h.state().session.active().unwrap().wb.active().unwrap().charts[0].anchor;
    let before = anchor(&h);
    let geo = h.state().grid.geo.clone().unwrap();
    let sheet = h.state().session.active().unwrap().wb.active().unwrap().clone();
    let start = geo.cell_rect(&sheet, before.cell).left_top() + vec2(before.dx * geo.z + 30.0, before.dy * geo.z + 30.0);
    touch_drag(&mut h, start, start - vec2(0.0, 200.0));
    assert_eq!(anchor(&h), before, "the chart stays put");
    assert!(scroll(&h).y > 100.0, "the sheet panned");
}

#[test]
fn after_a_finger_the_mouse_selects_again() {
    let mut h = harness();
    let from = center(&h, "C12");
    touch_drag(&mut h, from, from - vec2(0.0, 100.0));
    assert!(h.state().grid.touch.active);
    let panned = scroll(&h);
    let (a, b) = (center(&h, "B8"), center(&h, "C10"));
    mouse_drag(&mut h, a, b);
    assert!(!h.state().grid.touch.active);
    assert_eq!(scroll(&h), panned);
    let (range, _) = selection(&h);
    assert!(range.height() == 3 && range.width() == 2, "the mouse drag selected {}", range.a1());
}

#[test]
fn a_quick_pan_flings_on_with_animations() {
    let mut h = harness();
    h.ctx.all_styles_mut(|s| s.animation_time = 0.1);
    h.step();
    let from = center(&h, "C14");
    touch_down(&mut h, from);
    for i in 1..=4 {
        touch_move(&mut h, from - vec2(0.0, 60.0 * i as f32));
    }
    touch_up(&mut h, from - vec2(0.0, 240.0));
    let released = scroll(&h).y;
    h.run_steps(20);
    let later = scroll(&h).y;
    assert!(later > released + 20.0, "the sheet kept moving: {released} → {later}");
    h.run_steps(120);
    let settled = scroll(&h).y;
    h.run_steps(10);
    assert_eq!(scroll(&h).y, settled, "and then stopped");
}

#[test]
fn a_pinch_zooms_without_panning() {
    let mut h = harness();
    let mid = center(&h, "B10");
    let (a, b) = (mid - vec2(40.0, 0.0), mid + vec2(40.0, 0.0));
    h.input_mut().events.push(button(a, true));
    h.step();
    h.input_mut().events.extend([button(a, true), finger(0, TouchPhase::Start, a), finger(1, TouchPhase::Start, b)]);
    h.step();
    for i in 1..=20 {
        let spread = vec2(6.0 * i as f32, 0.0);
        h.input_mut().events.extend([
            Event::PointerMoved(a - spread),
            finger(0, TouchPhase::Move, a - spread),
            finger(1, TouchPhase::Move, b + spread),
        ]);
        h.step();
    }
    let end = a - vec2(120.0, 0.0);
    h.input_mut().events.extend([
        button(end, false),
        Event::PointerGone,
        finger(0, TouchPhase::End, end),
        finger(1, TouchPhase::End, b + vec2(120.0, 0.0)),
    ]);
    h.run_steps(4);
    let zoom = h.state().session.active().unwrap().wb.active().unwrap().zoom;
    assert!(zoom > 100, "spreading two fingers zoomed in, to {zoom}%");
    assert_eq!(scroll(&h).x, 0.0, "the first finger moving left didn't pan");
}
