//! Native adapter for the optional PI ID account (`erpcons-account`). Installed only when the build
//! sets `ERPCONS_PI_ISSUER`; without it the app never creates the account, its thread, credential
//! store entries or files.

use std::sync::{
    Arc, Mutex, MutexGuard, OnceLock, PoisonError,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, Sender},
};
use std::time::{Duration, Instant};

use erpcons_account::{
    Account, ApiCode, Config, Error, LicenceState, MembershipStatus, Product, RevocationReason, SecretStore, Status, WarningStage,
};
use gridcraft_ui_egui::account::{
    AccountError, AccountHook, AccountSnapshot, AccountView, LicenceView, OrgView, SignedInView, WarningView, is_pro_feature,
};
use gridcraft_ui_egui::i18n::Language;

/// Read at build time only: a release names its issuer, and nothing at run time can point the app
/// at another one.
const ISSUER: Option<&str> = option_env!("ERPCONS_PI_ISSUER");
/// PI's end-user API base; unset, the library uses `{issuer}/api/v1/id`.
const API_BASE: Option<&str> = option_env!("ERPCONS_PI_API_BASE");

const REFRESH_EVERY: Duration = Duration::from_secs(3600);
const RETRY_OFFLINE: Duration = Duration::from_secs(600);
const MIN_RETRY: Duration = Duration::from_secs(60);

type Opener = Box<dyn Fn(&str) -> Result<(), String> + Send>;

enum Command {
    SignIn,
    SignOut,
    SelectOrg(Option<String>),
    RetryActivation,
    WarningShown(u32),
    Refresh,
}

#[derive(Default)]
struct Shared {
    snapshot: Mutex<AccountSnapshot>,
    ctx: Mutex<Option<egui::Context>>,
    /// Set by the worker once the account opened; `cancel_sign_in` must reach it while `sign_in` blocks.
    account: OnceLock<Arc<Account>>,
    /// User commands not yet finished (the page shows them as busy).
    pending: AtomicUsize,
    /// From the click until the browser flow (sign-in or organization switch) returns, so a
    /// background publish never replaces the waiting page.
    awaiting_browser: AtomicBool,
    /// The last command's error; a new command clears it at once.
    error: Mutex<Option<AccountError>>,
    refresh_queued: AtomicBool,
    /// A cancel that arrives before the browser flow starts listening would be lost by the library.
    cancel: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn repaint(&self) {
        if let Some(ctx) = lock(&self.ctx).as_ref() {
            ctx.request_repaint();
        }
    }
}

pub struct DesktopAccount {
    shared: Arc<Shared>,
    commands: Sender<Command>,
}

/// The account hook, when this build names a PI issuer.
pub fn install(language: Language) -> Option<DesktopAccount> {
    let issuer = ISSUER.filter(|issuer| !issuer.is_empty())?;
    let mut config = Config::new(issuer, Product::BangTinh, env!("CARGO_PKG_VERSION"));
    config.api_base = API_BASE.filter(|base| !base.is_empty()).map(str::to_owned);
    // PI has no Office product: no device activation or lease (ADR 0011).
    config.offline_licensing = false;
    config.ui_locale = Some(if language == Language::Vi { "vi" } else { "en" }.into());
    config.callback_success_html = callback_page(language, "Signed in to PI ID. You can close this tab and go back to the app.");
    config.callback_failure_html = callback_page(language, "Sign-in did not finish. Close this tab and try again in the app.");
    DesktopAccount::spawn(config, None, Box::new(crate::launch_with_system))
}

fn callback_page(language: Language, key: &str) -> String {
    let text = language.tr(key).replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!("<!doctype html><meta charset=\"utf-8\"><title>PI ID</title><p style=\"font:16px sans-serif;margin:3em\">{text}</p>")
}

