//! A document its page host made read-only (`DocAccess`) through real egui input: keys, typing,
//! paste, the fill handle and double-clicks change nothing and open no editor.

use egui::{Event, Key, Modifiers, OutputCommand, PointerButton, Pos2, vec2};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::doc_access::{DocAccess, NO_DOWNLOAD, READ_ONLY};
use gridcraft_ui_egui::{SheetApp, grid::Geo};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };

fn harness(access: DocAccess) -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "B1", "values": [[10], [20], [30]]})).unwrap();
    s.execute("selection.set", json!({"range": "B1:B3"})).unwrap();
    let d = s.active_mut().unwrap();
    d.saved = d.wb.clone();
    d.access = access;
    // Wide enough for the whole Home tab, so its last group can be clicked.
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(2000.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.run_steps(4);
    h
}

fn key(h: &mut Harness, k: Key, m: Modifiers) {
    h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m });
    h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: m });
    h.run_steps(2);
}

fn cell_pos(h: &Harness, cell: CellRef, corner: bool) -> Pos2 {
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    let geo = Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll);
    let r = geo.cell_rect(sheet, cell);
    if corner { r.right_bottom() - vec2(1.0, 1.0) } else { r.center() }
}

fn button(h: &mut Harness, pos: Pos2, pressed: bool) {
    h.input_mut()
        .events
        .extend([Event::PointerMoved(pos), Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    h.step();
}

/// The document is unchanged, its history too, and no editor is open.
fn assert_untouched(h: &Harness, what: &str) {
    let app = h.state();
    let d = app.session.active().unwrap();
    let sh = d.wb.active().unwrap();
    assert!(!d.is_dirty(), "{what} changed the document");
    assert_eq!(d.undo.len(), 1, "{what} changed the history");
    let column: Vec<Value> = (0..5).map(|r| sh.value(CellRef::new(r, 1))).collect();
    assert_eq!(column, [Value::Number(10.0), Value::Number(20.0), Value::Number(30.0), Value::Empty, Value::Empty], "{what}");
    assert!(app.editor.is_none() && app.text_box_editor.is_none(), "{what} opened an editor");
}

#[test]
fn typing_keys_and_paste_change_nothing_on_a_read_only_document() {
    let mut h = harness(DocAccess { read_only: true, no_download: false, commenter: false });
    h.input_mut().events.push(Event::Text("7".into()));
    h.run_steps(2);
    assert_untouched(&h, "typing");
    assert_eq!(h.state().toast.as_ref().map(|(m, _)| m.as_str()), Some(READ_ONLY), "the read-only notice says why");
    key(&mut h, Key::F2, Modifiers::NONE);
    assert_untouched(&h, "F2");
    key(&mut h, Key::Delete, Modifiers::NONE);
    key(&mut h, Key::Backspace, Modifiers::NONE);
    assert_untouched(&h, "Delete and Backspace");
    key(&mut h, Key::Z, CTRL);
    key(&mut h, Key::B, CTRL);
    key(&mut h, Key::D, CTRL);
    assert_untouched(&h, "Ctrl+Z, Ctrl+B, Ctrl+D");
    h.input_mut().events.push(Event::Paste("5\t6".into()));
    h.run_steps(2);
    assert_untouched(&h, "paste");
    // Copying out is still allowed.
    h.input_mut().events.push(Event::Copy);
    h.step();
    assert!(h.output().platform_output.commands.iter().any(|c| matches!(c, OutputCommand::CopyText(_))));
    // Moving around works.
    key(&mut h, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(h.state().session.active().unwrap().selection.a1(), "B2");
}

#[test]
fn the_fill_handle_and_double_click_change_nothing_on_a_read_only_document() {
    let mut h = harness(DocAccess { read_only: true, no_download: false, commenter: false });
    let handle = cell_pos(&h, CellRef::new(2, 1), true);
    let below = cell_pos(&h, CellRef::new(4, 1), false);
    button(&mut h, handle, true);
    for step in 1..=6 {
        let at = handle + (below - handle) * (step as f32 / 6.0);
        h.input_mut().events.push(Event::PointerMoved(at));
        h.step();
    }
    button(&mut h, below, false);
    h.run_steps(2);
    assert_untouched(&h, "dragging the fill handle");
    let cell = cell_pos(&h, CellRef::new(0, 1), false);
    for _ in 0..2 {
        button(&mut h, cell, true);
        button(&mut h, cell, false);
    }
    h.run_steps(2);
    assert_untouched(&h, "double-clicking a cell");
}

fn enabled(h: &Harness, label: &str) -> bool {
    !h.get_by_label(label).accesskit_node().is_disabled()
}

#[test]
fn the_home_tab_keeps_copy_and_find_on_a_read_only_document() {
    let mut h = harness(DocAccess { read_only: true, no_download: false, commenter: false });
    assert_eq!(h.state().ui.ribbon_tab, "Home");
    assert!(enabled(&h, "Copy (⌘C)") && enabled(&h, "Find & Select"));
    for label in ["Paste", "Cut (⌘X)", "Format Painter", "Sort & Filter", "Bold (⌘B)", "Italic (⌘I)", "Conditional Formatting", "Cell Styles"] {
        assert!(!enabled(&h, label), "{label}");
    }
    h.get_by_label("Find & Select").click();
    h.run_steps(3);
    assert!(enabled(&h, "Find…") && enabled(&h, "Go To…") && enabled(&h, "Go To Special…"));
    assert!(!enabled(&h, "Replace…"), "replacing edits");

    let h = harness(DocAccess { read_only: true, no_download: true, commenter: false });
    assert!(!enabled(&h, "Copy (⌘C)"), "copying is a download");
    assert!(enabled(&h, "Find & Select"));
    let h = harness(DocAccess::default());
    for label in ["Paste", "Cut (⌘X)", "Copy (⌘C)", "Format Painter", "Sort & Filter", "Find & Select", "Bold (⌘B)", "Cell Styles"] {
        assert!(enabled(&h, label), "{label} on an editable document");
    }
}

#[test]
fn a_held_edit_reopens_when_its_document_is_active_and_editable_again() {
    let mut h = harness(DocAccess::default());
    let (first, second) = {
        let app = h.state_mut();
        app.session.execute("data.validation", json!({"range": "A1", "type": "list", "formula1": "\"First,Second\""})).unwrap();
        app.session.execute("selection.set", json!({"range": "A1"})).unwrap();
        let first = app.session.active().unwrap().uid;
        app.begin_edit(Some("Third".into()), false);
        app.set_document_access(first, DocAccess { read_only: true, no_download: false, commenter: false });
        assert!(app.editor.is_none() && app.message.is_some());
        app.message = None;
        let i = app.session.new_workbook().unwrap();
        (first, app.session.documents()[i].uid)
    };
    h.run_steps(2);
    h.state_mut().set_document_access(first, DocAccess::default());
    h.run_steps(2);
    assert!(h.state().editor.is_none(), "not while another document is in front");
    assert_eq!(h.state().session.active().unwrap().uid, second);

    let i = h.state().document_index(Some(first)).unwrap();
    h.state_mut().session.set_active(i);
    h.run_steps(2);
    let app = h.state();
    let edit = app.editor.as_ref().expect("reopened on the next frame");
    assert_eq!((edit.cell, edit.text.as_str()), (CellRef::new(0, 0), "Third"));
    assert!(app.session.active().unwrap().wb.active().unwrap().value(CellRef::new(0, 0)).is_empty());
}

/// A page host that stores every open document and shares them itself.
struct SharingHost;

impl gridcraft_ui_egui::host_docs::DocumentHost for SharingHost {
    fn hosts(&self, _uid: u64) -> bool {
        true
    }

    fn save(&self, _app: &mut SheetApp, _uid: u64, _drop_macros: bool) {}

    fn shares(&self) -> bool {
        true
    }
}

#[test]
fn the_title_bars_share_is_hidden_when_the_host_shares_the_document() {
    let mut h = harness(DocAccess::default());
    assert!(h.query_by_label("Share").is_some(), "no host: the fork's own Share");
    h.state_mut().document_host = Some(std::rc::Rc::new(SharingHost));
    h.run_steps(2);
    assert!(h.query_by_label("Share").is_none(), "the shell's Share is the only one");
}

#[test]
fn copy_is_refused_when_downloads_are_off() {
    let mut h = harness(DocAccess { read_only: true, no_download: true, commenter: false });
    // Cut is an edit too, so the read-only notice wins there.
    for (event, notice) in [(Event::Copy, NO_DOWNLOAD), (Event::Cut, READ_ONLY)] {
        h.input_mut().events.push(event);
        h.step();
        assert!(!h.output().platform_output.commands.iter().any(|c| matches!(c, OutputCommand::CopyText(_))));
        assert!(h.state().session.clipboard.is_none());
        assert_eq!(h.state().toast.as_ref().map(|(m, _)| m.as_str()), Some(notice));
    }
    assert_untouched(&h, "copy and cut");
}
