//! The PivotTable Fields pane keeps every button of its bottom row inside the pane, on a phone and
//! in a wide window, in English and in Vietnamese (whose labels are longer).

use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::i18n::Language;

type Harness = egui_kittest::Harness<'static, SheetApp>;

const PANE_WIDTH: f32 = 280.0;

fn pivot_harness(size: egui::Vec2, language: Language) -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.language = language;
    let mut h = Harness::builder().with_size(size).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    let st = h.state_mut();
    st.run("range.setValues", serde_json::json!({"range": "A1:B3", "values": [["Region", "Amount"], ["North", 10], ["South", 20]]})).unwrap();
    st.run("insert.pivotTable", serde_json::json!({"source": "Sheet1!A1:B3", "destination": "Sheet1!E3", "rows": ["Region"], "values": ["Amount"]}))
        .unwrap();
    st.run("selection.set", serde_json::json!({"cell": "E3"})).unwrap();
    h.run_steps(4);
    h
}

#[test]
fn every_pivot_pane_button_stays_inside_the_pane() {
    for (size, narrow) in [(egui::vec2(390.0, 844.0), true), (egui::vec2(1366.0, 768.0), false)] {
        for language in [Language::En, Language::Vi] {
            let h = pivot_harness(size, language);
            assert!(h.query_by_label(language.tr("PivotTable Fields")).is_some(), "the pane is open ({size:?}, {language:?})");
            let pane = if narrow {
                h.ctx.memory(|m| m.area_rect(egui::Id::new(("pivot_pane", "floating")))).expect("the floating pane")
            } else {
                egui::containers::panel::PanelState::load(&h.ctx, egui::Id::new("pivot_pane")).expect("the docked pane").outer_rect
            };
            assert!(pane.width() <= PANE_WIDTH + 1.0, "the buttons don't push the pane wider ({size:?}, {language:?}): {pane:?}");
            for label in ["Refresh", "Calculated Field…", "Report Layout"] {
                let button = h.get_by_label_contains(language.tr(label)).rect();
                assert!(
                    button.left() >= pane.left() && button.right() <= pane.right() && button.right() <= size.x,
                    "{label} sticks out of the pane ({size:?}, {language:?}): {button:?} in {pane:?}"
                );
            }
        }
    }
}

#[test]
fn report_layout_still_opens_its_menu_and_applies_the_choice() {
    let mut h = pivot_harness(egui::vec2(390.0, 844.0), Language::Vi);
    h.get_by_label_contains("Bố trí Báo cáo").click();
    h.run_steps(3);
    h.get_by_label("Dạng bảng").click();
    h.run_steps(3);
    let layout = h.state().session.active().unwrap().wb.active().unwrap().pivots[0].layout;
    assert_eq!(layout, gridcraft_engine::model::PivotLayout::Tabular);
}
