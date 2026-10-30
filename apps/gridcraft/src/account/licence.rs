//! The edit gate: editing needs `erpconsoffice.use` in an active lease of this device.

use erpcons_account::{LicenceState, OFFICE_USE, Status};
use gridcraft_ui_egui::edit_lock::EditLock;

/// Why editing is locked for `status`, `None` when it is allowed; `allowed` is asked only while
/// the lease lists the feature, where it answers from the lease and requests no refresh.
pub(super) fn lock_of(status: &Status, allowed: impl FnOnce(&str) -> bool) -> Option<EditLock> {
    let signed_in = match status {
        Status::Guest | Status::SigningIn => return Some(EditLock::SignIn),
        Status::SignedIn(signed_in) => signed_in,
    };
    // PI licenses organizations only; each Office user gets a one-person one (ADR 0013).
    if signed_in.active_org.is_none() {
        return Some(EditLock::ChooseOrg);
    }
    // `allowed()` alone also grants on fresh online entitlements; only this device's lease
    // reflects a revoked or capped device, so it must grant too.
    match &signed_in.licence {
        LicenceState::DeviceRevoked => Some(EditLock::Revoked),
        LicenceState::DeviceCapReached => Some(EditLock::DeviceCap),
        LicenceState::NotEntitled => Some(EditLock::NoPlan),
        LicenceState::NotActivated => Some(EditLock::NotActivated),
        LicenceState::SessionOnly => Some(EditLock::SessionOnly),
        LicenceState::Inactive { .. } if !signed_in.online => Some(EditLock::RenewOffline),
        // Online yet expired: renewal failed or has not run yet, which is not "no plan".
        LicenceState::Inactive { .. } => Some(EditLock::RenewFailed),
        LicenceState::Active { features, .. } if features.iter().any(|f| f == OFFICE_USE) && allowed(OFFICE_USE) => None,
        LicenceState::Active { .. } => Some(EditLock::NoPlan),
    }
}

#[cfg(test)]
mod tests {
    use erpcons_account::{OFFICE_VIEW, SignedIn};

    use super::*;

    fn signed_in(active_org: Option<&str>, licence: LicenceState, online: bool) -> Status {
        Status::SignedIn(Box::new(SignedIn {
            user: None,
            orgs: Vec::new(),
            active_org: active_org.map(str::to_owned),
            licence,
            licences: Default::default(),
            warning: None,
            online,
            session_only: false,
            reauth_required: false,
        }))
    }

    fn active(features: &[&str]) -> LicenceState {
        LicenceState::Active { lease_id: "l".into(), features: features.iter().map(|f| f.to_string()).collect(), expires_at: 99 }
    }

    #[test]
    fn each_state_has_its_reason() {
        assert_eq!(lock_of(&Status::Guest, |_| true), Some(EditLock::SignIn));
        assert_eq!(lock_of(&Status::SigningIn, |_| true), Some(EditLock::SignIn));
        assert_eq!(lock_of(&signed_in(None, active(&[OFFICE_USE]), true), |_| true), Some(EditLock::ChooseOrg), "personal context");
        let inactive = || LicenceState::Inactive { lease_id: "l".into(), expires_at: 9 };
        for (licence, online, lock) in [
            (LicenceState::DeviceRevoked, true, Some(EditLock::Revoked)),
            (LicenceState::DeviceCapReached, true, Some(EditLock::DeviceCap)),
            (LicenceState::NotEntitled, true, Some(EditLock::NoPlan)),
            (LicenceState::NotActivated, true, Some(EditLock::NotActivated)),
            (LicenceState::SessionOnly, true, Some(EditLock::SessionOnly)),
            (inactive(), false, Some(EditLock::RenewOffline)),
            (inactive(), true, Some(EditLock::RenewFailed)),
            (active(&[OFFICE_VIEW]), true, Some(EditLock::NoPlan)),
            (active(&[OFFICE_VIEW, OFFICE_USE]), false, None),
        ] {
            assert_eq!(lock_of(&signed_in(Some("o"), licence.clone(), online), |_| true), lock, "{licence:?}");
        }
        assert_eq!(lock_of(&signed_in(Some("o"), active(&[OFFICE_USE]), true), |_| false), Some(EditLock::NoPlan), "allowed() has the last word");
        // Per-app ids of erpcons-account 0.4 grant nothing.
        assert_eq!(lock_of(&signed_in(Some("o"), active(&["bangtinh.use"]), true), |_| true), Some(EditLock::NoPlan));
    }

    #[test]
    fn allowed_is_asked_only_while_the_lease_lists_use() {
        let asked = std::cell::Cell::new(false);
        lock_of(&signed_in(Some("o"), LicenceState::NotActivated, true), |_| {
            asked.set(true);
            true
        });
        assert!(!asked.get());
    }

    #[test]
    fn retry_is_offered_where_a_sync_or_the_user_can_lift_the_lock() {
        assert!(EditLock::RenewFailed.sync_may_lift() && EditLock::RenewOffline.sync_may_lift() && EditLock::NoPlan.sync_may_lift());
        assert!(!EditLock::DeviceCap.sync_may_lift() && EditLock::DeviceCap.retry_offered());
        for lock in [EditLock::Revoked, EditLock::SignIn, EditLock::SessionOnly, EditLock::Starting, EditLock::Unavailable] {
            assert!(!lock.retry_offered(), "{lock:?}");
        }
    }
}
