//! The optional PI ID account UI, driven by a fake hook: nothing without a hook, File › Account in
//! each state, the title-bar button (also in narrow windows) and the lease warning.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use egui::vec2;
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::account::{
    AccountError, AccountHook, AccountSnapshot, AccountView, LicenceView, OrgView, SignedInView, WarningView, format_date,
};
use gridcraft_ui_egui::backstage::Page;
use gridcraft_ui_egui::i18n::Language;

type Harness = egui_kittest::Harness<'static, SheetApp>;

const DESKTOP: egui::Vec2 = egui::Vec2::new(1366.0, 768.0);
/// 15 March 2027, noon UTC.
const EXPIRES: i64 = 1_805_112_000;

#[derive(Default)]
struct Fake {
    snapshot: Mutex<AccountSnapshot>,
    calls: Mutex<Vec<String>>,
    attached: AtomicUsize,
    deny: AtomicBool,
}

impl Fake {
    fn with(view: AccountView) -> Arc<Fake> {
        let fake = Arc::new(Fake::default());
        fake.set(view, None);
        fake
    }
    fn set(&self, view: AccountView, error: Option<AccountError>) {
        *self.snapshot.lock().unwrap() = AccountSnapshot { view, busy: false, error };
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

impl AccountHook for Fake {
    fn snapshot(&self) -> AccountSnapshot {
        self.snapshot.lock().unwrap().clone()
    }
    fn sign_in(&self) {
        self.record("sign_in".into());
    }
    fn cancel_sign_in(&self) {
        self.record("cancel_sign_in".into());
    }
    fn sign_out(&self) {
        self.record("sign_out".into());
    }
    fn select_org(&self, org_id: Option<String>) {
        self.record(format!("select_org:{org_id:?}"));
    }
    fn retry_activation(&self) {
        self.record("retry_activation".into());
    }
    fn allowed(&self, feature: &str) -> bool {
        self.record(format!("allowed:{feature}"));
        !self.deny.load(Ordering::SeqCst)
    }
    fn warning_shown(&self, days: u32) {
        self.record(format!("warning_shown:{days}"));
    }
    fn attach(&self, _ctx: &egui::Context) {
        self.attached.fetch_add(1, Ordering::SeqCst);
    }
}

fn signed_in(licence: LicenceView, warning: Option<WarningView>) -> AccountView {
    AccountView::SignedIn(SignedInView {
        name: "Nguyễn Văn An".into(),
        email: Some("an.nguyen@example.com".into()),
        orgs: vec![
            OrgView { id: "org_a".into(), name: "Công ty Xây dựng Ví dụ".into(), role: Some("member".into()) },
            OrgView { id: "org_b".into(), name: "Văn phòng Kiến trúc Mẫu".into(), role: None },
        ],
        active_org: None,
        licence,
        warning,
        online: true,
        session_only: false,
        reauth_required: false,
    })
}

fn harness(hook: Option<Arc<Fake>>, size: egui::Vec2, language: Language) -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.language = language;
    app.account_hook = hook.map(|hook| hook as Arc<dyn AccountHook>);
    let mut h = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn open_account_page(h: &mut Harness) {
    h.state_mut().open_backstage(Page::Account);
    h.run_steps(3);
}

/// Saves a render when `GRIDCRAFT_ACCOUNT_SCREENSHOTS` names a directory (for reviews; never asserted).
fn screenshot(h: &mut Harness, name: &str) {
    if let Some(dir) = std::env::var_os("GRIDCRAFT_ACCOUNT_SCREENSHOTS") {
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(std::path::Path::new(&dir).join(name)).unwrap();
    }
}

#[test]
fn without_a_hook_nothing_is_shown_and_everything_is_allowed() {
    let mut h = harness(None, DESKTOP, Language::En);
    assert!(h.query_by_label("Sign in").is_none());
    assert!(h.state().allowed("bangtinh.advanced"));
    assert!(h.state_mut().run("ui.backstage", serde_json::json!({"page": "account"})).is_err(), "no page by id");
    open_account_page(&mut h);
    assert_eq!(h.state().backstage.as_ref().map(|b| b.page), Some(Page::Home), "the page falls back to Home");
    assert!(h.query_by_label("Account").is_none());
    assert!(h.query_by_label("Sign in with PI ID").is_none());
}

#[test]
fn the_title_button_opens_the_guest_page_and_signs_in() {
    let fake = Fake::with(AccountView::Guest);
    let mut h = harness(Some(fake.clone()), DESKTOP, Language::En);
    screenshot(&mut h, "title-button-guest-en.png");
    h.get_by_label("Sign in").click();
    h.run_steps(3);
    assert_eq!(h.state().backstage.as_ref().map(|b| b.page), Some(Page::Account));
    h.get_by_label("Signing in is optional. Every feature works without an account.");
    assert!(h.get_all_by_label("Account").count() >= 2, "the heading and the navigation entry");
    screenshot(&mut h, "account-guest-en.png");
    h.get_by_label("Sign in with PI ID").click();
    h.run_steps(2);
    assert_eq!(fake.calls(), ["sign_in"]);
    assert_eq!(fake.attached.load(Ordering::SeqCst), 1, "attached once");
}

#[test]
fn signing_in_shows_the_browser_step_and_cancel_reaches_the_hook() {
    let fake = Fake::with(AccountView::SigningIn);
    let mut h = harness(Some(fake.clone()), DESKTOP, Language::En);
    open_account_page(&mut h);
    h.get_by_label("Continue in your browser…");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(fake.calls(), ["cancel_sign_in"]);
}

#[test]
fn the_signed_in_page_switches_organization_and_signs_out() {
    let fake = Fake::with(signed_in(LicenceView::Free, None));
    let mut h = harness(Some(fake.clone()), DESKTOP, Language::Vi);
    // The title button carries the user's name.
    h.get_by_label("Nguyễn Văn An").click();
    h.run_steps(3);
    assert!(h.get_all_by_label("Tài khoản").count() >= 2);
    h.get_by_label("an.nguyen@example.com");
    h.get_by_label("Miễn phí");
    h.get_by_label("Đã kết nối PI ID");
    screenshot(&mut h, "account-signed-in-vi.png");
    h.get_all_by_role(egui::accesskit::Role::ComboBox).next().expect("organization picker").click();
    h.run_steps(3);
    h.get_by_label("Văn phòng Kiến trúc Mẫu").click();
    h.run_steps(3);
    assert_eq!(fake.calls(), ["select_org:Some(\"org_b\")"]);
    h.get_by_label("Đăng xuất").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some("sign_out"));
}

#[test]
fn licence_lines_and_device_limit_retry() {
    let fake = Fake::with(signed_in(LicenceView::Active { expires_at: EXPIRES, pro: true }, None));
    let mut h = harness(Some(fake.clone()), DESKTOP, Language::En);
    open_account_page(&mut h);
    h.get_by_label(&format!("Valid until {}", format_date(Language::En, EXPIRES)));
    fake.set(signed_in(LicenceView::Inactive { expires_at: EXPIRES }, None), None);
    h.run_steps(2);
    h.get_by_label(&format!("Expired on {}", format_date(Language::En, EXPIRES)));
    fake.set(signed_in(LicenceView::DeviceCapReached, None), None);
    h.run_steps(2);
    h.get_by_label("Device limit reached");
    h.get_by_label("Retry").click();
    h.run_steps(2);
    assert_eq!(fake.calls(), ["retry_activation"]);
    let mut session_only = signed_in(LicenceView::SessionOnly, None);
    if let AccountView::SignedIn(s) = &mut session_only {
        s.online = false;
        s.reauth_required = true;
    }
    fake.set(session_only, None);
    h.run_steps(2);
    h.get_by_label("This session only (no credential store on this computer)");
    h.get_by_label("Offline: showing what is saved on this device");
    h.get_by_label("Sign in again").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some("sign_in"));
}

