//! Documents a page host opened and stores itself (web: the ERPCons office shell's `erpHost`).
//! Their Save goes to the host instead of a picker or download, and they get no web drafts;
//! without a host (desktop, a plain web page) nothing here is reached.

use std::rc::Rc;
use std::sync::Arc;

use gridcraft_engine::io::{ContentFormat, PASSWORD_REQUIRED, sniff_content};
use gridcraft_engine::model::Workbook;
use serde_json::{Value as Json, json};

use crate::{DeliveryReport, PendingSave, SheetApp};

/// The owner of the documents it opened through [`SheetApp::open_hosted`].
pub trait DocumentHost {
    /// Whether open document `uid` is one of the host's.
    fn hosts(&self, uid: u64) -> bool;
    /// Save (Ctrl+S, the Save button, Close › Save) of hosted document `uid`. `drop_macros`: the
    /// user agreed that the saved file leaves the workbook's VBA project out.
    fn save(&self, app: &mut SheetApp, uid: u64, drop_macros: bool);
}

/// Why [`SheetApp::open_hosted`] didn't open a document; the app has already told the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostOpenError {
    /// Encrypted with a password, which the online copy can't keep.
    Password,
    /// Not an XLSX package (an `.xls`, `.xlsb`, `.ods` or CSV would be converted on save).
    Unsupported,
    /// The open limit.
    TooMany,
    Unreadable,
}

/// Shown when the host hands over a password-protected workbook.
pub const HOST_PASSWORD: &str =
    "Password-protected workbooks can't be opened online yet. Remove the password in Excel and upload the workbook again.";
pub const HOST_UNSUPPORTED: &str = "This file can't be opened online. Only Excel workbooks (.xlsx) can be opened here.";
pub const HOST_UNREADABLE: &str = "This workbook couldn't be opened.";
/// The reason shown for a host's failed save that came without one.
pub const HOST_SAVE_FAILED: &str = "The document couldn't be saved to the server.";

impl SheetApp {
    pub fn is_hosted(&self, uid: u64) -> bool {
        self.document_host.as_ref().is_some_and(|h| h.hosts(uid))
    }

    /// Opens a document the host stores as `name` (an `.xlsx`), as the active workbook. Its uid,
    /// or why it was refused (shown to the user like a refused dropped file).
    pub fn open_hosted(&mut self, name: &str, bytes: &[u8]) -> Result<u64, HostOpenError> {
        let refused = match sniff_content(bytes) {
            ContentFormat::Xlsx => None,
            ContentFormat::Encrypted => Some((HostOpenError::Password, HOST_PASSWORD)),
            _ => Some((HostOpenError::Unsupported, HOST_UNSUPPORTED)),
        };
        if let Some((error, text)) = refused {
            self.message = Some((self.product_name.into(), text.into()));
            return Err(error);
        }
        // Moved into the params: `json!` would copy a large file's base64.
        let params = Json::Object(serde_json::Map::from_iter([
            ("name".to_string(), Json::String(name.to_string())),
            ("base64".to_string(), Json::String(gridcraft_engine::io::base64_encode(bytes))),
        ]));
        let opened = self.run("file.open", params);
        self.after_engine();
        match opened {
            Ok(r) => {
                self.backstage = None;
                self.show_open_warnings(&r);
                let d = self.session.active_mut().ok_or(HostOpenError::Unreadable)?;
                d.title = name.to_string();
                Ok(d.uid)
            }
            Err(e) => {
                let error = if e.contains(gridcraft_engine::TOO_MANY_DOCUMENTS) {
                    HostOpenError::TooMany
                } else if e.contains(PASSWORD_REQUIRED) {
                    HostOpenError::Password
                } else {
                    HostOpenError::Unreadable
                };
                self.alert(e);
                Err(error)
            }
        }
    }

    /// A save handed to the host, followed like a download: "Saving…", Close › Save waits for it,
    /// and once the host reports it delivered the document is saved as of `written`.
    pub fn track_host_save(&mut self, uid: u64, written: Arc<Workbook>, report: DeliveryReport) {
        let Some(name) = self.session.documents().iter().find(|d| d.uid == uid).map(|d| d.display_title()) else { return };
        self.pending_saves.push(PendingSave { uid, written, name, report, copy: false });
    }

