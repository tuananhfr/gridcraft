//! Optional PI ID account: the hook the desktop and web hosts implement, the plain view types they
//! map their state into, and the UI (File › Account, the title-bar button, the lease warning).
//!
//! Without a hook nothing here is drawn and [`SheetApp::allowed`] allows everything.

use egui::{Align2, Color32, RichText, Sense, Ui, vec2};

use crate::SheetApp;
use crate::backstage::Page;
use crate::i18n::Language;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// Implemented by the native and web adapters. Every method returns at once; work runs on the
/// adapter's own worker (thread on desktop, promise on web) and the result shows up in the next
/// `snapshot()`.
pub trait AccountHook {
    fn snapshot(&self) -> AccountSnapshot;
    fn sign_in(&self);
    fn cancel_sign_in(&self);
    fn sign_out(&self);
    fn select_org(&self, org_id: Option<String>);
    fn retry_activation(&self);
    fn allowed(&self, feature: &str) -> bool;
    fn warning_shown(&self, days: u32);
    /// Called once with the egui context so the adapter can request a repaint when its snapshot
    /// changes.
    fn attach(&self, ctx: &egui::Context);
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountSnapshot {
    pub view: AccountView,
    pub busy: bool,
    pub error: Option<AccountError>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AccountView {
    #[default]
    Guest,
    SigningIn,
    SignedIn(SignedInView),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedInView {
    pub name: String,
    pub email: Option<String>,
    pub orgs: Vec<OrgView>,
    pub active_org: Option<String>,
    pub licence: LicenceView,
    pub warning: Option<WarningView>,
    pub online: bool,
    pub session_only: bool,
    pub reauth_required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrgView {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LicenceView {
    /// No non-free entitlement (normal today).
    Free,
    /// Desktop lease.
    Active {
        expires_at: i64,
        pro: bool,
    },
    /// Web: no lease, entitlements read online.
    Online {
        pro: bool,
    },
    Inactive {
        expires_at: i64,
    },
    DeviceRevoked,
    DeviceCapReached,
    SessionOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WarningView {
    pub days: u32,
    pub expires_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountError {
    Cancelled,
    PopupBlocked,
    Network,
    Timeout,
    Denied,
    RateLimited { retry_after_secs: Option<u64> },
    DeviceCap,
    DeviceRevoked,
    Suspended,
    OrgUnavailable,
    ReauthRequired,
    ClientTooOld,
    ServiceUnavailable,
    Other,
}

/// A feature that needs a paid entitlement: anything but the built-in free set (products.md §2).
pub fn is_pro_feature(feature: &str) -> bool {
    !(feature.ends_with(".view") || feature.ends_with(".export") || feature.ends_with(".use"))
}

/// The avatar letter: the given name's first letter (Vietnamese names put it last, as the office
/// shell does), else the email's.
pub fn avatar_letter(name: &str, email: Option<&str>) -> Option<String> {
    let given = name.split_whitespace().next_back().unwrap_or_default();
    given.chars().chain(email.unwrap_or_default().chars()).find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect())
}

/// The UI's copy of the hook's state, refreshed once per frame.
#[derive(Default)]
pub(crate) struct AccountState {
    attached: bool,
    snapshot: AccountSnapshot,
    /// The warning (days, lease expiry) dismissed here, hidden before the hook reports it shown.
    dismissed_warning: Option<(u32, i64)>,
}

impl SheetApp {
    /// Whether `feature` may be used: always without an account hook (every feature is free).
    pub fn allowed(&self, feature: &str) -> bool {
        self.account_hook.as_ref().is_none_or(|hook| hook.allowed(feature))
    }
}

pub(crate) fn poll(app: &mut SheetApp, ctx: &egui::Context) {
    let Some(hook) = &app.account_hook else { return };
    if !app.account.attached {
        app.account.attached = true;
        hook.attach(ctx);
    }
    app.account.snapshot = hook.snapshot();
}

/// The local calendar date of `unix`: `dd/mm/yyyy` in Vietnamese, Excel's `m/d/yyyy` otherwise.
pub fn format_date(language: Language, unix: i64) -> String {
    let Some(time) = chrono::DateTime::from_timestamp(unix, 0) else { return String::new() };
    let pattern = if language == Language::Vi { "%d/%m/%Y" } else { "%-m/%-d/%Y" };
    time.with_timezone(&chrono::Local).format(pattern).to_string()
}

/// The plain-language error line; `None` for a cancelled sign-in, which is not an error.
pub fn error_text(language: Language, error: AccountError) -> Option<String> {
    let key = match error {
        AccountError::Cancelled => return None,
        AccountError::PopupBlocked => "The browser blocked the sign-in window. Allow pop-ups for this site and try again.",
        AccountError::Network => "Could not reach PI ID. Check your connection and try again.",
        AccountError::Timeout => "PI ID did not answer in time. Try again.",
        AccountError::Denied => "PI ID refused the request.",
        AccountError::RateLimited { retry_after_secs: Some(secs) } => {
            let minutes = secs.div_ceil(60).max(1);
            return Some(language.tr("PI ID is busy. Try again in {minutes} min.").replace("{minutes}", &minutes.to_string()));
        }
        AccountError::RateLimited { retry_after_secs: None } => "PI ID is busy. Try again in a few minutes.",
        AccountError::DeviceCap => "Your account has reached its device limit. Remove a device in PI ID, then retry.",
        AccountError::DeviceRevoked => "This device was removed from your account. Sign in again to use it.",
        AccountError::Suspended => "Your PI ID account is suspended. Every free feature still works.",
        AccountError::OrgUnavailable => "This organization is not available to your account.",
        AccountError::ReauthRequired => "Your session has expired. Sign in again.",
        AccountError::ClientTooOld => "This version can no longer sign in. Update the app, then try again.",
        AccountError::ServiceUnavailable => "PI ID is temporarily unavailable. Try again later.",
        AccountError::Other => "Something went wrong with PI ID. Try again.",
    };
    Some(language.tr(key).to_string())
}

pub fn licence_text(language: Language, licence: LicenceView) -> String {
    let dated = |key: &str, at: i64| language.tr(key).replace("{date}", &format_date(language, at));
    match licence {
        LicenceView::Free | LicenceView::Online { pro: false } => language.tr("Free").into(),
        LicenceView::Active { expires_at, pro: false } => dated("Free · offline until {date}", expires_at),
        LicenceView::Active { expires_at, pro: true } => dated("Valid until {date}", expires_at),
        LicenceView::Online { pro: true } => language.tr("Paid features active").into(),
        LicenceView::Inactive { expires_at } => dated("Expired on {date}", expires_at),
        LicenceView::DeviceRevoked => language.tr("This device was removed from your account").into(),
        LicenceView::DeviceCapReached => language.tr("Device limit reached").into(),
        LicenceView::SessionOnly => language.tr("This session only (no credential store on this computer)").into(),
    }
}

fn display_name(view: &SignedInView) -> &str {
    if !view.name.trim().is_empty() { view.name.trim() } else { view.email.as_deref().filter(|e| !e.is_empty()).unwrap_or("PI ID") }
}

/// The title-bar button that opens File › Account: the user's initial, or a person when signed out.
pub(crate) fn title_button(app: &mut SheetApp, ui: &mut Ui, size: egui::Vec2) {
    if app.account_hook.is_none() {
        return;
    }
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    let (tip, initial) = match &app.account.snapshot.view {
        AccountView::SignedIn(s) => {
            let name = display_name(s);
            (name.to_string(), avatar_letter(&s.name, s.email.as_deref()))
        }
        AccountView::SigningIn => (lang.tr("Signing in…").to_string(), None),
        AccountView::Guest => (lang.tr("Sign in").to_string(), None),
    };
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 4.0, t.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    match initial {
        Some(initial) => {
            let radius = (rect.height().min(rect.width()) / 2.0 - 3.0).max(6.0);
            ui.painter().circle_filled(rect.center(), radius, t.accent);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, initial, theme::ui_bold(11.0), Color32::WHITE);
        }
        None => icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), Icon::Person, t.text_dim),
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &tip));
    if resp.on_hover_text(&tip).clicked() {
        app.open_backstage(Page::Account);
    }
}

/// File › Account.
pub(crate) fn page(app: &mut SheetApp, ui: &mut Ui) {
    let Some(hook) = app.account_hook.clone() else { return };
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    let snapshot = app.account.snapshot.clone();
    let busy = snapshot.busy;
    match &snapshot.view {
        AccountView::Guest => {
            ui.label(RichText::new(lang.tr("Signing in is optional. Every feature works without an account.")).color(t.text_dim));
            ui.add_space(14.0);
            if ui.add_enabled(!busy, accent_button(lang.tr("Sign in with PI ID"), &t)).clicked() {
                hook.sign_in();
            }
        }
        AccountView::SigningIn => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(lang.tr("Continue in your browser…")).color(t.text));
            });
            ui.add_space(10.0);
            if ui.button(lang.tr("Cancel")).clicked() {
                hook.cancel_sign_in();
            }
        }
        AccountView::SignedIn(s) => signed_in(hook.as_ref(), lang, ui, s, busy, &t),
    }
    if let Some(text) = snapshot.error.and_then(|e| error_text(lang, e)) {
        ui.add_space(14.0);
        ui.label(RichText::new(text).color(t.danger));
    }
}