#[test]
fn the_lease_warning_shows_only_for_paid_features_and_dismissing_reports_it() {
    let warning = Some(WarningView { days: 3, expires_at: EXPIRES });
    let text = "Your offline license expires on";
    for licence in [LicenceView::Free, LicenceView::Active { expires_at: EXPIRES, pro: false }, LicenceView::Online { pro: true }] {
        let fake = Fake::with(signed_in(licence, warning));
        let h = harness(Some(fake), DESKTOP, Language::En);
        assert!(h.query_by_label_contains(text).is_none(), "{licence:?}");
    }
    let fake = Fake::with(signed_in(LicenceView::Active { expires_at: EXPIRES, pro: false }, None));
    assert!(harness(Some(fake), DESKTOP, Language::En).query_by_label_contains(text).is_none());

    let fake = Fake::with(signed_in(LicenceView::Active { expires_at: EXPIRES, pro: true }, warning));
    let mut h = harness(Some(fake.clone()), DESKTOP, Language::En);
    h.get_by_label_contains(&format_date(Language::En, EXPIRES));
    screenshot(&mut h, "lease-warning-en.png");
    h.get_by_label("Close").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains(text).is_none(), "hidden at once");
    assert_eq!(fake.calls(), ["warning_shown:3"]);
}

#[test]
fn errors_read_in_plain_language_and_a_cancel_is_silent() {
    let fake = Fake::with(AccountView::Guest);
    fake.set(AccountView::Guest, Some(AccountError::Network));
    let mut h = harness(Some(fake.clone()), DESKTOP, Language::Vi);
    open_account_page(&mut h);
    h.get_by_label("Không kết nối được PI ID. Kiểm tra mạng rồi thử lại.");
    screenshot(&mut h, "account-error-vi.png");
    fake.set(AccountView::Guest, Some(AccountError::RateLimited { retry_after_secs: Some(120) }));
    h.run_steps(2);
    h.get_by_label("PI ID đang bận. Hãy thử lại sau 2 phút.");
    fake.set(AccountView::Guest, Some(AccountError::Cancelled));
    h.run_steps(2);
    assert!(h.query_by_label_contains("thử lại").is_none(), "a cancelled sign-in is not an error");
    h.get_by_label("Đăng nhập bằng PI ID");
}

