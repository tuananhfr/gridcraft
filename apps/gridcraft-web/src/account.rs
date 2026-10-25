//! Bridge to the ERPCons site shell's `window.erpAccount` (v1). File › Account is a second view of
//! the shell's session: sign-in runs in the shell's popup, and there is no lease on the web.
//!
//! The shell's snapshots are read as JSON, so an unexpected shape reads as a guest, never a panic.

use gridcraft_ui_egui::account::{AccountError, AccountSnapshot, AccountView, LicenceView, OrgView, SignedInView, is_pro_feature};
use serde_json::Value;

const API_VERSION: u64 = 1;

/// Whether the shell offers accounts here (`disabled`: flag off or embed mode).
pub(crate) fn enabled(snapshot: &Value) -> bool {
    snapshot.get("version").and_then(Value::as_u64) == Some(API_VERSION)
        && snapshot.get("status").and_then(Value::as_str).is_some_and(|status| status != "disabled")
}

/// An `erpAccount` error (`{code, retryAfter}`, from a snapshot or a rejected promise).
pub(crate) fn error_of(error: &Value) -> Option<AccountError> {
    let code = error.get("code").and_then(Value::as_str)?;
    let retry_after_secs = error.get("retryAfter").and_then(Value::as_f64).filter(|s| s.is_finite() && *s >= 0.0).map(|s| s.ceil() as u64);
    Some(match code {
        "cancelled" => AccountError::Cancelled,
        "popup_blocked" => AccountError::PopupBlocked,
        "network" | "discovery_failed" => AccountError::Network,
        "timeout" => AccountError::Timeout,
        "access_denied" | "forbidden" => AccountError::Denied,
        "rate_limited" | "quota_exceeded" => AccountError::RateLimited { retry_after_secs },
        "dependency_unavailable" => AccountError::ServiceUnavailable,
        "user_suspended" => AccountError::Suspended,
        "membership_inactive" | "unknown_org" => AccountError::OrgUnavailable,
        "session_expired" | "insufficient_scope" | "invalid_grant" | "unauthenticated" => AccountError::ReauthRequired,
        "client_version_unsupported" => AccountError::ClientTooOld,
        "device_cap_reached" => AccountError::DeviceCap,
        "device_revoked" => AccountError::DeviceRevoked,
        _ => AccountError::Other,
    })
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// The account page's view of a shell snapshot; `busy` counts the requests still running.
pub(crate) fn snapshot_of(snapshot: &Value, busy: bool) -> AccountSnapshot {
    let error = snapshot.get("error").and_then(error_of);
    let view = match snapshot.get("status").and_then(Value::as_str) {
        Some("signing-in") => AccountView::SigningIn,
        Some("signed-in") => AccountView::SignedIn(signed_in(snapshot, error)),
        _ => AccountView::Guest,
    };
    AccountSnapshot { view, busy, error }
}

fn signed_in(snapshot: &Value, error: Option<AccountError>) -> SignedInView {
    let user = snapshot.get("user").unwrap_or(&Value::Null);
    let orgs = snapshot
        .get("orgs")
        .and_then(Value::as_array)
        .map(|orgs| {
            orgs.iter()
                .filter_map(|org| {
                    let id = text(org, "id")?;
                    Some(OrgView { name: text(org, "name").unwrap_or_else(|| id.clone()), role: text(org, "role"), id })
                })
                .collect()
        })
        .unwrap_or_default();
    // The shell gives the active organization as an object; a bare id is read too.
    let active_org = snapshot.get("activeOrg").and_then(|org| org.as_str().map(str::to_owned).or_else(|| text(org, "id")));
    let pro = snapshot
        .get("entitlements")
        .and_then(Value::as_array)
        .is_some_and(|list| list.iter().filter_map(|e| e.get("feature").and_then(Value::as_str)).any(is_pro_feature));
    SignedInView {
        name: text(user, "name").unwrap_or_default(),
        email: text(user, "email").filter(|e| !e.is_empty()),
        orgs,
        active_org,
        licence: if pro { LicenceView::Online { pro: true } } else { LicenceView::Free },
        warning: None,
        online: !matches!(error, Some(AccountError::Network | AccountError::Timeout)),
        session_only: false,
        reauth_required: error == Some(AccountError::ReauthRequired),
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::install;

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::cell::RefCell;

    use gridcraft_ui_egui::account::{AccountError, AccountHook, AccountSnapshot, is_pro_feature};
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast as _, JsValue};

    use super::{enabled, error_of, snapshot_of};

    struct Bridge {
        api: JsValue,
        ctx: Option<egui::Context>,
        last: serde_json::Value,
        /// A rejected request, shown until the next request or snapshot error.
        rejected: Option<AccountError>,
        running: usize,
        /// Subscribed for the page's lifetime.
        _listener: Closure<dyn FnMut(JsValue)>,
    }

    thread_local! {
        // JS handles are not `Send`; the hook itself stays a plain token.
        static BRIDGE: RefCell<Option<Bridge>> = const { RefCell::new(None) };
    }

    /// The hook; its state lives in [`BRIDGE`] on the page's only thread.
    pub struct WebAccount;

    fn api() -> Option<JsValue> {
        let window = web_sys::window()?;
        let api = js_sys::Reflect::get(&window, &JsValue::from_str("erpAccount")).ok()?;
        api.is_object().then_some(api)
    }

    fn method(api: &JsValue, name: &str) -> Option<js_sys::Function> {
        js_sys::Reflect::get(api, &JsValue::from_str(name)).ok()?.dyn_into().ok()
    }

    fn json(value: &JsValue) -> serde_json::Value {
        js_sys::JSON::stringify(value).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(serde_json::Value::Null)
    }

    fn read_snapshot(api: &JsValue) -> serde_json::Value {
        match method(api, "snapshot").map(|f| f.call0(api)) {
            Some(Ok(value)) => json(&value),
            Some(Err(e)) => {
                log::warn!("erpAccount.snapshot failed: {e:?}");
                serde_json::Value::Null
            }
            None => serde_json::Value::Null,
        }
    }

    fn with_bridge<R>(f: impl FnOnce(&mut Bridge) -> R) -> Option<R> {
        BRIDGE.with(|cell| cell.try_borrow_mut().ok().and_then(|mut bridge| bridge.as_mut().map(f)))
    }

    fn store(snapshot: serde_json::Value) {
        let ctx = with_bridge(|b| {
            if snapshot.get("error").is_some_and(|e| !e.is_null()) {
                b.rejected = None;
            }
            b.last = snapshot;
            b.ctx.clone()
        });
        if let Some(Some(ctx)) = ctx {
            ctx.request_repaint();
        }
    }

    /// The hook, when the shell offers accounts on this page.
    pub fn install() -> Option<WebAccount> {
        let api = api()?;
        let version = js_sys::Reflect::get(&api, &JsValue::from_str("version")).ok()?.as_f64();
        if version != Some(1.0) {
            return None;
        }
        let last = read_snapshot(&api);
        if !enabled(&last) {
            return None;
        }
        let listener = Closure::<dyn FnMut(JsValue)>::new(|snapshot: JsValue| store(json(&snapshot)));
        let subscribe = method(&api, "onChange")?;
        if let Err(e) = subscribe.call1(&api, listener.as_ref()) {
            log::warn!("erpAccount.onChange failed: {e:?}");
            return None;
        }
        BRIDGE.with(|cell| {
            if let Ok(mut bridge) = cell.try_borrow_mut() {
                *bridge = Some(Bridge { api, ctx: None, last, rejected: None, running: 0, _listener: listener });
            }
        });
        Some(WebAccount)
    }

    /// Calls `erpAccount.<name>(args)` without holding the bridge: the shell may notify synchronously.
    fn request(name: &str, args: &[JsValue], counts_as_busy: bool) {
        let Some(api) = with_bridge(|b| {
            b.rejected = None;
            if counts_as_busy {
                b.running += 1;
            }
            b.api.clone()
        }) else {
            return;
        };
        let called = method(&api, name)
            .ok_or_else(|| JsValue::from_str("not a function"))
            .and_then(|f| f.apply(&api, &args.iter().collect::<js_sys::Array>()));
        let settle = move |result: Result<JsValue, JsValue>| {
            let snapshot = result.as_ref().ok().map(json);
            let ctx = with_bridge(|b| {
                if counts_as_busy {
                    b.running = b.running.saturating_sub(1);
                }
                if let Err(e) = &result {
                    b.rejected = Some(error_of(&json(e)).unwrap_or(AccountError::Other));
                }
                b.ctx.clone()
            });
            if let Some(snapshot) = snapshot.filter(|s| s.is_object()) {
                store(snapshot);
            } else if let Some(Some(ctx)) = ctx {
                ctx.request_repaint();
            }
        };
        match called {
            Ok(value) => match value.dyn_into::<js_sys::Promise>() {
                Ok(promise) => wasm_bindgen_futures::spawn_local(async move { settle(wasm_bindgen_futures::JsFuture::from(promise).await) }),
                Err(value) => settle(Ok(value)),
            },
            Err(e) => {
                log::warn!("erpAccount.{name} failed: {e:?}");
                settle(Err(e));
            }
        }
    }

    impl AccountHook for WebAccount {
        fn snapshot(&self) -> AccountSnapshot {
            with_bridge(|b| {
                let mut snapshot = snapshot_of(&b.last, b.running > 0);
                if b.rejected.is_some() {
                    snapshot.error = b.rejected;
                }
                snapshot
            })
            .unwrap_or_default()
        }
        // Runs in the frame that handled the click, so the shell's popup keeps the user activation.
        fn sign_in(&self) {
            request("signIn", &[], false);
        }
        // The shell owns the popup; closing it cancels.
        fn cancel_sign_in(&self) {}
        fn sign_out(&self) {
            request("signOut", &[], true);
        }
        fn select_org(&self, org_id: Option<String>) {
            let org = org_id.map_or(JsValue::NULL, |id| JsValue::from_str(&id));
            request("selectOrg", &[org], true);
        }
        // No device activation on the web.
        fn retry_activation(&self) {}
        fn allowed(&self, feature: &str) -> bool {
            let fallback = !is_pro_feature(feature);
            let Some(api) = with_bridge(|b| b.api.clone()) else { return fallback };
            match method(&api, "allowed").map(|f| f.call1(&api, &JsValue::from_str(feature))) {
                Some(Ok(answer)) => answer.as_bool().unwrap_or(fallback),
                _ => fallback,
            }
        }
        // No lease, so no lease warning, on the web.
        fn warning_shown(&self, _days: u32) {}
        fn attach(&self, ctx: &egui::Context) {
            with_bridge(|b| b.ctx = Some(ctx.clone()));
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn signed_in_snapshot() -> Value {
        json!({
            "version": 1,
            "status": "signed-in",
            "user": {"id": "u1", "name": "Nguyễn Văn An", "email": "an@example.com"},
            "orgs": [{"id": "o1", "name": "Công ty A", "role": "member"}, {"id": "o2", "name": "Công ty B", "role": null}, {"name": "no id"}],
            "activeOrg": {"id": "o2", "name": "Công ty B", "role": null},
            "product": "bangtinh",
            "entitlements": [{"id": "e1", "feature": "bangtinh.use", "scope": "personal", "orgId": null, "validTo": null, "offlineAllowed": true}],
            "entitlementsFresh": true,
            "error": null
        })
    }

    #[test]
    fn only_an_enabled_v1_shell_installs_the_hook() {
        assert!(enabled(&json!({"version": 1, "status": "signed-out"})));
        assert!(enabled(&signed_in_snapshot()));
        assert!(!enabled(&json!({"version": 1, "status": "disabled"})));
        assert!(!enabled(&json!({"version": 2, "status": "signed-out"})));
        assert!(!enabled(&json!({"status": "signed-out"})));
        assert!(!enabled(&Value::Null));
    }

    #[test]
    fn a_signed_in_snapshot_maps_to_the_page() {
        let s = snapshot_of(&signed_in_snapshot(), false);
        assert!(!s.busy);
        assert_eq!(s.error, None);
        let AccountView::SignedIn(view) = s.view else { panic!("not signed in: {:?}", s.view) };
        assert_eq!(view.name, "Nguyễn Văn An");
        assert_eq!(view.email.as_deref(), Some("an@example.com"));
        assert_eq!(view.orgs.len(), 2, "an organization without an id is skipped");
        assert_eq!(view.orgs[0], OrgView { id: "o1".into(), name: "Công ty A".into(), role: Some("member".into()) });
        assert_eq!(view.orgs[1].role, None);
        assert_eq!(view.active_org.as_deref(), Some("o2"));
        assert_eq!(view.licence, LicenceView::Free);
        assert!(view.online && !view.session_only && !view.reauth_required && view.warning.is_none());

        let mut pro = signed_in_snapshot();
        pro["entitlements"] = json!([{"feature": "bangtinh.advanced"}]);
        pro["activeOrg"] = json!("o1");
        let AccountView::SignedIn(view) = snapshot_of(&pro, true).view else { panic!() };
        assert_eq!(view.licence, LicenceView::Online { pro: true });
        assert_eq!(view.active_org.as_deref(), Some("o1"), "a bare id is read too");
    }

    #[test]
    fn statuses_and_errors_map_without_panicking() {
        assert_eq!(snapshot_of(&json!({"version": 1, "status": "signing-in"}), false).view, AccountView::SigningIn);
        assert_eq!(snapshot_of(&json!({"version": 1, "status": "signed-out"}), false).view, AccountView::Guest);
        for odd in
            [Value::Null, json!([]), json!("x"), json!({"status": 7}), json!({"status": "signed-in", "user": 3, "orgs": {}, "entitlements": "no"})]
        {
            let s = snapshot_of(&odd, false);
            assert!(matches!(s.view, AccountView::Guest | AccountView::SignedIn(_)), "{odd}");
        }
        let limited = json!({"version": 1, "status": "signed-out", "error": {"code": "rate_limited", "message": "…", "retryAfter": 30}});
        assert_eq!(snapshot_of(&limited, false).error, Some(AccountError::RateLimited { retry_after_secs: Some(30) }));
        for (code, error) in [
            ("cancelled", AccountError::Cancelled),
            ("network", AccountError::Network),
            ("discovery_failed", AccountError::Network),
            ("user_suspended", AccountError::Suspended),
            ("membership_inactive", AccountError::OrgUnavailable),
            ("unknown_org", AccountError::OrgUnavailable),
            ("session_expired", AccountError::ReauthRequired),
            ("dependency_unavailable", AccountError::ServiceUnavailable),
            ("popup_blocked", AccountError::PopupBlocked),
        ] {
            assert_eq!(error_of(&json!({"code": code, "retryAfter": null})), Some(error), "{code}");
        }
        assert_eq!(error_of(&json!({"retryAfter": -1})), None);
        assert_eq!(error_of(&json!({"code": "rate_limited", "retryAfter": -5})), Some(AccountError::RateLimited { retry_after_secs: None }));
        let mut offline = signed_in_snapshot();
        offline["error"] = json!({"code": "network", "message": "", "retryAfter": null});
        let AccountView::SignedIn(view) = snapshot_of(&offline, false).view else { panic!() };
        assert!(!view.online);
    }
}