impl DesktopAccount {
    /// An account over `store` instead of the OS credential store, signing in through the test browser.
    #[cfg(test)]
    fn new(config: Config, store: Arc<dyn SecretStore>) -> Option<Self> {
        Self::spawn(config, Some(store), Box::new(tests::browser))
    }

    fn spawn(config: Config, store: Option<Arc<dyn SecretStore>>, open: Opener) -> Option<Self> {
        let shared = Arc::new(Shared::default());
        let (commands, rx) = mpsc::channel();
        let worker = shared.clone();
        let spawned = std::thread::Builder::new().name("pi-account".into()).spawn(move || run(worker, config, store, open, rx));
        match spawned {
            Ok(_) => Some(Self { shared, commands }),
            Err(e) => {
                log::warn!("PI account worker not started: {e}");
                None
            }
        }
    }

    fn send(&self, command: Command) {
        let shared = &self.shared;
        shared.pending.fetch_add(1, Ordering::SeqCst);
        *lock(&shared.error) = None;
        {
            let mut snapshot = lock(&shared.snapshot);
            snapshot.busy = true;
            snapshot.error = None;
            if shared.awaiting_browser.load(Ordering::SeqCst) {
                snapshot.view = AccountView::SigningIn;
            }
        }
        if self.commands.send(command).is_err() {
            shared.pending.fetch_sub(1, Ordering::SeqCst);
            shared.awaiting_browser.store(false, Ordering::SeqCst);
            *lock(&shared.snapshot) = AccountSnapshot { error: Some(AccountError::Other), ..AccountSnapshot::default() };
        }
    }

    /// Sends a command that waits for the browser; the page shows the sign-in wait with Cancel.
    fn send_browser_flow(&self, command: Command) {
        self.shared.cancel.store(false, Ordering::SeqCst);
        self.shared.awaiting_browser.store(true, Ordering::SeqCst);
        self.send(command);
    }
}

impl AccountHook for DesktopAccount {
    fn snapshot(&self) -> AccountSnapshot {
        lock(&self.shared.snapshot).clone()
    }
    fn sign_in(&self) {
        self.send_browser_flow(Command::SignIn);
    }
    /// Also cancels an organization switch, which waits for the browser the same way.
    fn cancel_sign_in(&self) {
        self.shared.cancel.store(true, Ordering::SeqCst);
        if let Some(account) = self.shared.account.get() {
            account.cancel_sign_in();
        }
    }
    fn sign_out(&self) {
        self.send(Command::SignOut);
    }
    // PI switches organization only through its authorization page (ADR 0011).
    fn select_org(&self, org_id: Option<String>) {
        self.send_browser_flow(Command::SelectOrg(org_id));
    }
    fn retry_activation(&self) {
        self.send(Command::RetryActivation);
    }
    fn allowed(&self, feature: &str) -> bool {
        // Until the account has opened, answer as for a guest.
        let Some(account) = self.shared.account.get() else { return !is_pro_feature(feature) };
        let allowed = account.allowed(feature);
        if account.take_refresh_request() && !self.shared.refresh_queued.swap(true, Ordering::SeqCst) {
            let _ = self.commands.send(Command::Refresh);
        }
        allowed
    }
    fn warning_shown(&self, days: u32) {
        let _ = self.commands.send(Command::WarningShown(days));
    }
    fn attach(&self, ctx: &egui::Context) {
        *lock(&self.shared.ctx) = Some(ctx.clone());
        ctx.request_repaint();
    }
}

// The clippy ban on the clock protects the web build; this module is desktop only.
#[allow(clippy::disallowed_methods)]
fn now() -> Instant {
    Instant::now()
}

struct Worker {
    shared: Arc<Shared>,
    account: Arc<Account>,
    open: Opener,
    next_refresh: Instant,
}

