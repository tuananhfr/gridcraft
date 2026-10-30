//! The account hook's edit lock through real egui input: the read-only state on every document
//! with the lock's reason, Save, Export and Print kept, the reason and Retry in File › Account, and
//! no lock from a hook that never asks for one (the web).

use std::sync::{Arc, Mutex};

use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::account::{AccountHook, AccountSnapshot, AccountView, LicenceView, SignedInView};
use gridcraft_ui_egui::backstage::Page;
use gridcraft_ui_egui::edit_lock::EditLock;
use gridcraft_ui_egui::i18n::Language;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

#[derive(Default)]
struct Fake {
    view: Mutex<AccountView>,
    lock: Mutex<Option<EditLock>>,
    calls: Mutex<Vec<String>>,
}

impl Fake {
    fn with(view: AccountView, lock: Option<EditLock>) -> Arc<Fake> {
        let fake = Arc::new(Fake::default());
        *fake.view.lock().unwrap() = view;
        fake.set_lock(lock);
        fake
    }
    fn set_lock(&self, lock: Option<EditLock>) {
        *self.lock.lock().unwrap() = lock;
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn record(&self, call: &str) {
        self.calls.lock().unwrap().push(call.into());
    }
}

impl AccountHook for Fake {
    fn snapshot(&self) -> AccountSnapshot {
        AccountSnapshot { view: self.view.lock().unwrap().clone(), busy: false, error: None }
    }
    fn sign_in(&self) {
        self.record("sign_in");
    }
    fn cancel_sign_in(&self) {}
    fn sign_out(&self) {}
    fn select_org(&self, _org_id: Option<String>) {}
    fn retry_activation(&self) {
        self.record("retry_activation");
    }
    fn allowed(&self, _feature: &str) -> bool {
        true
    }
    fn warning_shown(&self, _days: u32) {}
    fn attach(&self, _ctx: &egui::Context) {}
    fn edit_lock(&self) -> Option<EditLock> {
        *self.lock.lock().unwrap()
    }
    fn edit_refused(&self) {
        self.record("edit_refused");
    }
}

/// A hook that leaves `edit_lock` at its default, as the web adapter does.
struct WebLike;

impl AccountHook for WebLike {
    fn snapshot(&self) -> AccountSnapshot {
        AccountSnapshot::default()
    }
    fn sign_in(&self) {}
    fn cancel_sign_in(&self) {}
    fn sign_out(&self) {}
    fn select_org(&self, _org_id: Option<String>) {}
    fn retry_activation(&self) {}
    fn allowed(&self, _feature: &str) -> bool {
        false
    }
    fn warning_shown(&self, _days: u32) {}
    fn attach(&self, _ctx: &egui::Context) {}
}

fn harness(hook: Arc<dyn AccountHook>, language: Language) -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell": "A1", "input": "10"})).unwrap();
    let mut app = SheetApp::new(s, Default::default());
    app.ui.language = language;
    app.account_hook = Some(hook);
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(2000.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn signed_in() -> AccountView {
    AccountView::SignedIn(SignedInView {
        name: "Nguyễn Văn An".into(),
        email: None,
        orgs: Vec::new(),
        active_org: Some("org".into()),
        licence: LicenceView::Free,
        warning: None,
        online: true,
        session_only: false,
        reauth_required: false,
    })
}

fn a1(h: &Harness) -> Value {
    h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::new(0, 0))
}

fn type_digit(h: &mut Harness, text: &str) {
    h.input_mut().events.push(Event::Text(text.into()));
    h.run_steps(2);
    h.input_mut().events.push(Event::Key {
        key: Key::Enter,
        physical_key: Some(Key::Enter),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.input_mut().events.push(Event::Key {
        key: Key::Enter,
        physical_key: Some(Key::Enter),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
    let _ = h.state_mut().session.execute("selection.set", json!({"range": "A1"}));
    h.run_steps(1);
}

/// Saves a render when `GRIDCRAFT_ACCOUNT_SCREENSHOTS` names a directory (for reviews; never asserted).
fn screenshot(h: &mut Harness, name: &str) {
    if let Some(dir) = std::env::var_os("GRIDCRAFT_ACCOUNT_SCREENSHOTS") {
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(std::path::Path::new(&dir).join(name)).unwrap();
    }
}

fn enabled(h: &Harness, label: &str) -> bool {
    !h.get_by_label(label).accesskit_node().is_disabled()
}

#[test]
fn a_locked_app_refuses_typing_with_the_reason_and_keeps_save_export_and_print() {
    let fake = Fake::with(AccountView::Guest, Some(EditLock::SignIn));
    let mut h = harness(fake.clone(), Language::En);
    assert!(h.state().read_only());
    type_digit(&mut h, "7");
    assert_eq!(a1(&h), Value::Number(10.0));
    assert_eq!(h.state().toast.as_ref().map(|(m, _)| m.as_str()), Some(EditLock::SignIn.reason()));
    assert!(fake.calls().contains(&"edit_refused".to_string()), "the hook hears of the refusal");
    for id in ["file.save", "file.saveAs", "file.exportPdf", "file.print", "edit.copy"] {
        assert!(h.state().offers(id), "{id}");
    }
    assert!(enabled(&h, "Copy (⌘C)") && !enabled(&h, "Bold (⌘B)"), "the ribbon greys as for a read-only document");
    screenshot(&mut h, "edit-lock-ribbon-en.png");
    // A workbook opened while locked is locked too.
    h.state_mut().session.new_workbook();
    h.run_steps(2);
    assert!(h.state().read_only());
}

#[test]
fn a_new_lease_unlocks_the_open_workbook_and_a_lock_commits_what_is_typed() {
    let fake = Fake::with(signed_in(), Some(EditLock::RenewOffline));
    let mut h = harness(fake.clone(), Language::Vi);
    let uid = h.state().session.active().unwrap().uid;
    fake.set_lock(None);
    h.run_steps(2);
    assert!(!h.state().read_only());
    type_digit(&mut h, "7");
    assert_eq!(a1(&h), Value::Number(7.0), "editable again without reopening");
    assert_eq!(h.state().session.active().unwrap().uid, uid);

    // The lease runs out while a cell is being typed: the text is entered, not lost.
    h.input_mut().events.push(Event::Text("8".into()));
    h.run_steps(2);
    assert!(h.state().editor.is_some());
    fake.set_lock(Some(EditLock::RenewOffline));
    h.run_steps(2);
    assert!(h.state().editor.is_none() && h.state().read_only());
    assert_eq!(a1(&h), Value::Number(8.0));
    h.state_mut().run("file.save", json!({"path": std::env::temp_dir().join("gridcraft-edit-lock.xlsx").to_string_lossy()})).expect("Save");
    let _ = std::fs::remove_file(std::env::temp_dir().join("gridcraft-edit-lock.xlsx"));
}

#[test]
fn the_account_page_gives_the_reason_with_sign_in_or_retry() {
    let fake = Fake::with(AccountView::Guest, Some(EditLock::SignIn));
    let mut h = harness(fake.clone(), Language::Vi);
    h.state_mut().open_backstage(Page::Account);
    h.run_steps(3);
    h.get_by_label("Đăng nhập PI ID để sửa");
    screenshot(&mut h, "edit-lock-account-guest-vi.png");
    assert!(h.query_by_label_contains("không bắt buộc").is_none(), "signing in is no longer optional to edit");
    h.get_by_label("Đăng nhập bằng PI ID").click();
    h.run_steps(2);
    assert_eq!(fake.calls(), ["sign_in"]);

    *fake.view.lock().unwrap() = signed_in();
    fake.set_lock(Some(EditLock::NoPlan));
    h.run_steps(3);
    h.get_by_label("Chỉnh sửa");
    h.get_by_label("Tổ chức này chưa có gói ERPCons Office");
    screenshot(&mut h, "edit-lock-account-no-plan-vi.png");
    h.get_by_label("Thử lại").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some("retry_activation"));

    fake.set_lock(Some(EditLock::Revoked));
    h.run_steps(3);
    h.get_by_label("Máy này đã bị gỡ khỏi tài khoản PI ID. Đăng nhập lại để sửa");
    assert!(h.query_by_label("Thử lại").is_none(), "Retry cannot lift a revoked device");

    fake.set_lock(None);
    h.run_steps(3);
    assert!(h.query_by_label("Chỉnh sửa").is_none(), "no editing row once editing is allowed");
}

#[test]
fn a_hook_without_an_edit_lock_never_locks() {
    let mut h = harness(Arc::new(WebLike), Language::En);
    assert!(!h.state().read_only());
    type_digit(&mut h, "7");
    assert_eq!(a1(&h), Value::Number(7.0));
    h.state_mut().open_backstage(Page::Account);
    h.run_steps(3);
    h.get_by_label("Signing in is optional. Every feature works without an account.");
}