/// Narrower than this, the account details stack (caption over value) instead of two columns.
const TWO_COLUMNS_MIN_W: f32 = 480.0;
const ORG_PICKER_W: f32 = 260.0;

fn signed_in(hook: &dyn AccountHook, lang: Language, ui: &mut Ui, s: &SignedInView, busy: bool, t: &Tokens) {
    // One line each, elided with the full text on hover: wrapping would split an address mid-word.
    ui.add(egui::Label::new(RichText::new(display_name(s)).font(theme::ui_bold(18.0)).color(t.text)).truncate());
    if let Some(email) = s.email.as_deref().filter(|e| !e.is_empty() && *e != display_name(s)) {
        ui.add(egui::Label::new(RichText::new(email).font(theme::ui_font(12.5)).color(t.text_dim)).truncate());
    }
    ui.add_space(16.0);
    let organization = |ui: &mut Ui| org_picker(hook, lang, ui, s, busy);
    let licence = |ui: &mut Ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(licence_text(lang, s.licence));
            if s.licence == LicenceView::DeviceCapReached && ui.add_enabled(!busy, egui::Button::new(lang.tr("Retry"))).clicked() {
                hook.retry_activation();
            }
        });
    };
    let connection = |ui: &mut Ui| {
        ui.label(lang.tr(if s.online { "Connected to PI ID" } else { "Offline: showing what is saved on this device" }));
    };
    let rows: [(&str, &dyn Fn(&mut Ui)); 3] = [("Organization", &organization), ("License", &licence), ("Connection", &connection)];
    if ui.available_width() >= TWO_COLUMNS_MIN_W {
        egui::Grid::new("account_details").num_columns(2).spacing(vec2(18.0, 10.0)).show(ui, |ui| {
            for (caption, value) in rows {
                ui.label(lang.tr(caption));
                value(ui);
                ui.end_row();
            }
        });
    } else {
        for (caption, value) in rows {
            ui.label(RichText::new(lang.tr(caption)).color(t.text_dim));
            ui.add_space(2.0);
            value(ui);
            ui.add_space(10.0);
        }
    }
    if s.reauth_required {
        ui.add_space(14.0);
        ui.label(RichText::new(lang.tr("Your session has expired. Sign in again to keep using your account.")).color(t.text));
        ui.add_space(6.0);
        if ui.add_enabled(!busy, accent_button(lang.tr("Sign in again"), t)).clicked() {
            hook.sign_in();
        }
    }
    ui.add_space(18.0);
    ui.horizontal(|ui| {
        if ui.add_enabled(!busy, egui::Button::new(lang.tr("Sign out"))).clicked() {
            hook.sign_out();
        }
        if busy {
            ui.spinner();
        }
    });
}