fn run(shared: Arc<Shared>, config: Config, store: Option<Arc<dyn SecretStore>>, open: Opener, rx: Receiver<Command>) {
    let opened = match store {
        Some(store) => Account::new(config, store),
        None => Account::with_os_store(config),
    };
    let account = match opened {
        Ok(account) => Arc::new(account),
        Err(e) => {
            log::warn!("PI account unavailable: {e}");
            let unavailable = || {
                shared.pending.store(0, Ordering::SeqCst);
                shared.awaiting_browser.store(false, Ordering::SeqCst);
                *lock(&shared.snapshot) = AccountSnapshot { error: Some(AccountError::Other), ..AccountSnapshot::default() };
                shared.repaint();
            };
            unavailable();
            for _ in rx {
                unavailable();
            }
            return;
        }
    };
    let _ = shared.account.set(account.clone());
    if let Err(e) = account.reload() {
        log::warn!("PI session not read: {e}");
    }
    let mut worker = Worker { shared, account, open, next_refresh: now() };
    worker.publish();
    loop {
        match rx.recv_timeout(worker.next_refresh.saturating_duration_since(now())) {
            Ok(Command::Refresh) => {
                worker.shared.refresh_queued.store(false, Ordering::SeqCst);
                worker.refresh();
            }
            Ok(Command::WarningShown(days)) => worker.warning_shown(days),
            Ok(command) => {
                worker.handle(command);
                worker.shared.pending.fetch_sub(1, Ordering::SeqCst);
            }
            Err(RecvTimeoutError::Timeout) => worker.refresh(),
            Err(RecvTimeoutError::Disconnected) => return,
        }
        worker.publish();
    }
}

impl Worker {
    fn handle(&mut self, command: Command) {
        let result = match command {
            Command::SignIn => self.browser_flow(|account, open| account.sign_in(open)),
            Command::SignOut => self.account.sign_out(),
            Command::SelectOrg(org) => self.browser_flow(|account, open| account.select_org(org.as_deref(), open)),
            Command::RetryActivation => self.account.retry_activation(),
            Command::WarningShown(_) | Command::Refresh => Ok(()),
        };
        match result {
            Ok(()) => self.next_refresh = now() + REFRESH_EVERY,
            Err(e) => {
                log::warn!("PI account: {e}");
                self.set_error(error_view(&e));
            }
        }
    }

    /// Runs `flow` with the browser opener; a cancel that won before the browser opened reads as cancelled.
    fn browser_flow(&self, flow: impl FnOnce(&Account, &dyn Fn(&str) -> Result<(), String>) -> Result<(), Error>) -> Result<(), Error> {
        let cancel = &self.shared.cancel;
        let open = |url: &str| if cancel.load(Ordering::SeqCst) { Err("cancelled".into()) } else { (self.open)(url) };
        let result = flow(&self.account, &open);
        self.shared.awaiting_browser.store(false, Ordering::SeqCst);
        match result {
            Err(Error::Browser(_)) if cancel.load(Ordering::SeqCst) => Err(Error::Cancelled),
            other => other,
        }
    }

    fn set_error(&self, error: Option<AccountError>) {
        *lock(&self.shared.error) = error;
    }

    /// Start-up, hourly and on-demand sync. A failure that may pass on its own is not shown: the
    /// page already says when the app is offline.
    fn refresh(&mut self) {
        if !matches!(self.account.status(), Status::SignedIn(_)) {
            self.next_refresh = now() + REFRESH_EVERY;
            return;
        }
        match self.account.refresh() {
            Ok(()) => self.next_refresh = now() + REFRESH_EVERY,
            Err(e) if e.is_transient() => {
                log::info!("PI account refresh postponed: {e}");
                let wait = e.retry_after().map_or(RETRY_OFFLINE, |after| after.max(MIN_RETRY));
                self.next_refresh = now() + wait;
            }
            Err(e) => {
                log::warn!("PI account refresh: {e}");
                self.set_error(error_view(&e));
                self.next_refresh = now() + REFRESH_EVERY;
            }
        }
    }

