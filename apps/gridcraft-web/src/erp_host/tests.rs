//! `erpHost` rules 1–11 against the real app with a fake shell: what is saved when, and what
//! counts as saved.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gridcraft_engine::Session;
use gridcraft_engine::model::Workbook;
use gridcraft_ui_egui::save_status::SaveState;
use gridcraft_ui_egui::{Delivery, DeliveryReport, Services, SheetApp};
use serde_json::json;

use super::autosave::{CHECKPOINT_MS, IDLE_MS};
use super::docs::{HostDocs, Reply, Stop};
use super::protocol::{API_VERSION, OpenRequest, Reason, SaveResult, Shell};

struct Save {
    doc_id: String,
    revision: u64,
    reason: Reason,
    bytes: Vec<u8>,
    reply: Option<Reply>,
}

#[derive(Debug, PartialEq)]
enum Call {
    Dirty(String, bool),
    Opened(String, bool, Option<String>),
    Closed(String),
}

#[derive(Default)]
struct FakeShell {
    saves: RefCell<Vec<Save>>,
    calls: RefCell<Vec<Call>>,
}

impl Shell for FakeShell {
    fn save(&self, doc_id: &str, bytes: Vec<u8>, revision: u64, reason: Reason, reply: Reply) {
        self.saves.borrow_mut().push(Save { doc_id: doc_id.into(), revision, reason, bytes, reply: Some(reply) });
    }
    fn dirty(&self, doc_id: &str, dirty: bool) {
        self.calls.borrow_mut().push(Call::Dirty(doc_id.into(), dirty));
    }
    fn opened(&self, doc_id: &str, ok: bool, reason: Option<&str>) {
        self.calls.borrow_mut().push(Call::Opened(doc_id.into(), ok, reason.map(str::to_string)));
    }
    fn closed(&self, doc_id: &str) {
        self.calls.borrow_mut().push(Call::Closed(doc_id.into()));
    }
}

struct T {
    app: SheetApp,
    docs: Rc<RefCell<HostDocs>>,
    shell: Rc<FakeShell>,
    downloads: Rc<RefCell<Vec<DeliveryReport>>>,
}

fn setup() -> T {
    let downloads: Rc<RefCell<Vec<DeliveryReport>>> = Rc::default();
    let started = downloads.clone();
    let services = Services { download: Some(Box::new(move |_, _, report| started.borrow_mut().push(report))), ..Default::default() };
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, services);
    let shell = Rc::new(FakeShell::default());
    let docs = HostDocs::new(shell.clone());
    HostDocs::connect(&docs, &mut app);
    T { app, docs, shell, downloads }
}

fn xlsx(a1: &str) -> Vec<u8> {
    let mut s = Session::new();
    s.new_workbook();
    s.run("cell.set", json!({"cell": "A1", "input": a1})).unwrap();
    gridcraft_engine::io::save_bytes(&s.active().unwrap().wb, "x.xlsx").unwrap()
}

impl T {
    fn open_as(&mut self, doc_id: &str, format: &str, bytes: Option<Vec<u8>>) {
        let request = OpenRequest { doc_id: doc_id.into(), version: Some(1), title: "Báo cáo".into(), format: format.into(), bytes };
        self.docs.borrow_mut().open(&mut self.app, request);
    }

    /// Opens `doc_id` and returns its uid.
    fn open(&mut self, doc_id: &str) -> u64 {
        self.open_as(doc_id, "xlsx", Some(xlsx("1")));
        self.app.session.active().unwrap().uid
    }

    /// One frame: the app applies settled saves, then the host documents run.
    fn frame(&mut self, now: f64) {
        self.app.settle_saves();
        self.docs.borrow_mut().after_frame(&mut self.app, now);
    }

    fn activate(&mut self, uid: u64) {
        let i = self.app.document_index(Some(uid)).unwrap();
        self.app.session.set_active(i);
    }

    fn edit(&mut self, cell: &str, input: &str) {
        self.app.run("cell.set", json!({"cell": cell, "input": input})).unwrap();
    }

    fn ctrl_s(&mut self) {
        self.app.run("file.save", json!({})).unwrap();
    }

