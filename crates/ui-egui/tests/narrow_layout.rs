//! Narrow windows: the ribbon folds once when the window gets phone-narrow, the user can open it
//! again, and a wide window gets its own choice back. Task panes float over the sheet there, and
//! the ribbon's tab row scrolls to the tabs past the edge.

use egui::{Event, Modifiers, PointerButton, Pos2, TouchDeviceId, TouchId, TouchPhase, pos2, vec2};
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::ribbon::TABS;

type Harness = egui_kittest::Harness<'static, SheetApp>;

const PHONE: egui::Vec2 = egui::Vec2::new(390.0, 844.0);
const TABLET: egui::Vec2 = egui::Vec2::new(768.0, 1024.0);
const DESKTOP: egui::Vec2 = egui::Vec2::new(1366.0, 768.0);

fn harness(size: egui::Vec2) -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
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

fn resize(h: &mut Harness, size: egui::Vec2) {
    h.set_size(size);
    h.run_steps(3);
}

fn collapsed(h: &Harness) -> bool {
    h.state().ui.ribbon_collapsed
}

fn click(h: &mut Harness, at: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(at));
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// The height of the ribbon's tab row, half way down.
fn tab_row_y(h: &Harness) -> f32 {
    let ribbon = egui::containers::panel::PanelState::load(&h.ctx, egui::Id::new("ribbon")).expect("the ribbon panel");
    ribbon.outer_rect.top() + 16.0
}

#[test]
fn the_ribbon_folds_when_the_window_narrows_and_comes_back_when_it_widens() {
    let mut h = harness(DESKTOP);
    assert!(!collapsed(&h));
    resize(&mut h, TABLET);
    assert!(!collapsed(&h), "a tablet in portrait keeps the ribbon");
    resize(&mut h, PHONE);
    assert!(collapsed(&h), "a phone folds it");
    assert!(h.state().narrow.is_narrow());
    // Opening it again sticks while the window stays narrow.
    h.state_mut().run("view.collapseRibbon", serde_json::json!({})).unwrap();
    assert!(!collapsed(&h));
    resize(&mut h, vec2(420.0, 844.0));
    assert!(!collapsed(&h), "the user's expand holds through narrow resizes");
    // The wide window gets the choice it had; a folded one is saved as such meanwhile.
    h.state_mut().run("view.collapseRibbon", serde_json::json!({})).unwrap();
    assert!(collapsed(&h));
    assert!(!h.state().saved_ui().ribbon_collapsed, "ui.json keeps the desktop choice");
    resize(&mut h, DESKTOP);
    assert!(!collapsed(&h));
}

#[test]
fn a_desktop_choice_to_fold_survives_a_trip_through_narrow() {
    let mut h = harness(DESKTOP);
    h.state_mut().run("view.collapseRibbon", serde_json::json!({})).unwrap();
    h.run_steps(2);
    resize(&mut h, PHONE);
    h.state_mut().run("view.collapseRibbon", serde_json::json!({})).unwrap();
    assert!(!collapsed(&h));
    resize(&mut h, DESKTOP);
    assert!(collapsed(&h));
}

#[test]
fn a_task_pane_floats_over_the_sheet_when_narrow() {
    let mut h = harness(PHONE);
    let full = h.state().grid.rect.unwrap().width();
    h.state_mut().grid.pane = Some("watch".into());
    h.run_steps(3);
    assert_eq!(h.state().grid.rect.unwrap().width(), full, "the grid keeps its width under the pane");

    let mut wide = harness(DESKTOP);
    let full = wide.state().grid.rect.unwrap().width();
    wide.state_mut().grid.pane = Some("watch".into());
    wide.run_steps(3);
    assert!(wide.state().grid.rect.unwrap().width() < full - 250.0, "a wide window still docks the pane beside the grid");
}