    fn warning_shown(&self, days: u32) {
        let stage = match days {
            7 => WarningStage::Days7,
            3 => WarningStage::Days3,
            1 => WarningStage::Days1,
            _ => return,
        };
        if let Err(e) = self.account.mark_warning_shown(stage) {
            log::warn!("PI lease warning not recorded: {e}");
        }
    }

    fn publish(&self) {
        {
            // Held while reading the flags, so a click's optimistic state is never overwritten by an older view.
            let mut snapshot = lock(&self.shared.snapshot);
            let view = if self.shared.awaiting_browser.load(Ordering::SeqCst) { AccountView::SigningIn } else { view_of(self.account.status()) };
            *snapshot = AccountSnapshot { view, busy: self.shared.pending.load(Ordering::SeqCst) > 0, error: *lock(&self.shared.error) };
        }
        self.shared.repaint();
    }
}

fn view_of(status: Status) -> AccountView {
    let s = match status {
        Status::Guest => return AccountView::Guest,
        Status::SigningIn => return AccountView::SigningIn,
        Status::SignedIn(s) => *s,
    };
    let (name, email) = s.user.map(|u| (u.display_name, u.email)).unwrap_or_default();
    AccountView::SignedIn(SignedInView {
        name,
        email,
        orgs: s
            .orgs
            .into_iter()
            .filter(|m| m.status == MembershipStatus::Active)
            .map(|m| OrgView { id: m.org_id, name: m.name, role: Some(m.role).filter(|r| !r.is_empty()) })
            .collect(),
        active_org: s.active_org,
        licence: match s.licence {
            // `NotEntitled` needs offline licensing, which Office keeps off; it leaves the free features.
            LicenceState::NotActivated | LicenceState::NotEntitled => LicenceView::Free,
            LicenceState::Active { features, expires_at, .. } => LicenceView::Active { expires_at, pro: features.iter().any(|f| is_pro_feature(f)) },
            LicenceState::Inactive { expires_at, .. } => LicenceView::Inactive { expires_at },
            LicenceState::DeviceRevoked => LicenceView::DeviceRevoked,
            LicenceState::DeviceCapReached => LicenceView::DeviceCapReached,
            LicenceState::SessionOnly => LicenceView::SessionOnly,
        },
        warning: s.warning.map(|w| WarningView { days: w.stage.days(), expires_at: w.expires_at }),
        online: s.online,
        session_only: s.session_only,
        reauth_required: s.reauth_required,
    })
}

