//! Share… (web, with the office shell's share sheet): the first item of the title bar's Share
//! menu hands the workbook's Save bytes to the shell, and downloads them when it declines.

use std::sync::{Arc, Mutex};

use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::io::{base64_encode, open_bytes, save_bytes};
use gridcraft_ui_egui::save_as::{DROP_MACROS, SAVE_AS_XLSM};
use gridcraft_ui_egui::share::SHARE;
use gridcraft_ui_egui::{Delivery, DeliveryReport, Services, SheetApp};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;
type Files = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

const VBA: &[u8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1 synthetic vbaProject";

struct Web {
    h: Harness,
    shared: Files,
    downloaded: Files,
}

/// The web build with downloads and, when `share` is `Some`, a share sheet answering with it.
fn web(share: Option<bool>) -> Web {
    let (shared, downloaded): (Files, Files) = Default::default();
    let (to_share, to_download) = (shared.clone(), downloaded.clone());
    let services = Services {
        download: Some(Box::new(move |name: &str, bytes: &[u8], report: DeliveryReport| {
            to_download.lock().unwrap().push((name.to_string(), bytes.to_vec()));
            report.settle(Delivery::Delivered);
        })),
        share: share.map(|taken| -> Box<dyn Fn(&str, &[u8]) -> bool> {
            Box::new(move |name: &str, bytes: &[u8]| {
                to_share.lock().unwrap().push((name.to_string(), bytes.to_vec()));
                taken
            })
        }),
        ..Default::default()
    };
    let mut session = Session::new();
    session.new_workbook();
    session.execute("cell.set", json!({"cell": "A1", "input": "shared value"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, services),
    );
    h.run_steps(4);
    Web { h, shared, downloaded }
}

fn open_share_menu(h: &mut Harness) {
    h.get_by_label("Share").click();
    h.run_steps(3);
}

fn click(h: &mut Harness, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn a1(name: &str, bytes: &[u8]) -> Value {
    open_bytes(name, bytes).unwrap().0.active().unwrap().value(CellRef::parse("A1").unwrap())
}

#[test]
fn share_is_not_offered_without_the_share_service() {
    let mut web = web(None);
    open_share_menu(&mut web.h);
    assert!(web.h.query_by_label("Export as PDF…").is_some(), "the menu is open");
    assert!(web.h.query_by_label(SHARE).is_none());
}

#[test]
fn share_hands_the_workbook_bytes_to_the_shell() {
    let mut web = web(Some(true));
    open_share_menu(&mut web.h);
    click(&mut web.h, SHARE);
    let shared = web.shared.lock().unwrap().clone();
    assert_eq!(shared.len(), 1);
    let (name, bytes) = &shared[0];
    assert_eq!(name, "Book1.xlsx");
    assert_eq!(a1(name, bytes), Value::Text("shared value".into()), "the shared bytes reopen as the workbook");
    assert!(web.downloaded.lock().unwrap().is_empty(), "the shell took it");
    assert!(web.h.state().session.active().unwrap().is_dirty(), "a shared copy doesn't save the workbook");
}

#[test]
fn a_declined_share_downloads_the_same_copy() {
    let mut web = web(Some(false));
    open_share_menu(&mut web.h);
    click(&mut web.h, SHARE);
    let shared = web.shared.lock().unwrap().clone();
    let downloaded = web.downloaded.lock().unwrap().clone();
    assert_eq!(downloaded, shared, "the download is what the shell declined");
    assert_eq!(downloaded.len(), 1);
    assert_eq!(a1(&downloaded[0].0, &downloaded[0].1), Value::Text("shared value".into()));
    assert!(web.h.state().session.active().unwrap().is_dirty(), "a copy, not a save");
}

/// A workbook with a VBA project that Share would write as `.xlsx`, as an imported `.xls` is.
fn open_macros_named_xlsx(h: &mut Harness) {
    let mut wb = gridcraft_engine::model::Workbook::new();
    wb.vba_project = Some(VBA.to_vec());
    let xlsm = save_bytes(&wb, "macro.xlsm").unwrap();
    let r = h.state_mut().run("file.open", json!({"name": "macro.xlsm", "base64": base64_encode(&xlsm)})).unwrap();
    h.state_mut().after_open(None, &r);
    h.state_mut().message = None;
    h.state_mut().session.active_mut().unwrap().title = "macro.xlsx".into();
    h.run_steps(2);
}

/// Activates `label` as Enter, Space or a screen reader does: no pointer click.
fn activate_without_pointer(h: &mut Harness, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}

#[test]
fn share_without_a_pointer_click_downloads_instead() {
    let mut web = web(Some(true));
    open_share_menu(&mut web.h);
    activate_without_pointer(&mut web.h, SHARE);
    assert!(web.shared.lock().unwrap().is_empty(), "the browser would refuse a share sheet outside the click");
    let downloaded = web.downloaded.lock().unwrap().clone();
    assert_eq!(downloaded.len(), 1);
    assert_eq!(downloaded[0].0, "Book1.xlsx");
    assert_eq!(a1(&downloaded[0].0, &downloaded[0].1), Value::Text("shared value".into()));
}

#[test]
fn the_macro_question_answered_without_a_pointer_click_downloads_instead() {
    let mut web = web(Some(true));
    open_macros_named_xlsx(&mut web.h);
    open_share_menu(&mut web.h);
    click(&mut web.h, SHARE);
    activate_without_pointer(&mut web.h, SAVE_AS_XLSM);
    assert!(web.shared.lock().unwrap().is_empty());
    let downloaded = web.downloaded.lock().unwrap().clone();
    assert_eq!(downloaded.len(), 1);
    assert_eq!(downloaded[0].0, "macro.xlsm");
}

#[test]
fn sharing_a_workbook_that_would_lose_its_macros_asks_and_the_answer_shares() {
    let mut web = web(Some(true));
    open_macros_named_xlsx(&mut web.h);

    open_share_menu(&mut web.h);
    click(&mut web.h, SHARE);
    assert_eq!(web.h.state().dialog.as_ref().map(|d| d.name().to_string()).as_deref(), Some(DROP_MACROS));
    assert!(web.shared.lock().unwrap().is_empty(), "nothing is shared before the answer");

    click(&mut web.h, SAVE_AS_XLSM);
    let shared = web.shared.lock().unwrap().clone();
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].0, "macro.xlsm");
    assert_eq!(open_bytes(&shared[0].0, &shared[0].1).unwrap().0.vba_project.as_deref(), Some(VBA));
    assert!(web.downloaded.lock().unwrap().is_empty());
}
