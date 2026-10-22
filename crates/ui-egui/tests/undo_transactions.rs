use egui::{Rect, vec2};
use egui_kittest::kittest::{NodeT as _, Queryable};
use gridcraft_engine::Session;
use gridcraft_ui_egui::{SheetApp, drafts};
use serde_json::json;

#[test]
fn touch_and_narrow_history_targets_are_reachable_and_disabled() {
    for width in [320.0, 375.0, 390.0, 1200.0] {
        let mut s = Session::new();
        s.new_workbook();
        let mut app = SheetApp::new(s, Default::default());
        app.grid.touch.active = width >= 600.0;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(width, 700.0)).wgpu().build_ui_state(
            |ui, app: &mut SheetApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
        h.run_steps(3);
        let layout = || h.ctx.data(|d| d.get_temp::<(Rect, Rect, bool, bool)>(egui::Id::new("gridcraft.history_layout"))).unwrap();
        let (undo, redo, undo_enabled, redo_enabled) = layout();
        assert!(!undo_enabled && !redo_enabled);
        assert!(h.get_by_label("Undo (⌘Z)").accesskit_node().is_disabled());
        assert!(h.get_by_label("Redo (⌘Y)").accesskit_node().is_disabled());
        for rect in [undo, redo] {
            assert!(rect.width() >= 44.0 && rect.height() >= 44.0);
            assert!(rect.left() >= 0.0 && rect.right() <= width);
        }
        assert!(!undo.intersects(redo));
        h.state_mut().run("cell.set", json!({"cell":"A1", "input":"7"})).unwrap();
        h.state_mut().run("edit.undo", json!({})).unwrap();
        h.run_steps(3);
        let (_, redo, undo_enabled, redo_enabled) =
            h.ctx.data(|d| d.get_temp::<(Rect, Rect, bool, bool)>(egui::Id::new("gridcraft.history_layout"))).unwrap();
        assert!(!undo_enabled && redo_enabled);
        assert!(!h.get_by_label("Redo (⌘Y)").accesskit_node().is_disabled());
        assert!(redo.right() <= width);
        if let Some(dir) = std::env::var_os("GRIDCRAFT_CALC_SCREENSHOTS") {
            std::fs::create_dir_all(&dir).unwrap();
            h.render().unwrap().save(std::path::Path::new(&dir).join(format!("undo-redo-{width:.0}.png"))).unwrap();
        }
    }
}

#[test]
fn draft_and_explicit_save_preserve_history_and_undo_marks_dirty() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell":"A1", "input":"before"})).unwrap();
    s.execute("cell.set", json!({"cell":"A1", "input":"after"})).unwrap();
    let d = s.doc().unwrap();
    let count = d.undo.len();
    let doc = drafts::DraftDoc { uid: d.uid, wb: d.wb.clone(), saved: d.saved.clone(), unsaved: true, name: "Book.xlsx".into() };
    let mut planner = drafts::Planner::default();
    planner.plan(std::slice::from_ref(&doc), 0.0, false);
    assert_eq!(planner.plan(std::slice::from_ref(&doc), drafts::IDLE_MS + 1.0, false).put, vec![doc.uid]);
    let bytes = drafts::draft_bytes(&doc.wb, &doc.name).unwrap();
    assert!(!bytes.is_empty());
    planner.record_put(doc.uid, &doc.wb, drafts::IDLE_MS + 1.0, 1.0);
    assert_eq!(s.doc().unwrap().undo.len(), count);
    let path = std::env::temp_dir().join(format!("gridcraft-undo-save-{}-{}.xlsx", std::process::id(), doc.uid));
    s.execute("file.save", json!({"path":path.to_string_lossy()})).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
    assert_eq!(s.doc().unwrap().undo.len(), count);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.doc().unwrap().is_dirty());
    assert_eq!(s.doc().unwrap().wb.active().unwrap().value(gridcraft_engine::cell(0, 0)), gridcraft_engine::core::Value::text("before"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn autosave_timer_preserves_history() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell":"A1", "input":"before"})).unwrap();
    let path = std::env::temp_dir().join(format!("gridcraft-undo-autosave-{}-{}.xlsx", std::process::id(), s.doc().unwrap().uid));
    s.execute("file.save", json!({"path":path.to_string_lossy()})).unwrap();
    s.execute("cell.set", json!({"cell":"A1", "input":"after"})).unwrap();
    let count = s.doc().unwrap().undo.len();
    let mut app = SheetApp::new(s, Default::default());
    app.grid.autosave = true;
    app.grid.last_autosave = gridcraft_ui_egui::now_ms() - 3000.0;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    assert!(!h.state().session.doc().unwrap().is_dirty());
    assert_eq!(h.state().session.doc().unwrap().undo.len(), count);
    h.state_mut().grid.autosave = false;
    h.state_mut().run("edit.undo", json!({})).unwrap();
    let d = h.state().session.doc().unwrap();
    assert!(d.is_dirty());
    assert_eq!(d.wb.active().unwrap().value(gridcraft_engine::cell(0, 0)), gridcraft_engine::core::Value::text("before"));
    let mut reopened = Session::new();
    reopened.execute("file.open", json!({"path":path.to_string_lossy()})).unwrap();
    assert_eq!(reopened.doc().unwrap().wb.active().unwrap().value(gridcraft_engine::cell(0, 0)), gridcraft_engine::core::Value::text("after"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn disabled_history_widget_has_no_hover_or_pressed_response() {
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(200.0, 100.0))),
        events: vec![
            egui::Event::PointerMoved(egui::pos2(10.0, 10.0)),
            egui::Event::PointerButton {
                pos: egui::pos2(10.0, 10.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        let response =
            gridcraft_ui_egui::widgets::enabled_icon_button(ui, false, gridcraft_ui_egui::icons::Icon::Undo, "Undo (⌘Z)", vec2(44.0, 44.0));
        assert!(!response.enabled());
        assert!(!response.hovered());
        assert!(!response.is_pointer_button_down_on());
        assert!(!response.clicked());
    });
    // Debug builds panic when a frame's texture deltas are dropped unapplied.
    output.textures_delta.clear();
}
