//! The app-wide edit lock an account hook asks for (desktop: no PI licence to edit). It reuses the
//! read-only state of [`crate::read_only`] on every open document, so the ribbon greys and refused
//! edits show the reason, while opening, Save, Save As, Export and Print keep working.

use crate::SheetApp;

/// Why the account hook refuses edits now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditLock {
    /// The account has not opened yet.
    Starting,
    SignIn,
    ChooseOrg,
    Revoked,
    DeviceCap,
    /// No lease on this device yet.
    NotActivated,
    /// No OS credential store: the device key cannot be kept, so there is no lease either.
    SessionOnly,
    RenewOffline,
    RenewFailed,
    NoPlan,
    /// The account could not be opened on this computer.
    Unavailable,
}

impl EditLock {
    /// The notice, an interface key the catalogs translate.
    pub fn reason(self) -> &'static str {
        match self {
            EditLock::Starting => "Checking your license…",
            EditLock::SignIn => "Sign in with PI ID to edit",
            EditLock::ChooseOrg => "Choose an organization to edit",
            EditLock::Revoked => "This computer was removed from your PI ID account. Sign in again to edit",
            EditLock::DeviceCap => "Your plan's device limit is reached. Remove a device in PI ID, then press Retry in File › Account",
            EditLock::NotActivated => "This computer is not activated for editing yet",
            EditLock::SessionOnly => "This computer has no credential store to keep a license, so editing is off",
            EditLock::RenewOffline => "Connect to the internet to renew your license and edit",
            EditLock::RenewFailed => "Could not renew your license; the app keeps trying",
            EditLock::NoPlan => "This organization has no ERPCons Office plan",
            EditLock::Unavailable => "PI ID licensing is unavailable on this computer, so editing is off",
        }
    }

    /// A plain sync with PI may lift it: activation retried, lease renewed, a plan bought or an
    /// organization joined since the last sync.
    pub fn sync_may_lift(self) -> bool {
        matches!(self, EditLock::ChooseOrg | EditLock::NotActivated | EditLock::RenewOffline | EditLock::RenewFailed | EditLock::NoPlan)
    }

    /// File › Account offers Retry; a device-cap block lifts only on the user's retry (lease.md §6).
    pub fn retry_offered(self) -> bool {
        self.sync_may_lift() || self == EditLock::DeviceCap
    }
}

impl SheetApp {
    /// Locks or unlocks editing of every open document and of those opened later. Locking first
    /// commits what is still typed in an editor, as the host's read-only switch does; unlocking
    /// needs no reopening.
    pub fn set_edit_lock(&mut self, lock: Option<EditLock>) {
        let reason = lock.map(EditLock::reason);
        if self.session.edit_lock() == reason {
            return;
        }
        let active = self.session.active().map(|d| d.uid);
        if let (Some(uid), Some(_)) = (active, reason) {
            self.commit_held_edits(uid);
        }
        self.session.set_edit_lock(reason);
        if reason.is_none() {
            self.reopen_held_edit();
        } else if active.is_some() {
            self.stop_edit_gestures();
        }
    }

    /// Tells the account hook an edit met the edit lock, so it can sync with PI sooner.
    pub(crate) fn note_edit_refusal(&self, notice: &str) {
        if self.session.edit_lock() == Some(notice)
            && let Some(hook) = &self.account_hook
        {
            hook.edit_refused();
        }
    }
}