fn org_picker(hook: &dyn AccountHook, lang: Language, ui: &mut Ui, s: &SignedInView, busy: bool) {
    let active = s.active_org.as_deref();
    let selected = match active {
        Some(id) => s.orgs.iter().find(|o| o.id == id).map_or(id, |o| o.name.as_str()).to_string(),
        None => lang.tr("Personal").to_string(),
    };
    // ComboBox::width is only a minimum: a long organization name would widen it past the page.
    let size = vec2(ui.available_width().min(ORG_PICKER_W), ui.spacing().interact_size.y);
    ui.allocate_ui(size, |ui| {
        ui.add_enabled_ui(!busy, |ui| {
            egui::ComboBox::from_id_salt("account_org").width(size.x).truncate().selected_text(selected).show_ui(ui, |ui| {
                if ui.add(egui::Button::selectable(active.is_none(), lang.tr("Personal")).truncate()).clicked() && active.is_some() {
                    hook.select_org(None);
                }
                for org in &s.orgs {
                    let current = active == Some(org.id.as_str());
                    let item = ui.add(egui::Button::selectable(current, &org.name).truncate()).on_hover_text(&org.name);
                    if item.clicked() && !current {
                        hook.select_org(Some(org.id.clone()));
                    }
                }
            });
        });
    });
}

fn accent_button(label: &str, t: &Tokens) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label.to_string()).color(t.window).strong()).fill(t.accent).corner_radius(16)
}