    /// Asks before saving hosted document `uid` without its VBA project ("Save Without Macros?").
    pub fn ask_drop_macros_for_host(&mut self, uid: u64) {
        self.open_dialog(crate::save_as::DROP_MACROS, json!({"host": uid}));
    }

    /// Routes Save of a hosted active document to its host; `false` for any other document.
    pub(crate) fn save_hosted_active(&mut self) -> bool {
        let Some(uid) = self.session.active().map(|d| d.uid) else { return false };
        let Some(host) = self.document_host.clone().filter(|h| h.hosts(uid)) else { return false };
        host.save(self, uid, false);
        true
    }

    /// "Save anyway as .xlsx" in the macro question for hosted document `uid`.
    pub(crate) fn save_hosted_dropping_macros(&mut self, uid: u64) {
        if let Some(host) = self.document_host.clone() {
            host.save(self, uid, true);
        }
    }
}

/// The host as [`SheetApp::document_host`] keeps it.
pub type SharedHost = Rc<dyn DocumentHost>;

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde_json::json;

    use super::DocumentHost;
    use crate::{Delivery, DeliveryReport, Services, SheetApp};

    #[derive(Default)]
    struct FakeHost {
        hosted: RefCell<Vec<u64>>,
        saves: RefCell<Vec<(u64, bool)>>,
    }

    impl DocumentHost for FakeHost {
        fn hosts(&self, uid: u64) -> bool {
            self.hosted.borrow().contains(&uid)
        }
        fn save(&self, _: &mut SheetApp, uid: u64, drop_macros: bool) {
            self.saves.borrow_mut().push((uid, drop_macros));
        }
    }

    fn app() -> (SheetApp, Rc<RefCell<Vec<DeliveryReport>>>) {
        let reports: Rc<RefCell<Vec<DeliveryReport>>> = Rc::default();
        let started = reports.clone();
        let services = Services { download: Some(Box::new(move |_, _, report| started.borrow_mut().push(report))), ..Default::default() };
        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        let mut app = SheetApp::new(session, services);
        app.run("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
        (app, reports)
    }

    #[test]
    fn without_a_host_save_downloads_and_counts_as_saved() {
        let (mut app, reports) = app();
        let uid = app.session.active().unwrap().uid;
        assert!(!app.is_hosted(uid));
        assert_eq!(app.draft_docs().len(), 1);
        app.run("file.save", json!({})).unwrap();
        reports.borrow()[0].settle(Delivery::Delivered);
        app.settle_saves();
        assert!(!app.session.active().unwrap().is_dirty());
    }

    #[test]
    fn a_hosted_document_saves_to_its_host_and_save_as_is_a_local_copy() {
        let (mut app, reports) = app();
        let uid = app.session.active().unwrap().uid;
        let host = Rc::new(FakeHost::default());
        host.hosted.borrow_mut().push(uid);
        app.document_host = Some(host.clone());
        assert!(app.draft_docs().is_empty());
        app.run("file.save", json!({})).unwrap();
        assert_eq!(*host.saves.borrow(), [(uid, false)]);
        assert!(reports.borrow().is_empty());
        app.run("file.saveAs", json!({})).unwrap();
        reports.borrow()[0].settle(Delivery::Delivered);
        app.settle_saves();
        assert!(app.session.active().unwrap().is_dirty(), "a downloaded copy isn't the host's save");
        app.save_hosted_dropping_macros(uid);
        assert_eq!(host.saves.borrow().last(), Some(&(uid, true)));
    }

    #[test]
    fn a_closed_hosted_document_is_not_kept_for_reopening() {
        let (mut app, _) = app();
        app.session.new_workbook();
        let hosted = app.session.active().unwrap().uid;
        let host = Rc::new(FakeHost::default());
        host.hosted.borrow_mut().push(hosted);
        app.document_host = Some(host);
        let r = app.run("file.close", json!({})).unwrap();
        assert_eq!(r["closed"], true);
        assert!(!app.can_reopen_closed(), "reopening would make a local copy cut off from its host");
        app.run("file.close", json!({"force": true})).unwrap();
        assert!(app.can_reopen_closed(), "a local document is still kept");
    }
}