#[test]
fn the_tab_row_scrolls_to_the_tabs_past_the_edge() {
    let mut h = harness(PHONE);
    let y = tab_row_y(&h);
    let edge = pos2(PHONE.x - 30.0, y);
    // egui hit-tests against the last frame, so a scroll area takes drags only from the second
    // touch on: the first one is this tap.
    tap(&mut h, edge);
    let before = TABS.iter().position(|t| *t == h.state().ui.ribbon_tab).unwrap();
    assert!(before + 1 < TABS.len(), "some tabs start past the right edge (last visible: {})", TABS[before]);
    let from = pos2(PHONE.x - 40.0, y);
    h.input_mut().events.extend([press(from, true), finger(TouchPhase::Start, from)]);
    h.step();
    for i in 1..=10 {
        let p = from - vec2(30.0 * i as f32, 0.0);
        h.input_mut().events.extend([Event::PointerMoved(p), finger(TouchPhase::Move, p)]);
        h.step();
    }
    let end = from - vec2(300.0, 0.0);
    h.input_mut().events.extend([press(end, false), Event::PointerGone, finger(TouchPhase::End, end)]);
    h.run_steps(60);
    tap(&mut h, edge);
    let after = TABS.iter().position(|t| *t == h.state().ui.ribbon_tab).unwrap();
    assert!(after > before, "the row scrolled: {} → {}", TABS[before], TABS[after]);
}

#[test]
fn a_mouse_drag_leaves_the_tab_row_alone() {
    let mut h = harness(PHONE);
    let y = tab_row_y(&h);
    let edge = pos2(PHONE.x - 30.0, y);
    click(&mut h, edge);
    let before = h.state().ui.ribbon_tab.clone();
    let from = pos2(PHONE.x - 40.0, y);
    h.input_mut().events.push(Event::PointerMoved(from));
    h.step();
    h.input_mut().events.push(press(from, true));
    h.step();
    for i in 1..=10 {
        h.input_mut().events.push(Event::PointerMoved(from - vec2(30.0 * i as f32, 0.0)));
        h.step();
    }
    h.input_mut().events.push(press(from - vec2(300.0, 0.0), false));
    h.run_steps(10);
    click(&mut h, edge);
    click(&mut h, edge);
    assert_eq!(h.state().ui.ribbon_tab, before, "only a finger drags the row; the mouse wheel scrolls it");
}

fn press(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn finger(phase: TouchPhase, pos: Pos2) -> Event {
    Event::Touch { device_id: TouchDeviceId(1), id: TouchId(0), phase, pos, force: None }
}

fn tap(h: &mut Harness, at: Pos2) {
    h.input_mut().events.push(press(at, true));
    h.step();
    h.input_mut().events.extend([press(at, true), finger(TouchPhase::Start, at)]);
    h.step();
    h.input_mut().events.extend([press(at, false), Event::PointerGone, finger(TouchPhase::End, at)]);
    h.run_steps(2);
}

type Layout3 = (egui::Rect, egui::Rect, Option<egui::Rect>);

fn title_layout(h: &Harness) -> Layout3 {
    h.ctx.data(|d| d.get_temp::<Layout3>(egui::Id::new(gridcraft_ui_egui::ribbon::TITLE_BAR_LAYOUT))).expect("title bar layout")
}

#[test]
fn the_title_bar_never_overlaps_in_a_narrow_window() {
    let mut h = harness(PHONE);
    h.state_mut().session.active_mut().unwrap().title = "Báo cáo doanh thu quý bốn của chi nhánh miền Nam.xlsx".into();
    let wide_right = {
        resize(&mut h, DESKTOP);
        title_layout(&h).1.width()
    };
    for width in [599.0, 480.0, 430.0, 390.0, 360.0, 320.0] {
        resize(&mut h, vec2(width, 844.0));
        let (left, right, title) = title_layout(&h);
        assert!(left.right() <= right.left(), "controls overlap at {width}: {left:?} {right:?}");
        assert!(right.right() <= width + 0.5, "the right group stays on screen at {width}");
        if let Some(title) = title {
            assert!(title.left() >= left.right() && title.right() <= right.left(), "the title sits between them at {width}: {title:?}");
        }
        if width <= 430.0 {
            assert!(right.width() < wide_right, "the right controls gave way at {width}");
        }
    }
    resize(&mut h, PHONE);
    assert!(title_layout(&h).2.is_some_and(|t| t.width() >= 40.0), "a phone still shows the title, elided");
}

#[test]
fn the_status_bar_totals_never_cover_the_notes() {
    let mut h = harness(PHONE);
    let st = h.state_mut();
    st.run("range.setValues", serde_json::json!({"range": "A1:C3", "values": [[1234567, 2345678, 3456789], [4, 5, 6], [7, 8, 9]]})).unwrap();
    st.run("selection.set", serde_json::json!({"range": "A1:C3"})).unwrap();
    h.run_steps(3);
    for width in [599.0, 430.0, 390.0, 360.0, 320.0] {
        resize(&mut h, vec2(width, 844.0));
        let (left, right) = h
            .ctx
            .data(|d| d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new(gridcraft_ui_egui::tabs::STATUS_BAR_LAYOUT)))
            .expect("status bar layout");
        assert!(left.right() <= right.left(), "the totals reach over the notes at {width}: {left:?} {right:?}");
        assert!(h.query_by_label_contains("Sum:").is_some(), "the sum stays in view at {width}");
    }
}