    fn resolve(&self, i: usize, result: SaveResult) {
        let reply = self.shell.saves.borrow_mut()[i].reply.take().unwrap();
        reply.resolve(result);
    }

    fn saves(&self) -> Vec<(String, u64, Reason)> {
        self.shell.saves.borrow().iter().map(|s| (s.doc_id.clone(), s.revision, s.reason)).collect()
    }

    fn doc(&self, uid: u64) -> &gridcraft_engine::DocState {
        self.app.session.documents().iter().find(|d| d.uid == uid).unwrap()
    }

    fn calls(&self) -> std::cell::Ref<'_, Vec<Call>> {
        self.shell.calls.borrow()
    }
}

#[test]
fn open_reports_once_and_the_document_is_hosted_not_drafted() {
    let mut t = setup();
    let uid = t.open("d1");
    assert_eq!(*t.calls(), [Call::Opened("d1".into(), true, None)]);
    assert!(t.app.is_hosted(uid));
    assert_eq!(t.doc(uid).display_title(), "Báo cáo.xlsx");
    assert_eq!(t.app.session.documents().len(), 1, "the blank Book1 was replaced");
    t.edit("A1", "2");
    assert!(t.app.draft_docs().is_empty(), "rule 8: no erpDrafts for host documents");
    t.frame(0.0);
    assert_eq!(t.calls().iter().filter(|c| matches!(c, Call::Opened(..))).count(), 1);
}

#[test]
fn refused_opens_report_the_shells_intake_key() {
    let mut t = setup();
    t.open_as("d1", "docx", Some(xlsx("1")));
    t.open_as("d2", "xlsx", Some(b"not a workbook".to_vec()));
    let encrypted = gridcraft_xlsx::testing::encrypt_workbook(&xlsx("1"), "synthetic-pw-1");
    t.open_as("d3", "xlsx", Some(encrypted));
    t.open_as("d4", "xlsx", None);
    let key = |s: &str| Some(s.to_string());
    assert_eq!(
        *t.calls(),
        [
            Call::Opened("d1".into(), false, key("unsupported")),
            Call::Opened("d2".into(), false, key("unsupported")),
            Call::Opened("d3".into(), false, key("unsupported")),
            Call::Opened("d4".into(), false, key("parserFailed")),
        ]
    );
    assert!(t.app.message.is_some(), "the refusal is shown like a refused dropped file");
    assert!(t.app.session.documents().iter().all(|d| !t.app.is_hosted(d.uid)));
    assert!(t.app.dialog.is_none(), "no password prompt online");
}

#[test]
fn the_open_limit_is_reported() {
    let mut t = setup();
    for i in 0..gridcraft_engine::MAX_DOCUMENTS {
        t.open(&format!("d{i}"));
    }
    t.open_as("one-more", "xlsx", Some(xlsx("1")));
    assert_eq!(t.calls().last(), Some(&Call::Opened("one-more".into(), false, Some("parserFailed".into()))));
}

#[test]
fn ctrl_s_saves_to_the_shell_and_cleans_only_as_of_the_revision_sent() {
    let mut t = setup();
    let uid = t.open("d1");
    t.edit("A1", "42");
    t.frame(0.0);
    assert_eq!(t.calls().last(), Some(&Call::Dirty("d1".into(), true)));
    let sent = t.doc(uid).revision;
    t.ctrl_s();
    assert_eq!(t.saves(), [("d1".to_string(), sent, Reason::Manual)], "rule 9");
    assert!(t.downloads.borrow().is_empty(), "never the picker or a download");
    let (wb, _) = gridcraft_engine::io::open_bytes("x.xlsx", &t.shell.saves.borrow()[0].bytes).unwrap();
    assert_eq!(wb.sheets[0].value(gridcraft_engine::core::CellRef::new(0, 0)).as_f64(), Some(42.0));
    let written: Arc<Workbook> = t.doc(uid).wb.clone();
    assert_eq!(t.app.save_state(uid), SaveState::Saving);

    t.edit("A2", "typed while saving");
    t.frame(100.0);
    t.resolve(0, SaveResult::Saved { version: Some(2) });
    t.frame(200.0);
    let d = t.doc(uid);
    assert!(Arc::ptr_eq(&d.saved, &written), "rule 1: saved as of the revision sent");
    assert!(d.is_dirty(), "the edit made in flight stays unsaved");
    assert_eq!(t.docs.borrow().version(uid), Some(2));
    assert!(!t.calls().contains(&Call::Dirty("d1".into(), false)));

    t.frame(100.0 + IDLE_MS);
    assert_eq!(t.saves().len(), 2, "the in-flight edit triggers the next AutoSave");
    assert_eq!(t.saves()[1].2, Reason::Auto);
    t.resolve(1, SaveResult::Saved { version: Some(3) });
    t.frame(100.0 + IDLE_MS + 10.0);
    assert!(!t.doc(uid).is_dirty());
    assert_eq!(t.calls().last(), Some(&Call::Dirty("d1".into(), false)));
    assert_eq!(t.app.save_state(uid), SaveState::Idle);
}

