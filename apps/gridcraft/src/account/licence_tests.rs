//! The edit lock against pi-mock: what each account state lets the user do with an open workbook.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use erpcons_account::{Account, Clock, Config, LicenceState, MemorySecretStore, Status};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::account::AccountHook;
use gridcraft_ui_egui::edit_lock::EditLock;
use pi_mock::{Fault, OFFICE_PRODUCT, ORG_A, Scenario, Server, Tenant};
use serde_json::json;

use super::tests::{browser, mock_config};
use super::{DesktopAccount, ResyncTiming};

const T0: i64 = 1_792_828_800;
const DAY: i64 = 86_400;

struct TestClock {
    unix: AtomicI64,
    mono_ms: AtomicU64,
}

impl Clock for TestClock {
    fn unix_now(&self) -> i64 {
        self.unix.load(Ordering::SeqCst)
    }
    fn monotonic(&self) -> Duration {
        Duration::from_millis(self.mono_ms.load(Ordering::SeqCst))
    }
}

struct Env {
    server: Server,
    dir: tempfile::TempDir,
    clock: Arc<TestClock>,
}

impl Env {
    fn new(mut scenario: Scenario) -> Self {
        scenario.now.get_or_insert(T0);
        Self {
            server: Server::start(scenario),
            dir: tempfile::tempdir().unwrap(),
            clock: Arc::new(TestClock { unix: AtomicI64::new(T0), mono_ms: AtomicU64::new(1_000_000) }),
        }
    }

    fn config(&self) -> Config {
        let mut config = mock_config(&self.server, &self.dir);
        config.clock = self.clock.clone();
        config.timeout = Duration::from_secs(2);
        config
    }

    /// The hook as `main` installs it, over an in-memory credential store; refused edits may
    /// resync within milliseconds.
    fn hook(&self) -> DesktopAccount {
        let timing = ResyncTiming { min: Duration::from_millis(20), max: Duration::from_millis(50) };
        let hook = DesktopAccount::spawn_timed(self.config(), Some(Arc::new(MemorySecretStore::new())), Box::new(browser), timing);
        wait_for("the account to open", || hook.edit_lock() != Some(EditLock::Starting));
        hook
    }

    /// Moves client wall clock, client monotonic clock and platform time together.
    fn advance(&self, secs: i64) {
        self.clock.unix.fetch_add(secs, Ordering::SeqCst);
        self.clock.mono_ms.fetch_add(secs.unsigned_abs() * 1000, Ordering::SeqCst);
        let now = self.clock.unix_now();
        self.server.update_scenario(|s| s.now = Some(now));
    }
}