#[test]
fn a_narrow_pivot_pane_closes_until_the_cell_leaves_the_pivot() {
    let mut h = harness(PHONE);
    let st = h.state_mut();
    st.run("range.setValues", serde_json::json!({"range": "A1:B4", "values": [["Region", "Amount"], ["North", 10], ["South", 20], ["North", 5]]}))
        .unwrap();
    st.run("insert.pivotTable", serde_json::json!({"source": "Sheet1!A1:B4", "destination": "Sheet1!E3", "rows": ["Region"], "values": ["Amount"]}))
        .unwrap();
    let select = |h: &mut Harness, cell: &str| {
        h.state_mut().run("selection.set", serde_json::json!({"cell": cell})).unwrap();
        h.run_steps(3);
    };
    select(&mut h, "E3");
    assert!(h.query_by_label("PivotTable Fields").is_some(), "the pane opens in the pivot");
    let title = h.get_by_label("PivotTable Fields").rect();
    click(&mut h, pos2(PHONE.x - 21.0, title.center().y));
    assert!(h.query_by_label("PivotTable Fields").is_none(), "the close button closes it");
    select(&mut h, "E4");
    assert!(h.query_by_label("PivotTable Fields").is_none(), "it stays closed within the same pivot");
    select(&mut h, "A1");
    select(&mut h, "E3");
    assert!(h.query_by_label("PivotTable Fields").is_some(), "coming back to the pivot opens it again");
}

#[test]
fn a_wide_window_keeps_the_pivot_pane_docked() {
    let mut h = harness(DESKTOP);
    let st = h.state_mut();
    st.run("range.setValues", serde_json::json!({"range": "A1:B3", "values": [["Region", "Amount"], ["North", 10], ["South", 20]]})).unwrap();
    st.run("insert.pivotTable", serde_json::json!({"source": "Sheet1!A1:B3", "destination": "Sheet1!E3", "rows": ["Region"], "values": ["Amount"]}))
        .unwrap();
    let full = h.state().grid.rect.unwrap().width();
    h.state_mut().run("selection.set", serde_json::json!({"cell": "E3"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("PivotTable Fields").is_some());
    assert!(h.state().grid.rect.unwrap().width() < full - 200.0, "docked beside the grid");
    h.state_mut().grid.pivot_pane_closed = Some((h.state().session.active().unwrap().uid, 0, "PivotTable1".into()));
    h.run_steps(2);
    assert!(h.query_by_label("PivotTable Fields").is_some(), "only a narrow window lets it be closed");
}
