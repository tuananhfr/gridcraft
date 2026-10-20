//! Deferred close requests retain document identities while browser writes are pending.

use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::close_many::AfterClose;

#[derive(Debug)]
pub(crate) struct PendingClose {
    targets: Vec<u64>,
    then: AfterClose,
}

pub(crate) fn closed_last_document(closed: bool, remaining: usize) -> bool {
    closed && remaining == 0
}

impl SheetApp {
    pub(crate) fn defer_close(&mut self, targets: Vec<u64>, then: AfterClose) -> bool {
        if !self.pending_saves.iter().any(|save| targets.contains(&save.uid)) {
            return false;
        }
        if !self.pending_closes.iter().any(|close| close.targets == targets && close.then == then) {
            self.pending_closes.push(PendingClose { targets, then });
        }
        true
    }

    pub(crate) fn defer_close_command(&mut self, params: &Json) -> bool {
        let index = match params.get("index") {
            Some(value) => match value.as_u64().and_then(|i| usize::try_from(i).ok()) {
                Some(index) => index,
                None => return false,
            },
            None => self.session.active_index(),
        };
        let Some(uid) = self.session.documents().get(index).map(|d| d.uid) else { return false };
        self.defer_close(vec![uid], AfterClose::Close)
    }

    pub(crate) fn cancel_close_after_save(&mut self, uid: u64) {
        self.pending_closes.retain(|close| !close.targets.contains(&uid));
    }

    pub(crate) fn advance_pending_closes(&mut self) {
        let requests = std::mem::take(&mut self.pending_closes);
        for request in requests {
            if self.defer_close(request.targets.clone(), request.then) {
                continue;
            }
            if request.targets.len() == 1 && request.then == AfterClose::Close {
                if let Some(index) = request.targets.first().and_then(|uid| self.document_index(Some(*uid))) {
                    self.close_document_at(index);
                }
            } else {
                self.close_documents(request.targets, request.then);
            }
        }
    }

    pub(crate) fn save_before_close(&mut self, target: Option<u64>) {
        self.in_document(target, |app| {
            if let Some(uid) = app.session.active().map(|d| d.uid)
                && app.defer_close(vec![uid], AfterClose::Close)
            {
                return;
            }
            app.run_or_alert("file.save", json!({}));
            if let Some(uid) = app.session.active().map(|d| d.uid) {
                if app.defer_close(vec![uid], AfterClose::Close) {
                    return;
                }
                if app.session.active().is_some_and(|d| !d.is_dirty()) {
                    app.run_or_alert("file.close", json!({"force": true}));
                }
            }
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use gridcraft_engine::Session;
    use serde_json::json;

    use super::closed_last_document;
    use crate::close_many::AfterClose;
    use crate::{Delivery, DeliveryReport, Services, SheetApp};

    fn app() -> (SheetApp, Arc<Mutex<Vec<DeliveryReport>>>, Arc<AtomicUsize>) {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let writes = reports.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let closed = calls.clone();
        let services = Services {
            download: Some(Box::new(move |_, _, report| writes.lock().unwrap().push(report))),
            editor_closed: Some(Box::new(move || {
                closed.fetch_add(1, Ordering::SeqCst);
            })),
            ..Default::default()
        };
        let mut session = Session::new();
        session.new_workbook();
        let mut app = SheetApp::new(session, services);
        app.run("cell.set", json!({"cell": "A1", "input": "draft"})).unwrap();
        (app, reports, calls)
    }

    fn advance(app: &mut SheetApp) {
        app.settle_saves();
        app.advance_pending_closes();
    }

    #[test]
    fn save_on_close_waits_for_successful_write() {
        let (mut app, reports, calls) = app();
        let uid = app.session.active().unwrap().uid;
        app.save_before_close(Some(uid));
        advance(&mut app);
        assert!(app.document_index(Some(uid)).is_some());
        assert!(app.session.active().unwrap().is_dirty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        reports.lock().unwrap()[0].settle(Delivery::Delivered);
        advance(&mut app);
        assert!(app.document_index(Some(uid)).is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        advance(&mut app);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancelled_or_failed_write_keeps_close_target_open_and_dirty() {
        for outcome in [Delivery::Cancelled, Delivery::Failed("The browser could not save the file. Please try again.".into())] {
            let (mut app, reports, calls) = app();
            let uid = app.session.active().unwrap().uid;
            app.save_before_close(Some(uid));
            reports.lock().unwrap()[0].settle(outcome);
            advance(&mut app);
            assert!(app.document_index(Some(uid)).is_some());
            assert!(app.session.active().unwrap().is_dirty());
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            assert!(app.pending_closes.is_empty());
            if let Some((_, message)) = &app.message {
                let translated = crate::i18n::Language::Vi.tr_message(message);
                assert!(translated.contains("Trình duyệt không thể lưu tệp"));
            }
        }
    }

    #[test]
    fn close_during_save_waits_without_starting_another_write() {
        let (mut app, reports, _) = app();
        let uid = app.session.active().unwrap().uid;
        app.run("file.save", json!({})).unwrap();
        let result = app.run("file.close", json!({})).unwrap();
        assert_eq!(result["waiting"], true);
        assert_eq!(reports.lock().unwrap().len(), 1);
        advance(&mut app);
        assert!(app.document_index(Some(uid)).is_some());
        reports.lock().unwrap()[0].settle(Delivery::Delivered);
        advance(&mut app);
        assert!(app.document_index(Some(uid)).is_none());
    }

    #[test]
    fn close_all_during_save_waits_and_cancels_atomically() {
        for success in [false, true] {
            let (mut app, reports, calls) = app();
            let uid = app.session.active().unwrap().uid;
            app.run("file.save", json!({})).unwrap();
            app.run("file.new", json!({})).unwrap();
            let other = app.session.active().unwrap().uid;
            app.close_documents(vec![uid, other], AfterClose::Close);
            advance(&mut app);
            assert_eq!(app.session.documents().len(), 2);
            reports.lock().unwrap()[0].settle(if success { Delivery::Delivered } else { Delivery::Cancelled });
            advance(&mut app);
            assert_eq!(app.document_index(Some(uid)).is_none(), success);
            assert_eq!(app.document_index(Some(other)).is_none(), success);
            assert_eq!(calls.load(Ordering::SeqCst), usize::from(success));
        }
    }

    #[test]
    fn edits_during_write_still_require_confirmation() {
        let (mut app, reports, calls) = app();
        let uid = app.session.active().unwrap().uid;
        app.save_before_close(Some(uid));
        app.run("cell.set", json!({"cell": "A1", "input": "newer draft"})).unwrap();
        reports.lock().unwrap()[0].settle(Delivery::Delivered);
        advance(&mut app);
        assert!(app.document_index(Some(uid)).is_some());
        assert!(app.session.active().unwrap().is_dirty());
        assert_eq!(app.dialog.as_ref().unwrap().name, "saveChanges");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn closed_hook_fires_once_per_last_document_close() {
        let (mut app, _, calls) = app();
        app.run("file.new", json!({})).unwrap();
        app.run("file.close", json!({"force": true})).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        app.run("file.close", json!({"force": true})).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        app.session.new_workbook();
        app.run("file.close", json!({"force": true})).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn closed_hook_requires_a_successful_last_close() {
        assert!(closed_last_document(true, 0));
        assert!(!closed_last_document(true, 1));
        assert!(!closed_last_document(false, 0));
        assert!(!closed_last_document(false, 1));
    }
}