fn wait_for(what: &str, done: impl Fn() -> bool) {
    for _ in 0..1500 {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

fn account(hook: &DesktopAccount) -> Arc<Account> {
    hook.shared.account.get().cloned().unwrap()
}

fn licence(hook: &DesktopAccount) -> Option<LicenceState> {
    match account(hook).status() {
        Status::SignedIn(s) => Some(s.licence),
        _ => None,
    }
}

fn signed_in(hook: &DesktopAccount) {
    hook.sign_in();
    wait_for("sign-in", || matches!(account(hook).status(), Status::SignedIn(_)) && !hook.snapshot().busy);
}

/// A workbook with a value typed in before any lock, so saving it is not a no-op.
fn app() -> SheetApp {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("cell.set", json!({"cell": "A1", "input": "typed before the lock"})).unwrap();
    SheetApp::new(session, Default::default())
}

/// What the interface does each frame: take the hook's lock.
fn sync(app: &mut SheetApp, hook: &DesktopAccount) {
    app.set_edit_lock(hook.edit_lock());
}

fn edit(app: &mut SheetApp) -> Result<serde_json::Value, String> {
    app.session.run("cell.set", json!({"cell": "B2", "input": "42"}))
}

fn office_in_org_a() -> Scenario {
    Scenario { tenants: vec![Tenant::office_free("ten_office", ORG_A)], ..Default::default() }
}

#[test]
fn a_guest_opens_views_saves_exports_and_prints_but_cannot_edit() {
    let env = Env::new(Scenario::office_free());
    let hook = env.hook();
    let mut app = app();
    sync(&mut app, &hook);
    assert_eq!(hook.edit_lock(), Some(EditLock::SignIn));
    let refused = edit(&mut app).unwrap_err();
    assert!(refused.ends_with(EditLock::SignIn.reason()), "{refused}");
    assert!(app.read_only(), "the ribbon greys as for a read-only document");

    let path = |name: &str| env.dir.path().join(name).to_string_lossy().into_owned();
    app.session.run("file.save", json!({"path": path("book.xlsx")})).expect("Save keeps what was typed");
    assert!(!app.session.active().unwrap().is_dirty());
    app.session.run("file.saveAs", json!({"path": path("copy.xlsx")})).expect("Save As");
    app.session.run("file.exportCsv", json!({"path": path("book.csv")})).expect("Export");
    app.session.run("file.exportPdf", json!({"path": path("book.pdf")})).expect("Export PDF");
    app.session.run("file.print", json!({})).expect("Print");
    for id in ["file.save", "file.saveAs", "file.exportPdf", "file.print", "edit.copy"] {
        assert!(app.offers(id), "{id}");
    }
    assert!(!app.offers("home.bold") && !app.offers("edit.undo"));

    app.session.run("file.open", json!({"path": path("book.xlsx")})).expect("Open");
    assert_eq!(app.session.documents().len(), 2);
    assert!(edit(&mut app).is_err(), "a workbook opened later is locked too");
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    assert_eq!(sheet.value(CellRef::new(0, 0)), Value::from("typed before the lock"), "saved under the lock");
}

#[test]
fn signing_in_with_the_free_plan_unlocks_the_open_workbook() {
    let env = Env::new(Scenario::office_free());
    let hook = env.hook();
    let mut app = app();
    sync(&mut app, &hook);
    let uid = app.session.active().unwrap().uid;
    assert!(edit(&mut app).is_err());

    signed_in(&hook);
    wait_for("the lease", || hook.edit_lock().is_none());
    assert!(matches!(licence(&hook), Some(LicenceState::Active { ref features, .. }) if features.iter().any(|f| f == erpcons_account::OFFICE_USE)));
    sync(&mut app, &hook);
    assert_eq!(app.session.active().unwrap().uid, uid, "the same workbook, never reopened");
    edit(&mut app).expect("editable");
    assert!(env.server.requests().iter().any(|r| r.path == "/api/v1/id/devices/activate" && r.body.contains(OFFICE_PRODUCT)));
}

#[test]
fn a_revoked_device_locks_editing_after_a_refresh() {
    let env = Env::new(Scenario::office_free());
    let hook = env.hook();
    signed_in(&hook);
    wait_for("the lease", || hook.edit_lock().is_none());
    let account = account(&hook);
    let device = account.diagnostics().device_id.unwrap();
    assert!(env.server.revoke_device(&device));
    env.advance(13 * 3600);
    let _ = account.refresh();
    assert_eq!(licence(&hook), Some(LicenceState::DeviceRevoked));
    let mut app = app();
    sync(&mut app, &hook);
    assert!(edit(&mut app).unwrap_err().ends_with(EditLock::Revoked.reason()));
    assert!(!EditLock::Revoked.retry_offered(), "only signing in again helps");
}

#[test]
fn an_expired_lease_locks_offline_and_a_renewal_unlocks() {
    let env = Env::new(Scenario::office_free());
    let hook = env.hook();
    signed_in(&hook);
    wait_for("the lease", || hook.edit_lock().is_none());
    let account = account(&hook);
    let mut app = app();

    // Past the 7-day lease while still online: renewal has not run yet, not "no plan".
    env.advance(8 * DAY);
    assert_eq!(hook.edit_lock(), Some(EditLock::RenewFailed));
    sync(&mut app, &hook);
    assert!(edit(&mut app).is_err());
    account.refresh().unwrap();
    assert_eq!(hook.edit_lock(), None, "the renewed lease unlocks");
    sync(&mut app, &hook);
    edit(&mut app).expect("editable again, same workbook");

    // Offline past the next expiry (after `advance`, which resets the mock's faults).
    env.advance(8 * DAY);
    for (method, path) in [
        ("GET", "/.well-known/openid-configuration"),
        ("POST", "/id/token"),
        ("GET", "/api/v1/id/me"),
        ("GET", "/api/v1/id/me/entitlements"),
        ("GET", "/api/v1/id/devices/challenge"),
        ("POST", "/api/v1/id/devices/activate"),
        ("POST", "/api/v1/id/licenses/refresh"),
    ] {
        env.server.add_fault(Fault::delay(method, path, Duration::from_secs(5)));
    }
    assert!(account.refresh().is_err());
    assert_eq!(hook.edit_lock(), Some(EditLock::RenewOffline));
    sync(&mut app, &hook);
    assert!(edit(&mut app).unwrap_err().ends_with(EditLock::RenewOffline.reason()));
    app.session.run("file.save", json!({"path": env.dir.path().join("offline.xlsx").to_string_lossy()})).expect("Save while offline");
}

#[test]
fn the_personal_context_asks_for_an_organization() {
    let env = Env::new(office_in_org_a());
    let hook = env.hook();
    signed_in(&hook);
    assert_eq!(hook.edit_lock(), Some(EditLock::ChooseOrg));
    hook.select_org(Some(ORG_A.into()));
    wait_for("the lease of organization A", || hook.edit_lock().is_none());
}

#[test]
fn retry_activates_once_the_organization_has_a_plan() {
    let env = Env::new(Scenario::default());
    let hook = env.hook();
    signed_in(&hook);
    hook.select_org(Some(ORG_A.into()));
    wait_for("the refused activation", || hook.edit_lock() == Some(EditLock::NoPlan));
    assert_eq!(licence(&hook), Some(LicenceState::NotEntitled));
    assert!(EditLock::NoPlan.retry_offered());
    // The plan arrives; a plain sync keeps the block (ADR 0013), Retry lifts it.
    env.server.update_scenario(|s| s.tenants = vec![Tenant::office_free("ten_office", ORG_A)]);
    let _ = account(&hook).refresh();
    assert_eq!(hook.edit_lock(), Some(EditLock::NoPlan));
    hook.retry_activation();
    wait_for("the retried activation", || hook.edit_lock().is_none() && !hook.snapshot().busy);
    assert_eq!(hook.snapshot().error, None);
}

#[test]
fn a_refused_edit_syncs_again_without_waiting_for_the_hourly_refresh() {
    let env = Env::new(Scenario { faults: vec![Fault::problem("POST", "/api/v1/id/devices/activate", 422, "")], ..Scenario::office_free() });
    let hook = env.hook();
    signed_in(&hook);
    wait_for("the failed activation", || hook.edit_lock() == Some(EditLock::NotActivated));
    // PI recovered; nothing asks for a sync except the refused edit itself.
    env.server.clear_faults();
    let mut app = app();
    app.account_hook = Some(Arc::new(hook));
    let hook = app.account_hook.clone().unwrap();
    app.set_edit_lock(hook.edit_lock());
    app.begin_edit(Some("7".into()), false);
    assert!(app.editor.is_none(), "typing is refused");
    assert_eq!(app.toast.as_ref().map(|(notice, _)| notice.as_str()), Some(EditLock::NotActivated.reason()));
    wait_for("the resync to activate this device", || hook.edit_lock().is_none());
}

#[test]
fn an_account_that_cannot_open_keeps_editing_locked() {
    let env = Env::new(Scenario::office_free());
    let mut config = env.config();
    config.client_id = "another-client".into();
    let hook = DesktopAccount::spawn(config, Some(Arc::new(MemorySecretStore::new())), Box::new(browser));
    wait_for("the account to fail", || hook.edit_lock() == Some(EditLock::Unavailable));
}

#[test]
fn reasons_have_vietnamese_translations() {
    use gridcraft_ui_egui::i18n::Language;
    for (lock, vi) in [
        (EditLock::Starting, "Đang kiểm tra giấy phép…"),
        (EditLock::SignIn, "Đăng nhập PI ID để sửa"),
        (EditLock::ChooseOrg, "Chọn tổ chức để sửa"),
        (EditLock::Revoked, "Máy này đã bị gỡ khỏi tài khoản PI ID. Đăng nhập lại để sửa"),
        (EditLock::DeviceCap, "Gói đã đủ số máy. Gỡ bớt một máy trong PI ID rồi bấm Thử lại ở Tệp › Tài khoản"),
        (EditLock::NotActivated, "Máy này chưa được kích hoạt để sửa"),
        (EditLock::SessionOnly, "Máy này không có kho lưu thông tin đăng nhập để giữ giấy phép nên chưa sửa được"),
        (EditLock::RenewOffline, "Kết nối mạng để gia hạn giấy phép và sửa tiếp"),
        (EditLock::RenewFailed, "Chưa gia hạn được giấy phép; ứng dụng đang tự thử lại"),
        (EditLock::NoPlan, "Tổ chức này chưa có gói ERPCons Office"),
        (EditLock::Unavailable, "Không dùng được giấy phép PI ID trên máy này nên chưa sửa được"),
    ] {
        assert_eq!(Language::Vi.tr(lock.reason()), vi, "{lock:?}");
    }
    assert_eq!(Language::Vi.tr("Editing"), "Chỉnh sửa");
}
