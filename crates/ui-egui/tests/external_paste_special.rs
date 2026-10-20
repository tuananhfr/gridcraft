//! Paste Special reads host clipboard content before choosing the internal or external form.

use std::cell::Cell;
use std::rc::Rc;

use egui::Event;
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_ui_egui::control::{Outcome, handle};
use gridcraft_ui_egui::{Services, SheetApp, dialogs::Field, i18n::Language};
use serde_json::{Value as Json, json};

const HTML: &str = "<table><tr><td colspan=2 style='font-weight:bold;background:#abcdef' x:num=42>42</td></tr></table>";

fn app(services: Services) -> SheetApp {
    let mut s = Session::new();
    s.new_workbook();
    SheetApp::new(s, services)
}

fn host() -> Services {
    Services {
        paste_text: Some(Box::new(|| Some("external".into()))),
        paste_html: Some(Box::new(|| Some(HTML.into()))),
        ..Default::default()
    }
}

fn value(app: &SheetApp, at: &str) -> Value {
    app.session.active().unwrap().wb.active().unwrap().value(CellRef::parse(at).unwrap())
}

fn control(app: &mut SheetApp, method: &str, params: Json) {
    let result = match handle(app, &egui::Context::default(), method, &params) {
        Outcome::Done(result) => result,
        Outcome::Screenshot { .. } => json!({"ok": false}),
    };
    assert_eq!(result["ok"], true, "{result}");
}