/// The 7/3/1-day lease warning (lease.md §8), only while the lease grants a paid feature.
pub(crate) fn warning_notice(app: &mut SheetApp, ctx: &egui::Context) {
    let Some(hook) = app.account_hook.clone() else { return };
    let AccountView::SignedIn(s) = &app.account.snapshot.view else { return };
    let (LicenceView::Active { pro: true, .. }, Some(warning)) = (s.licence, s.warning) else { return };
    if app.account.dismissed_warning == Some((warning.days, warning.expires_at)) {
        return;
    }
    let lang = app.ui.language;
    let text = lang
        .tr("Your offline license expires on {date}. Connect to the internet to renew it.")
        .replace("{date}", &format_date(lang, warning.expires_at));
    let mut dismissed = false;
    let mut open_page = false;
    egui::Area::new(egui::Id::new("account-lease-warning")).anchor(Align2::RIGHT_TOP, vec2(-16.0, 56.0)).order(egui::Order::Foreground).show(
        ctx,
        |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width((ctx.content_rect().width() - 48.0).clamp(120.0, 360.0));
                ui.label(text);
                ui.horizontal_wrapped(|ui| {
                    open_page = ui.button(lang.tr("Account")).clicked();
                    dismissed = ui.button(lang.tr("Close")).clicked();
                });
            });
        },
    );
    if open_page {
        app.open_backstage(Page::Account);
    }
    if dismissed {
        app.account.dismissed_warning = Some((warning.days, warning.expires_at));
        hook.warning_shown(warning.days);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_avatar_shows_the_given_name_like_the_office_shell() {
        assert_eq!(super::avatar_letter("Nguyễn Văn An", None).as_deref(), Some("A"));
        assert_eq!(super::avatar_letter(" ", Some("binh@example.com")).as_deref(), Some("B"));
        assert_eq!(super::avatar_letter("", None), None);
    }

    #[test]
    fn only_features_outside_the_free_set_are_pro() {
        for free in ["bangtinh.view", "bangtinh.export", "bangtinh.use", "vanban.view"] {
            assert!(!is_pro_feature(free), "{free}");
        }
        for pro in ["bangtinh.advanced", "bangtinh.macro.run", "bangtinh.viewer"] {
            assert!(is_pro_feature(pro), "{pro}");
        }
    }

    #[test]
    fn dates_follow_the_interface_language() {
        // 15 March 2027 at noon UTC: the same calendar day in every time zone the tests run in.
        let noon = 1_805_112_000;
        let vi = format_date(Language::Vi, noon);
        let en = format_date(Language::En, noon);
        assert!(vi.ends_with("/03/2027") && vi.len() == 10, "{vi}");
        assert!(en.starts_with("3/") && en.ends_with("/2027"), "{en}");
        assert_eq!(format_date(Language::En, i64::MAX), "");
    }

    #[test]
    fn a_cancelled_sign_in_shows_no_error_and_rate_limits_say_when_to_retry() {
        assert_eq!(error_text(Language::En, AccountError::Cancelled), None);
        assert_eq!(error_text(Language::En, AccountError::RateLimited { retry_after_secs: Some(90) }).unwrap(), "PI ID is busy. Try again in 2 min.");
        assert_eq!(
            error_text(Language::Vi, AccountError::RateLimited { retry_after_secs: Some(1) }).unwrap(),
            "PI ID đang bận. Hãy thử lại sau 1 phút."
        );
        let all = [
            AccountError::PopupBlocked,
            AccountError::Network,
            AccountError::Timeout,
            AccountError::Denied,
            AccountError::RateLimited { retry_after_secs: None },
            AccountError::DeviceCap,
            AccountError::DeviceRevoked,
            AccountError::Suspended,
            AccountError::OrgUnavailable,
            AccountError::ReauthRequired,
            AccountError::ClientTooOld,
            AccountError::ServiceUnavailable,
            AccountError::Other,
        ];
        for error in all {
            let en = error_text(Language::En, error).unwrap();
            let vi = error_text(Language::Vi, error).unwrap();
            assert_ne!(en, vi, "untranslated: {en}");
        }
    }

    #[test]
    fn every_licence_line_is_translated() {
        let at = 1_805_112_000;
        for licence in [
            LicenceView::Free,
            LicenceView::Active { expires_at: at, pro: false },
            LicenceView::Active { expires_at: at, pro: true },
            LicenceView::Online { pro: true },
            LicenceView::Inactive { expires_at: at },
            LicenceView::DeviceRevoked,
            LicenceView::DeviceCapReached,
            LicenceView::SessionOnly,
        ] {
            assert_ne!(licence_text(Language::En, licence), licence_text(Language::Vi, licence), "{licence:?}");
        }
        assert_eq!(licence_text(Language::En, LicenceView::Online { pro: false }), "Free");
    }
}
