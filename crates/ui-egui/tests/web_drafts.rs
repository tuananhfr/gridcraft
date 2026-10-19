//! Web draft autosave over the real app: what the planner is told and when it puts.

use std::sync::Arc;

use gridcraft_engine::Session;
use gridcraft_ui_egui::drafts::Planner;
use gridcraft_ui_egui::{Delivery, DeliveryReport, Services, SheetApp};
use serde_json::json;

type Reports = Arc<std::sync::Mutex<Vec<DeliveryReport>>>;

fn harness(services: Services) -> egui_kittest::Harness<'static, SheetApp> {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, services),
    );
    h.run_steps(4);
    h
}

/// A harness whose web downloads wait for the test to settle them.
fn web_harness() -> (egui_kittest::Harness<'static, SheetApp>, Reports) {
    let reports: Reports = Default::default();
    let capture = reports.clone();
    let h = harness(Services { download: Some(Box::new(move |_, _, report| capture.lock().unwrap().push(report))), ..Default::default() });
    (h, reports)
}

/// One planner tick at `now`, recording every put as the web app does.
fn tick(h: &egui_kittest::Harness<'static, SheetApp>, p: &mut Planner, now: f64) -> (Vec<u64>, Vec<String>) {
    let docs = h.state().draft_docs();
    let plan = p.plan(&docs, now, h.state().editing_in_place());
    for uid in &plan.put {
        let d = docs.iter().find(|d| d.uid == *uid).unwrap();
        gridcraft_ui_egui::drafts::draft_bytes(&d.wb).unwrap();
        p.record_put(*uid, &d.wb, now, 1.0);
    }
    (plan.put, plan.remove)
}

#[test]
fn an_idle_workbook_is_drafted_once_however_many_frames_pass() {
    let (mut h, _) = web_harness();
    let mut p = Planner::default();
    tick(&h, &mut p, 0.0);
    let uid = h.state().session.active().unwrap().uid;
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "42"})).unwrap();
    let mut puts = Vec::new();
    let mut now = 0.0;
    for _ in 0..40 {
        h.run_steps(1);
        now += 500.0;
        puts.extend(tick(&h, &mut p, now).0);
    }
    assert_eq!(puts, vec![uid], "frames without input leave the workbook as drafted");
}

#[test]
fn an_open_cell_editor_holds_the_draft_until_it_commits() {
    let (mut h, _) = web_harness();
    let mut p = Planner::default();
    tick(&h, &mut p, 0.0);
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    h.state_mut().begin_edit(Some("2".into()), false);
    h.run_steps(2);
    assert!(h.state().editing_in_place());
    assert!(tick(&h, &mut p, 10_000.0).0.is_empty());
    assert!(h.state_mut().commit_edit(1, 0, false, false));
    h.run_steps(2);
    tick(&h, &mut p, 10_001.0);
    assert_eq!(tick(&h, &mut p, 13_000.0).0.len(), 1);
}

#[test]
fn a_delivered_save_removes_the_draft_a_cancelled_one_keeps_it() {
    let (mut h, reports) = web_harness();
    let mut p = Planner::default();
    tick(&h, &mut p, 0.0);
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "42"})).unwrap();
    tick(&h, &mut p, 1.0);
    assert_eq!(tick(&h, &mut p, 3_000.0).0.len(), 1);

    h.state_mut().open_dialog("saveAs", json!({}));
    h.run_steps(2);
    reports.lock().unwrap().pop().unwrap().settle(Delivery::Cancelled);
    h.run_steps(2);
    assert_eq!(tick(&h, &mut p, 4_000.0), (vec![], vec![]), "still unsaved: the draft stays");

    h.state_mut().open_dialog("saveAs", json!({}));
    h.run_steps(2);
    assert!(h.state().draft_docs()[0].unsaved, "unsaved while the save picker is open");
    reports.lock().unwrap().pop().unwrap().settle(Delivery::Delivered);
    h.run_steps(2);
    assert_eq!(tick(&h, &mut p, 5_000.0).1.len(), 1, "saved: the draft is removed");
    assert_eq!(tick(&h, &mut p, 6_000.0), (vec![], vec![]));
}

#[test]
fn a_restored_draft_opens_unsaved_under_its_name_and_replaces_the_blank_book() {
    let (mut h, _) = web_harness();
    let mut p = Planner::default();
    let mut s = Session::new();
    s.new_workbook();
    s.run("cell.set", json!({"cell": "A1", "input": "42"})).unwrap();
    let bytes = gridcraft_ui_egui::drafts::draft_bytes(&s.active().unwrap().wb).unwrap();

    let uid = h.state_mut().open_draft("Budget.xlsx", &bytes).unwrap();
    h.run_steps(2);
    let app = h.state();
    assert_eq!(app.session.documents().len(), 1, "the untouched blank Book1 is replaced");
    let d = app.session.active().unwrap();
    assert_eq!((d.uid, d.display_title(), d.is_dirty()), (uid, "Budget.xlsx".to_string(), true));
    let wb = d.wb.clone();
    p.adopt(uid, "rec-1", &wb, 0.0);
    assert_eq!(tick(&h, &mut p, 60_000.0), (vec![], vec![]), "the restored content counts as drafted");
    h.state_mut().run("cell.set", json!({"cell": "A2", "input": "1"})).unwrap();
    tick(&h, &mut p, 61_000.0);
    assert_eq!(tick(&h, &mut p, 64_000.0).0, vec![uid]);
    assert_eq!(p.key(uid), "rec-1");
}

#[test]
fn a_workbook_opened_with_its_password_is_not_drafted() {
    let (mut h, _) = web_harness();
    let mut s = Session::new();
    s.new_workbook();
    s.run("cell.set", json!({"cell": "A1", "input": "42"})).unwrap();
    let xlsx = gridcraft_engine::io::save_bytes(&s.active().unwrap().wb, "book.xlsx").unwrap();
    let encrypted = gridcraft_xlsx::testing::encrypt_workbook(&xlsx, "synthetic-pw-1");
    let b64 = gridcraft_engine::io::base64_encode(&encrypted);
    h.state_mut().run("file.open", json!({"name": "Secret.xlsx", "base64": b64, "password": "synthetic-pw-1"})).unwrap();
    h.state_mut().run("cell.set", json!({"cell": "A2", "input": "1"})).unwrap();
    assert!(h.state().session.active().unwrap().is_dirty());
    assert!(h.state().draft_docs().is_empty());
}