#[test]
fn autosave_waits_three_seconds_after_the_last_edit_and_checkpoints_every_thirty() {
    let mut t = setup();
    let uid = t.open("d1");
    t.edit("A1", "1");
    t.frame(0.0);
    t.frame(IDLE_MS - 1.0);
    assert!(t.saves().is_empty());
    t.frame(IDLE_MS);
    assert_eq!(t.saves(), [("d1".to_string(), t.doc(uid).revision, Reason::Auto)]);
    t.resolve(0, SaveResult::Saved { version: Some(2) });

    let start = 10_000.0;
    let mut now = start;
    let mut i = 0;
    while now < start + CHECKPOINT_MS {
        t.edit("B1", &i.to_string());
        t.frame(now);
        i += 1;
        now += 1_000.0;
    }
    assert_eq!(t.saves().len(), 1, "no pause yet");
    t.edit("B1", "last");
    t.frame(now);
    assert_eq!(t.saves().len(), 2, "a checkpoint while edits keep coming");
}

#[test]
fn one_save_in_flight_and_later_triggers_coalesce_into_one_follow_up() {
    let mut t = setup();
    let uid = t.open("d1");
    t.edit("A1", "1");
    t.frame(0.0);
    t.ctrl_s();
    t.edit("A1", "2");
    t.ctrl_s();
    t.edit("A1", "3");
    t.frame(1.0);
    t.frame(1.0 + IDLE_MS);
    t.ctrl_s();
    assert_eq!(t.saves().len(), 1, "rule 2: one in flight");
    t.resolve(0, SaveResult::Saved { version: Some(2) });
    let newest = t.doc(uid).revision;
    assert_eq!(t.saves().len(), 2);
    assert_eq!(t.saves()[1], ("d1".to_string(), newest, Reason::Manual), "one follow-up of the newest revision");
    t.frame(2.0 + IDLE_MS);
    t.frame(3.0 + 2.0 * IDLE_MS);
    assert_eq!(t.saves().len(), 2);
    t.resolve(1, SaveResult::Saved { version: Some(3) });
    t.frame(4.0 + 2.0 * IDLE_MS);
    assert!(!t.doc(uid).is_dirty());
    assert_eq!(t.saves().len(), 2, "nothing left to save");
}

#[test]
fn a_conflict_stops_autosave_keeps_the_edits_and_is_never_retried() {
    let mut t = setup();
    let uid = t.open("d1");
    t.edit("A1", "1");
    t.frame(0.0);
    t.ctrl_s();
    t.edit("A1", "2");
    t.ctrl_s();
    t.resolve(0, SaveResult::Conflict);
    t.frame(10.0);
    assert_eq!(t.saves().len(), 1, "the queued save is dropped too");
    assert!(t.doc(uid).is_dirty());
    assert_eq!(t.docs.borrow().stopped(uid), Some(Stop::Conflict));
    assert!(t.app.message.is_none(), "the shell shows the conflict");
    t.edit("A1", "3");
    t.frame(20.0);
    t.frame(20.0 + IDLE_MS);
    t.docs.borrow_mut().flush(Box::new(|| {}));
    assert_eq!(t.saves().len(), 1, "no AutoSave and no flush");
    t.ctrl_s();
    assert_eq!(t.saves().len(), 2, "the user can still save");
    t.resolve(1, SaveResult::Saved { version: Some(5) });
    assert_eq!(t.docs.borrow().stopped(uid), None);
}