#[test]
fn allowed_asks_the_hook() {
    let fake = Fake::with(AccountView::Guest);
    fake.deny.store(true, Ordering::SeqCst);
    let h = harness(Some(fake.clone()), DESKTOP, Language::En);
    assert!(!h.state().allowed("bangtinh.advanced"));
    assert_eq!(fake.calls(), ["allowed:bangtinh.advanced"]);
}

type Layout3 = (egui::Rect, egui::Rect, Option<egui::Rect>);

fn title_layout(h: &Harness) -> Layout3 {
    h.ctx.data(|d| d.get_temp::<Layout3>(egui::Id::new(gridcraft_ui_egui::ribbon::TITLE_BAR_LAYOUT))).expect("title bar layout")
}

#[test]
fn the_account_button_never_crowds_a_narrow_title_bar() {
    for view in [AccountView::Guest, signed_in(LicenceView::Free, None)] {
        let fake = Fake::with(view);
        let mut h = harness(Some(fake), DESKTOP, Language::Vi);
        h.state_mut().session.active_mut().unwrap().title = "Báo cáo doanh thu quý bốn của chi nhánh miền Nam.xlsx".into();
        for width in [599.0, 480.0, 430.0, 390.0, 360.0, 320.0] {
            h.set_size(vec2(width, 844.0));
            h.run_steps(3);
            let (left, right, title) = title_layout(&h);
            assert!(left.right() <= right.left(), "controls overlap at {width}: {left:?} {right:?}");
            assert!(right.right() <= width + 0.5, "the right group stays on screen at {width}");
            if let Some(title) = title {
                assert!(title.left() >= left.right() && title.right() <= right.left(), "the title sits between them at {width}: {title:?}");
            }
        }
        h.set_size(vec2(390.0, 844.0));
        h.run_steps(3);
        assert!(title_layout(&h).2.is_some_and(|t| t.width() >= 40.0), "a phone still shows the title, elided");
        screenshot(&mut h, "title-bar-390-vi.png");
        // File › Account stays reachable when the button gives way.
        open_account_page(&mut h);
        assert!(h.get_all_by_label("Tài khoản").count() >= 1);
    }
}