/// The error line for a failed command; `None` when there is nothing to tell the user.
fn error_view(error: &Error) -> Option<AccountError> {
    Some(match error {
        Error::SignInInProgress => return None,
        Error::Cancelled => AccountError::Cancelled,
        Error::SignInTimedOut | Error::Timeout => AccountError::Timeout,
        Error::Network(_) => AccountError::Network,
        Error::AuthorizationDenied(_) => AccountError::Denied,
        Error::InvalidGrant => AccountError::ReauthRequired,
        Error::UnknownOrg | Error::OrgUnavailable => AccountError::OrgUnavailable,
        Error::Revoked(RevocationReason::DeviceRevoked) => AccountError::DeviceRevoked,
        Error::Revoked(RevocationReason::MembershipRevoked) => AccountError::OrgUnavailable,
        Error::Revoked(RevocationReason::UserSuspended) => AccountError::Suspended,
        Error::Api(api) => match &api.code {
            ApiCode::Unauthenticated | ApiCode::InsufficientScope => AccountError::ReauthRequired,
            ApiCode::MembershipInactive => AccountError::OrgUnavailable,
            ApiCode::UserSuspended => AccountError::Suspended,
            ApiCode::DeviceRevoked | ApiCode::DeviceUnknown => AccountError::DeviceRevoked,
            ApiCode::Forbidden => AccountError::Denied,
            ApiCode::DeviceCapReached => AccountError::DeviceCap,
            ApiCode::ClientVersionUnsupported => AccountError::ClientTooOld,
            ApiCode::RateLimited | ApiCode::QuotaExceeded => {
                AccountError::RateLimited { retry_after_secs: api.retry_after.map(|after| after.as_secs()) }
            }
            ApiCode::DependencyUnavailable | ApiCode::IdempotencyInProgress => AccountError::ServiceUnavailable,
            _ => AccountError::Other,
        },
        _ => AccountError::Other,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use erpcons_account::{ApiError, ExpiryWarning, Membership, MemorySecretStore, PinnedKey, SignedIn, User};
    use pi_mock::{ORG_B, Scenario, Server};

    use super::*;

    /// The "system browser" of the tests: follows the authorization redirect to the loopback listener.
    pub(super) fn browser(url: &str) -> Result<(), String> {
        let url = url.to_owned();
        std::thread::spawn(move || {
            let _ = ureq::get(&url).call();
        });
        Ok(())
    }

    fn signed_in(licence: LicenceState) -> SignedIn {
        SignedIn {
            user: Some(User { user_id: "u1".into(), display_name: "Nguyễn Văn An".into(), email: Some("an@example.com".into()) }),
            orgs: vec![
                Membership { org_id: "o1".into(), name: "Công ty A".into(), role: "member".into(), status: MembershipStatus::Active },
                Membership { org_id: "o2".into(), name: "Công ty B".into(), role: String::new(), status: MembershipStatus::Active },
                Membership { org_id: "o3".into(), name: "Mời".into(), role: "member".into(), status: MembershipStatus::Invited },
            ],
            active_org: Some("o1".into()),
            licence,
            licences: BTreeMap::new(),
            warning: Some(ExpiryWarning { stage: WarningStage::Days3, expires_at: 99 }),
            online: false,
            session_only: false,
            reauth_required: true,
        }
    }

    fn signed_in_view(status: Status) -> SignedInView {
        match view_of(status) {
            AccountView::SignedIn(s) => s,
            other => panic!("expected signed in, got {other:?}"),
        }
    }

    #[test]
    fn status_maps_to_the_view() {
        assert_eq!(view_of(Status::Guest), AccountView::Guest);
        assert_eq!(view_of(Status::SigningIn), AccountView::SigningIn);
        let free = || LicenceState::Active { lease_id: "l".into(), features: vec!["bangtinh.view".into(), "bangtinh.use".into()], expires_at: 50 };
        let s = signed_in_view(Status::SignedIn(Box::new(signed_in(free()))));
        assert_eq!(s.name, "Nguyễn Văn An");
        assert_eq!(s.email.as_deref(), Some("an@example.com"));
        assert_eq!(s.orgs.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["o1", "o2"], "only active memberships can be picked");
        assert_eq!(s.orgs[0].role.as_deref(), Some("member"));
        assert_eq!(s.orgs[1].role, None);
        assert_eq!(s.active_org.as_deref(), Some("o1"));
        assert_eq!(s.licence, LicenceView::Active { expires_at: 50, pro: false });
        assert_eq!(s.warning, Some(WarningView { days: 3, expires_at: 99 }));
        assert!(!s.online && !s.session_only && s.reauth_required);
        let pro = LicenceState::Active { lease_id: "l".into(), features: vec!["bangtinh.advanced".into()], expires_at: 50 };
        assert_eq!(signed_in_view(Status::SignedIn(Box::new(signed_in(pro)))).licence, LicenceView::Active { expires_at: 50, pro: true });
        for (state, view) in [
            (LicenceState::NotActivated, LicenceView::Free),
            (LicenceState::Inactive { lease_id: "l".into(), expires_at: 7 }, LicenceView::Inactive { expires_at: 7 }),
            (LicenceState::DeviceRevoked, LicenceView::DeviceRevoked),
            (LicenceState::DeviceCapReached, LicenceView::DeviceCapReached),
            (LicenceState::SessionOnly, LicenceView::SessionOnly),
            (LicenceState::NotEntitled, LicenceView::Free),
        ] {
            assert_eq!(signed_in_view(Status::SignedIn(Box::new(signed_in(state)))).licence, view);
        }
        let mut anonymous = signed_in(LicenceState::NotActivated);
        anonymous.user = None;
        let s = signed_in_view(Status::SignedIn(Box::new(anonymous)));
        assert_eq!((s.name.as_str(), s.email), ("", None), "no profile yet");
    }

    fn api(status: u16, code: &str, retry_after: Option<u64>) -> Error {
        Error::Api(ApiError {
            status,
            code: ApiCode::from_parts(status, Some(code).filter(|c| !c.is_empty())),
            correlation_id: None,
            retry_after: retry_after.map(Duration::from_secs),
            max_devices: None,
            active_devices: None,
            errors: Box::default(),
        })
    }

    #[test]
    fn errors_map_to_plain_categories() {
        for (error, view) in [
            (Error::Cancelled, Some(AccountError::Cancelled)),
            (Error::SignInInProgress, None),
            (Error::SignInTimedOut, Some(AccountError::Timeout)),
            (Error::Timeout, Some(AccountError::Timeout)),
            (Error::Network("down".into()), Some(AccountError::Network)),
            (Error::AuthorizationDenied("access_denied".into()), Some(AccountError::Denied)),
            (Error::InvalidGrant, Some(AccountError::ReauthRequired)),
            (Error::OrgUnavailable, Some(AccountError::OrgUnavailable)),
            (Error::UnknownOrg, Some(AccountError::OrgUnavailable)),
            (Error::Revoked(RevocationReason::DeviceRevoked), Some(AccountError::DeviceRevoked)),
            (Error::Revoked(RevocationReason::UserSuspended), Some(AccountError::Suspended)),
            (Error::StateMismatch, Some(AccountError::Other)),
            (Error::Browser("no browser".into()), Some(AccountError::Other)),
            (api(401, "unauthenticated", None), Some(AccountError::ReauthRequired)),
            (api(403, "user_suspended", None), Some(AccountError::Suspended)),
            (api(403, "device_revoked", None), Some(AccountError::DeviceRevoked)),
            (api(403, "membership_inactive", None), Some(AccountError::OrgUnavailable)),
            (api(409, "device_cap_reached", None), Some(AccountError::DeviceCap)),
            (api(412, "client_version_unsupported", None), Some(AccountError::ClientTooOld)),
            (api(429, "rate_limited", Some(120)), Some(AccountError::RateLimited { retry_after_secs: Some(120) })),
            (api(429, "quota_exceeded", None), Some(AccountError::RateLimited { retry_after_secs: None })),
            (api(503, "dependency_unavailable", None), Some(AccountError::ServiceUnavailable)),
            (api(422, "validation_failed", None), Some(AccountError::Other)),
            // PI's envelope: UPPERCASE codes, a 401 without one, a bare 500 for unexpected failures.
            (api(401, "", None), Some(AccountError::ReauthRequired)),
            (api(403, "INSUFFICIENT_SCOPE", None), Some(AccountError::ReauthRequired)),
            (api(403, "ACCOUNT_LOCKED", None), Some(AccountError::Suspended)),
            (api(403, "MEMBERSHIP_INACTIVE", None), Some(AccountError::OrgUnavailable)),
            (api(409, "DEVICE_CAP", None), Some(AccountError::DeviceCap)),
            (api(409, "DEVICE_REVOKED", None), Some(AccountError::DeviceRevoked)),
            (api(500, "", None), Some(AccountError::ServiceUnavailable)),
        ] {
            assert_eq!(error_view(&error), view, "{error:?}");
        }
    }

    #[test]
    fn no_issuer_no_account() {
        // Test builds never set the issuer, so the app's normal start installs nothing.
        assert!(ISSUER.is_none_or(str::is_empty), "tests must be built without ERPCONS_PI_ISSUER");
        assert!(install(Language::Vi).is_none());
    }

    fn wait_until(hook: &DesktopAccount, what: &str, done: impl Fn(&AccountSnapshot) -> bool) -> AccountSnapshot {
        for _ in 0..600 {
            let snapshot = hook.snapshot();
            if done(&snapshot) {
                return snapshot;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for {what}: {:?}", hook.snapshot());
    }

    fn mock_config(server: &Server, dir: &tempfile::TempDir) -> Config {
        let mut config = Config::new(server.issuer(), Product::BangTinh, "0.6.5");
        config.platform_keys = vec![PinnedKey { kid: server.lease_key_id(), key: server.lease_public_key() }];
        config.state_dir = Some(dir.path().to_path_buf());
        config.timeout = Duration::from_secs(5);
        config.max_retry_wait = Duration::from_secs(3);
        config.sign_in_timeout = Duration::from_secs(20);
        config
    }

    fn token_requests(server: &Server) -> usize {
        server.requests().iter().filter(|r| r.path == "/id/token").count()
    }

    /// The active organization while signed in (`Some(None)`: personal).
    fn active_org(snapshot: &AccountSnapshot) -> Option<Option<&str>> {
        match &snapshot.view {
            AccountView::SignedIn(v) => Some(v.active_org.as_deref()),
            _ => None,
        }
    }

    #[test]
    fn signs_in_switches_organization_and_signs_out_against_pi_mock() {
        let server = Server::start(Scenario::default());
        let dir = tempfile::tempdir().unwrap();
        let hook = DesktopAccount::new(mock_config(&server, &dir), Arc::new(MemorySecretStore::new())).unwrap();
        wait_until(&hook, "the account to open", |_| hook.shared.account.get().is_some());
        let guest = wait_until(&hook, "guest", |s| !s.busy);
        assert_eq!(guest.view, AccountView::Guest);
        assert!(hook.allowed("bangtinh.use"));

        hook.sign_in();
        assert_eq!(hook.snapshot().view, AccountView::SigningIn, "the page shows the browser step at once");
        let signed = wait_until(&hook, "sign-in", |s| !s.busy && matches!(s.view, AccountView::SignedIn(_)));
        assert_eq!(signed.error, None);
        let AccountView::SignedIn(view) = signed.view else { panic!("not signed in") };
        assert_eq!(view.name, "Nguyễn Văn An");
        assert_eq!(view.email.as_deref(), Some("an.nguyen@example.com"));
        assert_eq!(view.orgs.len(), 2);
        assert_eq!(view.active_org, None);
        assert_eq!(view.licence, LicenceView::Free, "Office never activates a device");
        assert!(view.online && !view.session_only && !view.reauth_required);
        assert_eq!(view.warning, None);
        assert!(hook.allowed("bangtinh.use") && !hook.allowed("bangtinh.advanced"));

        hook.select_org(Some(ORG_B.into()));
        assert_eq!(hook.snapshot().view, AccountView::SigningIn, "the switch waits for the browser like sign-in");
        let switched = wait_until(&hook, "the organization switch", |s| !s.busy && active_org(s) == Some(Some(ORG_B)));
        assert_eq!(switched.error, None);
        hook.select_org(None);
        let personal = wait_until(&hook, "the switch back to personal", |s| !s.busy && active_org(s) == Some(None));
        assert_eq!(personal.error, None);
        let authorize: Vec<String> = server.requests().into_iter().filter(|r| r.path == "/id/authorize").map(|r| r.query).collect();
        assert_eq!(authorize.len(), 3, "sign-in and both switches go through PI's authorization page");
        assert!(authorize[1].contains(&format!("org_hint={ORG_B}")), "{}", authorize[1]);
        assert!(authorize[2].contains("prompt=select_account") && !authorize[2].contains("org_hint"), "{}", authorize[2]);

        hook.sign_out();
        let out = wait_until(&hook, "sign-out", |s| !s.busy && s.view == AccountView::Guest);
        assert_eq!(out.error, None);
        let requests = server.requests();
        assert!(requests.iter().all(|r| !r.path.contains("/devices") && !r.path.contains("/licenses")), "no device or lease call");
        let dir_text = dir.path().to_string_lossy().into_owned();
        assert!(requests.iter().all(|r| !r.body.contains(&dir_text) && !r.query.contains(&dir_text)), "no local path leaves the machine");
    }

    #[test]
    fn an_organization_switch_waits_for_the_browser_until_cancelled() {
        let server = Server::start(Scenario::default());
        let dir = tempfile::tempdir().unwrap();
        // The browser comes back only while `follow` is set; otherwise the flow waits for it.
        let follow = Arc::new(AtomicBool::new(true));
        let opened = Arc::new(AtomicUsize::new(0));
        let (follows, opens) = (follow.clone(), opened.clone());
        let open: Opener = Box::new(move |url| {
            opens.fetch_add(1, Ordering::SeqCst);
            if follows.load(Ordering::SeqCst) { browser(url) } else { Ok(()) }
        });
        let hook = DesktopAccount::spawn(mock_config(&server, &dir), Some(Arc::new(MemorySecretStore::new())), open).unwrap();
        wait_until(&hook, "guest", |s| !s.busy && hook.shared.account.get().is_some());
        hook.sign_in();
        wait_until(&hook, "sign-in", |s| !s.busy && active_org(s) == Some(None));
        follow.store(false, Ordering::SeqCst);
        let tokens = token_requests(&server);

        hook.select_org(Some(ORG_B.into()));
        wait_until(&hook, "the browser to open", |_| opened.load(Ordering::SeqCst) == 2);
        std::thread::sleep(Duration::from_millis(300));
        let waiting = hook.snapshot();
        assert_eq!(waiting.view, AccountView::SigningIn, "the page keeps the browser step and its Cancel");
        assert!(waiting.busy);
        hook.cancel_sign_in();
        let cancelled = wait_until(&hook, "the cancelled switch", |s| !s.busy && active_org(s).is_some());
        assert_eq!(cancelled.error, Some(AccountError::Cancelled));
        assert_eq!(active_org(&cancelled), Some(None), "the context is unchanged");

        hook.select_org(Some(ORG_B.into()));
        hook.cancel_sign_in();
        let early = wait_until(&hook, "the switch cancelled before the browser opened", |s| !s.busy && active_org(s).is_some());
        assert_eq!(early.error, Some(AccountError::Cancelled));
        assert_eq!(active_org(&early), Some(None));
        assert_eq!(token_requests(&server), tokens, "a cancelled switch exchanges no code");

        follow.store(true, Ordering::SeqCst);
        hook.select_org(Some(ORG_B.into()));
        let switched = wait_until(&hook, "a switch after the cancel", |s| !s.busy && active_org(s) == Some(Some(ORG_B)));
        assert_eq!(switched.error, None);
    }

    #[test]
    fn cancel_before_the_browser_opens_cancels_silently() {
        let server = Server::start(Scenario::default());
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::new(server.issuer(), Product::BangTinh, "0.6.5");
        config.state_dir = Some(dir.path().to_path_buf());
        config.sign_in_timeout = Duration::from_secs(20);
        // A browser that never comes back, so the cancel always wins.
        let hook = DesktopAccount::spawn(config, Some(Arc::new(MemorySecretStore::new())), Box::new(|_| Ok(()))).unwrap();
        wait_until(&hook, "guest", |s| !s.busy && hook.shared.account.get().is_some());
        hook.sign_in();
        hook.cancel_sign_in();
        let done = wait_until(&hook, "the cancelled sign-in", |s| !s.busy);
        assert_eq!(done.view, AccountView::Guest);
        assert_eq!(done.error, Some(AccountError::Cancelled));
        assert_eq!(token_requests(&server), 0);
    }
}