#[test]
fn offline_and_retryable_failures_stay_dirty_quietly_and_keep_autosave() {
    for result in [SaveResult::Offline, SaveResult::Failed { retryable: true, message: "Máy chủ đang bận".into() }] {
        let mut t = setup();
        let uid = t.open("d1");
        t.edit("A1", "1");
        t.frame(0.0);
        t.frame(IDLE_MS);
        t.resolve(0, result);
        t.frame(IDLE_MS + 1.0);
        assert!(t.doc(uid).is_dirty());
        assert!(t.app.message.is_none());
        assert_eq!(t.app.save_state(uid), SaveState::Idle, "rule 5: the shell shows the state");
        assert_eq!(t.docs.borrow().stopped(uid), None);
        t.edit("A1", "2");
        t.frame(10_000.0);
        t.frame(10_000.0 + IDLE_MS);
        assert_eq!(t.saves().len(), 2);
    }
}

#[test]
fn acknowledged_after_offline_cleans_as_of_that_revision() {
    let mut t = setup();
    let uid = t.open("d1");
    t.edit("A1", "1");
    t.frame(0.0);
    t.frame(IDLE_MS);
    let sent = t.doc(uid).revision;
    let written = t.doc(uid).wb.clone();
    t.resolve(0, SaveResult::Offline);
    t.frame(IDLE_MS + 1.0);
    assert!(t.doc(uid).is_dirty());
    t.docs.borrow_mut().acknowledged("d1", sent, Some(4));
    t.frame(IDLE_MS + 2.0);
    assert!(!t.doc(uid).is_dirty(), "treated like `saved`");
    assert!(Arc::ptr_eq(&t.doc(uid).saved, &written));
    assert_eq!(t.docs.borrow().version(uid), Some(4));
    assert_eq!(t.calls().last(), Some(&Call::Dirty("d1".into(), false)));
    t.frame(IDLE_MS * 3.0);
    assert_eq!(t.saves().len(), 1, "nothing left to save");
}

#[test]
fn an_edit_made_after_the_save_was_sent_stays_dirty_after_acknowledged() {
    let mut t = setup();
    let uid = t.open("d1");
    t.edit("A1", "1");
    t.frame(0.0);
    t.frame(IDLE_MS);
    let sent = t.doc(uid).revision;
    let written = t.doc(uid).wb.clone();
    t.edit("A2", "typed while saving");
    t.frame(IDLE_MS + 1.0);
    t.resolve(0, SaveResult::Offline);
    t.frame(IDLE_MS + 2.0);
    t.docs.borrow_mut().acknowledged("d1", sent, Some(4));
    t.frame(IDLE_MS + 3.0);
    assert!(Arc::ptr_eq(&t.doc(uid).saved, &written), "rule 1: saved as of the revision sent");
    assert!(t.doc(uid).is_dirty());
    assert!(!t.calls().contains(&Call::Dirty("d1".into(), false)));
    t.frame(2.0 * IDLE_MS + 1.0);
    assert_eq!(t.saves().len(), 2, "the later edit is still saved");
    assert_eq!(t.saves()[1].1, t.doc(uid).revision);
}

#[test]
fn an_older_or_unknown_acknowledged_is_ignored() {
    let mut t = setup();
    let uid = t.open("d1");
    t.edit("A1", "1");
    t.frame(0.0);
    t.frame(IDLE_MS);
    let older = t.doc(uid).revision;
    t.resolve(0, SaveResult::Offline);
    t.edit("A1", "2");
    t.frame(10_000.0);
    t.frame(10_000.0 + IDLE_MS);
    assert_eq!(t.saves().len(), 2);
    let newer = t.doc(uid).revision;
    let written = t.doc(uid).wb.clone();
    t.resolve(1, SaveResult::Offline);
    t.docs.borrow_mut().acknowledged("d1", newer, Some(5));
    t.frame(10_000.0 + IDLE_MS + 1.0);
    assert!(!t.doc(uid).is_dirty());

    t.docs.borrow_mut().acknowledged("d1", older, Some(4));
    t.docs.borrow_mut().acknowledged("other", newer, Some(9));
    t.docs.borrow_mut().acknowledged("d1", newer + 100, Some(9));
    t.frame(10_000.0 + IDLE_MS + 2.0);
    assert!(Arc::ptr_eq(&t.doc(uid).saved, &written), "never back to an older revision");
    assert!(!t.doc(uid).is_dirty());
    assert_eq!(t.docs.borrow().version(uid), Some(5));
}