#[test]
fn external_dialog_snapshots_host_content_and_offers_only_external_choices() {
    let mut app = app(host());
    app.open_dialog("pasteSpecial", json!({}));
    let d = app.dialog.as_ref().unwrap();
    assert_eq!(d.values["source"], "external");
    assert_eq!(d.values["html"], HTML);
    assert_eq!(d.values["text"], "external");
    assert_eq!(d.values["what"], "keepSource");
    assert!(matches!(d.fields.as_slice(), [Field::Choice { key: "what", options, .. }]
        if options.iter().map(|(id, _)| *id).collect::<Vec<_>>() == ["keepSource", "matchDestination", "values"]));
    control(&mut app, "ui.dialog.set", json!({"field":"what", "value":"matchDestination"}));
    control(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(value(&app, "A1"), Value::number(42.0));
    let d = app.session.active().unwrap();
    let sh = d.wb.active().unwrap();
    assert!(!d.wb.styles.get(sh.style_id(CellRef::new(0, 0))).font.bold);
    assert!(sh.merges.is_empty());
}

#[test]
fn external_ribbon_choices_read_host_and_keep_source_or_destination_styles() {
    for what in ["keepSource", "matchDestination", "values"] {
        let mut app = app(host());
        app.run("home.italic", json!({"range":"A1"})).unwrap();
        app.paste_special_from_clipboard(what);
        assert_eq!(value(&app, "A1"), Value::number(42.0));
        let d = app.session.active().unwrap();
        let sh = d.wb.active().unwrap();
        let style = d.wb.styles.get(sh.style_id(CellRef::new(0, 0)));
        assert_eq!(style.font.bold, what == "keepSource");
        assert_eq!(style.font.italic, what != "keepSource");
        assert_eq!(sh.merges.is_empty(), what != "keepSource");
    }
}

#[test]
fn internal_dialog_keeps_original_operations_and_mnemonics() {
    let mut app = app(Services::default());
    app.run("cell.set", json!({"input":"=21*2"})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    app.run("selection.set", json!({"cell":"D5"})).unwrap();
    app.open_dialog("pasteSpecial", json!({}));
    let d = app.dialog.as_ref().unwrap();
    assert_eq!(d.values["what"], "all");
    assert!(d.fields.iter().any(|f| matches!(f, Field::Choice { key: "operation", .. })));
    assert!(d.fields.iter().any(|f| matches!(f, Field::Check { key: "transpose", .. })));
    control(&mut app, "ui.dialog.confirm", json!({}));
    assert!(app.session.active().unwrap().wb.active().unwrap().cell(CellRef::parse("D5").unwrap()).unwrap().formula.is_some());
}

#[test]
fn matching_internal_clipboard_keeps_formula_despite_stale_host_html() {
    let mut app = app(Services {
        paste_text: Some(Box::new(|| Some("42\r\n".into()))),
        paste_html: Some(Box::new(|| Some("<table><tr><td>stale</td></tr></table>".into()))),
        ..Default::default()
    });
    app.run("cell.set", json!({"input":"=21*2"})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    app.run("selection.set", json!({"cell":"D5"})).unwrap();
    app.open_dialog("pasteSpecial", json!({}));
    assert_eq!(app.dialog.as_ref().unwrap().values["what"], "all");
    control(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(value(&app, "D5"), Value::number(42.0));
    assert!(app.session.active().unwrap().wb.active().unwrap().cell(CellRef::parse("D5").unwrap()).unwrap().formula.is_some());
}

#[test]
fn html_only_external_special_overrides_old_internal_clipboard() {
    let mut app = app(Services {
        paste_html: Some(Box::new(|| Some(HTML.into()))),
        ..Default::default()
    });
    app.run("cell.set", json!({"input":"old copy"})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    app.run("selection.set", json!({"cell":"D5"})).unwrap();
    app.paste_special_from_clipboard("keepSource");
    assert_eq!(value(&app, "D5"), Value::number(42.0));
}

#[test]
fn web_special_waits_for_clipboard_event_then_opens_dialog_without_pasting() {
    let requests = Rc::new(Cell::new(0));
    let received = requests.clone();
    let mut app = app(Services {
        request_clipboard: Some(Box::new(move || received.set(received.get() + 1))),
        paste_html: Some(Box::new(|| Some(HTML.into()))),
        ..Default::default()
    });
    app.open_dialog("pasteSpecial", json!({}));
    assert_eq!(requests.get(), 1);
    assert!(app.dialog.is_none());
    app.paste_external("external".into());
    assert_eq!(app.dialog.as_ref().unwrap().values["source"], "external");
    assert_eq!(value(&app, "A1"), Value::Empty);
    control(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(value(&app, "A1"), Value::number(42.0));
}

#[test]
fn web_special_values_applies_once_after_async_event() {
    let mut app = app(Services {
        request_clipboard: Some(Box::new(|| {})),
        paste_html: Some(Box::new(|| Some(HTML.into()))),
        ..Default::default()
    });
    app.paste_special_from_clipboard("values");
    assert_eq!(value(&app, "A1"), Value::Empty);
    app.paste_external("external".into());
    assert_eq!(value(&app, "A1"), Value::number(42.0));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(value(&app, "A1"), Value::Empty);
}

#[test]
fn paste_and_typed_vi_formulas_store_the_same_expression() {
    let mut typed = app(Services::default());
    let mut pasted = app(Services::default());
    for app in [&mut typed, &mut pasted] {
        app.run("app.region.set", json!({"region":"vi-VN"})).unwrap();
        app.run("range.setValues", json!({"range":"A1", "values":[[2,3]]})).unwrap();
        app.run("selection.set", json!({"cell":"C1"})).unwrap();
    }
    typed.begin_edit(Some("=SUM(A1;B1)".into()), false);
    assert!(typed.commit_edit(0, 0, false, false));
    pasted.paste_external("=SUM(A1;B1)".into());
    let at = CellRef::parse("C1").unwrap();
    let formula = |app: &SheetApp| app.session.active().unwrap().wb.active().unwrap().cell(at).unwrap().formula.clone();
    assert_eq!(formula(&typed), formula(&pasted));
    assert_eq!(value(&pasted, "C1"), Value::number(5.0));
}

#[test]
fn external_labels_translate_to_vietnamese_and_english() {
    for (en, vi) in [
        ("Keep Source Formatting", "Giữ định dạng nguồn"),
        ("Match Destination Formatting", "Theo định dạng đích"),
        ("Values Only", "Chỉ giá trị"),
    ] {
        assert_eq!(Language::En.tr(en), en);
        assert_eq!(Language::Vi.tr(en), vi);
    }
}

#[test]
fn external_dialog_ignores_internal_paste_mnemonics() {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app(host()),
    );
    h.run_steps(4);
    h.state_mut().open_dialog("pasteSpecial", json!({}));
    h.run_steps(2);
    h.input_mut().events.push(Event::Key {
        key: egui::Key::F,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    });
    h.step();
    let d = h.state().dialog.as_ref().unwrap();
    assert_eq!(d.values["what"], "keepSource");
    assert!(!d.values.contains_key("transpose"));
}