#[test]
fn a_failure_that_will_not_pass_shows_its_message_and_stops_autosave() {
    let mut t = setup();
    let uid = t.open("d1");
    t.edit("A1", "1");
    t.frame(0.0);
    t.frame(IDLE_MS);
    t.resolve(0, SaveResult::Failed { retryable: false, message: "Tệp quá lớn".into() });
    t.frame(IDLE_MS + 1.0);
    assert_eq!(t.app.save_state(uid), SaveState::Failed("Tệp quá lớn".into()));
    assert!(t.app.message.as_ref().is_some_and(|(_, m)| m.contains("Tệp quá lớn")));
    assert_eq!(t.docs.borrow().stopped(uid), Some(Stop::Failed));
    t.edit("A1", "2");
    t.frame(10_000.0);
    t.frame(10_000.0 + IDLE_MS);
    assert_eq!(t.saves().len(), 1);
}

#[test]
fn flush_saves_every_unacknowledged_document_and_settles_once_all_were_handed_over() {
    let mut t = setup();
    let first = t.open("d1");
    let second = t.open("d2");
    let clean = t.open("d3");
    t.activate(first);
    t.edit("A1", "1");
    t.frame(0.0);
    t.ctrl_s();
    t.edit("A1", "2");
    t.activate(second);
    t.edit("A1", "x");
    t.frame(10.0);
    let done = Rc::new(RefCell::new(false));
    let flag = done.clone();
    t.docs.borrow_mut().flush(Box::new(move || *flag.borrow_mut() = true));
    assert_eq!(t.saves().len(), 2);
    assert_eq!((t.saves()[1].0.as_str(), t.saves()[1].2), ("d2", Reason::Flush));
    assert!(!*done.borrow(), "d1's newest revision waits behind its save in flight");
    t.resolve(0, SaveResult::Saved { version: Some(2) });
    assert_eq!(t.saves().len(), 3);
    assert_eq!(t.saves()[2], ("d1".to_string(), t.doc(first).revision, Reason::Flush));
    assert!(*done.borrow());
    assert!(t.saves().iter().all(|s| s.0 != "d3"), "{clean} had nothing to save");

    let again = Rc::new(RefCell::new(false));
    let flag = again.clone();
    t.docs.borrow_mut().flush(Box::new(move || *flag.borrow_mut() = true));
    assert!(*again.borrow(), "nothing new: settles at once");
    // Saves started without the app are followed by it from the next frame on.
    t.frame(20.0);
    assert_eq!(t.app.save_state(first), SaveState::Saving);
}

#[test]
fn local_documents_keep_downloads_and_drafts() {
    let mut t = setup();
    let hosted = t.open("d1");
    t.app.run("file.new", json!({})).unwrap();
    let local = t.app.session.active().unwrap().uid;
    t.edit("A1", "local");
    t.frame(0.0);
    t.frame(IDLE_MS * 2.0);
    assert!(t.saves().is_empty(), "no AutoSave of a local document");
    assert_eq!(t.app.draft_docs().iter().map(|d| d.uid).collect::<Vec<_>>(), [local]);
    t.ctrl_s();
    assert!(t.saves().is_empty());
    assert_eq!(t.downloads.borrow().len(), 1);
    t.downloads.borrow()[0].settle(Delivery::Delivered);
    t.frame(IDLE_MS * 3.0);
    assert!(!t.doc(local).is_dirty(), "a local download still counts as saved");
    assert!(t.calls().iter().all(|c| !matches!(c, Call::Dirty(..))), "{hosted} never changed");
}

#[test]
fn save_as_of_a_host_document_is_a_local_copy() {
    let mut t = setup();
    let uid = t.open("d1");
    t.edit("A1", "1");
    t.app.run("file.saveAs", json!({})).unwrap();
    assert!(t.saves().is_empty());
    assert_eq!(t.downloads.borrow().len(), 1);
    t.downloads.borrow()[0].settle(Delivery::Delivered);
    t.frame(0.0);
    assert!(t.doc(uid).is_dirty(), "only the shell's `saved` cleans it");
}

#[test]
fn closing_the_tab_reports_closed_and_a_reload_replaces_the_tab() {
    let mut t = setup();
    let uid = t.open("d1");
    t.edit("A1", "mine");
    t.frame(0.0);
    t.open_as("d1", "xlsx", Some(xlsx("theirs")));
    let reloaded = t.app.session.active().unwrap().uid;
    assert_ne!(reloaded, uid);
    assert!(t.app.document_index(Some(uid)).is_none(), "one tab per document");
    t.frame(10.0);
    assert!(!t.calls().contains(&Call::Closed("d1".into())));
    assert_eq!(t.calls().last(), Some(&Call::Dirty("d1".into(), false)), "the latest version is clean");
    t.app.run("file.close", json!({})).unwrap();
    t.frame(20.0);
    t.frame(30.0);
    assert_eq!(t.calls().iter().filter(|c| **c == Call::Closed("d1".into())).count(), 1);
    assert!(!t.app.is_hosted(reloaded));
}

#[test]
fn a_closed_host_document_is_never_offered_for_reopening() {
    let mut t = setup();
    t.open("d1");
    t.app.run("file.new", json!({})).unwrap();
    let local = t.app.session.active().unwrap().uid;
    t.edit("A1", "local");
    let hosted = t.open("d2");
    t.edit("A1", "unsaved");
    t.frame(0.0);
    let r = t.app.run("file.close", json!({"force": true})).unwrap();
    assert_eq!(r["closed"], true);
    assert!(t.app.document_index(Some(hosted)).is_none());
    assert!(!t.app.can_reopen_closed(), "rule 11: reopening would make a local copy of it");
    t.activate(local);
    t.app.run("file.close", json!({"force": true})).unwrap();
    assert!(t.app.can_reopen_closed(), "local documents still are");
}

#[test]
fn a_save_that_would_drop_macros_asks_first() {
    let mut t = setup();
    let uid = t.open("d1");
    let d = t.app.session.active_mut().unwrap();
    Arc::make_mut(&mut d.wb).vba_project = Some(b"VBA".to_vec());
    t.edit("A1", "2");
    t.frame(0.0);
    t.frame(IDLE_MS);
    t.docs.borrow_mut().flush(Box::new(|| {}));
    assert!(t.saves().is_empty(), "AutoSave and flush never drop the macros silently");
    t.ctrl_s();
    assert!(t.saves().is_empty());
    assert_eq!(t.app.dialog.as_ref().map(|d| d.name()), Some(gridcraft_ui_egui::save_as::DROP_MACROS));
    let host = t.app.document_host.clone().unwrap();
    host.save(&mut t.app, uid, true);
    assert_eq!(t.saves().len(), 1);
    t.frame(IDLE_MS + 1.0);
    assert!(t.app.notice.as_ref().is_some_and(|n| n.text.contains("macros")), "the loss warning is shown");
}

#[test]
fn the_shell_vocabulary_matches_the_contract() {
    assert_eq!(API_VERSION, 1.0);
    assert_eq!([Reason::Auto, Reason::Manual, Reason::Flush].map(Reason::as_str), ["auto", "manual", "flush"]);
    assert_eq!(SaveResult::from_json(&json!({"status": "saved", "version": 7})), SaveResult::Saved { version: Some(7) });
    assert_eq!(SaveResult::from_json(&json!({"status": "conflict", "latestVersion": 8})), SaveResult::Conflict);
    assert_eq!(SaveResult::from_json(&json!({"status": "offline"})), SaveResult::Offline);
    assert_eq!(
        SaveResult::from_json(&json!({"status": "failed", "retryable": false, "message": "Lỗi"})),
        SaveResult::Failed { retryable: false, message: "Lỗi".into() }
    );
    assert_eq!(SaveResult::from_json(&json!(null)), SaveResult::Offline, "an odd answer loses nothing");
}
