//! The documents the shell opened through `erpHost` and their saves: what the shell last
//! acknowledged, one save in flight per document with later triggers coalesced, AutoSave, and
//! `flush`. Saving needs no `SheetApp` (a hidden page runs no frames), so each frame leaves a
//! snapshot of every document here, and saves started without the app are handed to it on the
//! next frame.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use gridcraft_engine::doc_access::DocAccess;
use gridcraft_engine::model::Workbook;
use gridcraft_ui_egui::host_docs::{self, DocumentHost, HostOpenError};
use gridcraft_ui_egui::reload_view::ViewSnapshot;
use gridcraft_ui_egui::{Delivery, DeliveryReport, SheetApp};

use super::autosave::Timing;
use super::protocol::{FORMAT, OpenRequest, Reason, Request, SaveResult, Shell, shell_shares};

/// Why AutoSave and `flush` leave a document alone until a manual save succeeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    /// Rule 4: the shell resolves a conflict; the fork never retries one itself.
    Conflict,
    /// Rule 6: the same bytes would fail again.
    Failed,
}

struct InFlight {
    seq: u64,
    revision: u64,
    written: Arc<Workbook>,
    report: DeliveryReport,
}

/// Saves the shell kept to retry itself, by revision, for a later `acknowledged`; only the newest
/// few matter, as the shell keeps the last bytes of each document.
const UNACKED_KEPT: usize = 4;

struct HostDoc {
    doc_id: String,
    version: Option<u64>,
    format: String,
    /// The workbook as the shell last acknowledged it (rule 1), and its revision.
    acked: Arc<Workbook>,
    acked_revision: u64,
    /// Sent but answered `offline` or retryable `failed`: (revision, workbook as sent).
    unacked: Vec<(u64, Arc<Workbook>)>,
    latest: Arc<Workbook>,
    revision: u64,
    /// The app's unsaved-changes state at the snapshot.
    dirty: bool,
    drops_macros: bool,
    /// The user answered "Save anyway as .xlsx" for this document.
    may_drop_macros: bool,
    in_flight: Option<InFlight>,
    /// A save asked for while one was in flight: one follow-up of the newest revision (rule 2).
    queued: Option<Reason>,
    stopped: Option<Stop>,
    timing: Timing,
    seen_revision: u64,
    reported_dirty: bool,
    /// v2 `mode` / `canDownload`. A read-only document is never auto-saved (rule 13).
    access: DocAccess,
}

impl HostDoc {
    fn needs_save(&self) -> bool {
        self.dirty && !Arc::ptr_eq(&self.latest, &self.acked) && self.in_flight.as_ref().is_none_or(|f| !Arc::ptr_eq(&f.written, &self.latest))
    }

    /// Saving now would leave the VBA project out without the user having agreed.
    fn held_for_macros(&self) -> bool {
        self.drops_macros && !self.may_drop_macros
    }

    /// AutoSave and `flush` leave it alone.
    fn idle(&self) -> bool {
        self.stopped.is_some() || self.held_for_macros()
    }

    /// Rule 13: only `flush` still saves a document downgraded with edits in hand.
    fn auto_saves(&self) -> bool {
        !self.idle() && !self.access.read_only
    }

    /// The shell stored `written` as `revision` (rule 1); an older revision never overrides a newer one.
    fn acknowledge(&mut self, revision: u64, written: Arc<Workbook>, version: Option<u64>) -> bool {
        if revision < self.acked_revision {
            return false;
        }
        self.acked = written;
        self.acked_revision = revision;
        self.version = version.or(self.version);
        self.stopped = None;
        self.unacked.retain(|(r, _)| *r > revision);
        true
    }

    fn refresh(&mut self, app: &SheetApp, uid: u64) {
        if let Some(d) = app.session.documents().iter().find(|d| d.uid == uid) {
            self.latest = d.wb.clone();
            self.revision = d.revision;
            self.dirty = d.is_dirty();
            self.drops_macros = gridcraft_engine::io::drops_macros(&d.wb, &format!("x.{}", self.format));
        }
    }
}

type Late = Rc<RefCell<Vec<(u64, u64, SaveResult)>>>;

/// `flush()` calls that arrived while `HostDocs` was borrowed.
type HeldFlushes = Rc<RefCell<Vec<Box<dyn FnOnce()>>>>;

pub(crate) struct HostDocs {
    shell: Rc<dyn Shell>,
    docs: BTreeMap<u64, HostDoc>,
    /// Read by [`Hook::hosts`] while `HostDocs` may be borrowed.
    hosted: Rc<RefCell<BTreeSet<u64>>>,
    next_seq: u64,
    /// Saves started without the app, for [`SheetApp::track_host_save`] on the next frame.
    untracked: Vec<(u64, Arc<Workbook>, DeliveryReport)>,
    warnings: Vec<(u64, Vec<String>)>,
    /// Acknowledged by the shell's own retry, for [`gridcraft_engine::Session::mark_saved`] on the next frame.
    acknowledgements: Vec<(u64, Arc<Workbook>)>,
    flush_waiters: Vec<Box<dyn FnOnce()>>,
    /// Results that arrived while `HostDocs` was borrowed.
    late: Late,
    held_flushes: HeldFlushes,
    me: Weak<RefCell<HostDocs>>,
}

/// The shell's `flush()`. The shell may call it from inside one of this side's calls to it (its `dirty` handler
/// does, right after a downgrade's commit) while `HostDocs` is borrowed; it then runs before that frame ends.
pub(crate) struct FlushEntry {
    docs: Weak<RefCell<HostDocs>>,
    held: HeldFlushes,
}

impl FlushEntry {
    /// Returns `false` when the flush was held for the end of the current or next frame.
    pub(crate) fn flush(&self, done: Box<dyn FnOnce()>) -> bool {
        let Some(docs) = self.docs.upgrade() else {
            done();
            return true;
        };
        match docs.try_borrow_mut() {
            Ok(mut docs) => {
                docs.flush(done);
                true
            }
            Err(_) => {
                self.held.borrow_mut().push(done);
                false
            }
        }
    }
}

/// Where the shell's answer to one `save` goes.
pub(crate) struct Reply {
    docs: Weak<RefCell<HostDocs>>,
    late: Late,
    uid: u64,
    seq: u64,
}

impl Reply {
    pub(crate) fn resolve(self, result: SaveResult) {
        let Some(docs) = self.docs.upgrade() else { return };
        match docs.try_borrow_mut() {
            Ok(mut docs) => docs.settle(self.uid, self.seq, result),
            Err(_) => self.late.borrow_mut().push((self.uid, self.seq, result)),
        }
    }
}

/// [`SheetApp::document_host`] for these documents.
struct Hook {
    docs: Weak<RefCell<HostDocs>>,
    hosted: Rc<RefCell<BTreeSet<u64>>>,
    shares: bool,
}

impl DocumentHost for Hook {
    fn hosts(&self, uid: u64) -> bool {
        self.hosted.try_borrow().is_ok_and(|set| set.contains(&uid))
    }

    fn save(&self, app: &mut SheetApp, uid: u64, drop_macros: bool) {
        let Some(docs) = self.docs.upgrade() else { return };
        match docs.try_borrow_mut() {
            Ok(mut docs) => docs.manual_save(app, uid, drop_macros),
            Err(e) => log::warn!("erpHost: save not started: {e}"),
        }
    }

    fn shares(&self) -> bool {
        self.shares
    }
}

/// The name a host document opens under: its title without any folder, as an `.xlsx`.
fn document_name(title: &str) -> String {
    let base = title.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let base = if base.is_empty() { "Book" } else { base };
    if base.to_ascii_lowercase().ends_with(".xlsx") { base.to_string() } else { format!("{base}.xlsx") }
}

/// `opened` reasons are keys of the shell's intake messages: it shows the sentence.
pub(crate) const UNSUPPORTED: &str = "unsupported";
pub(crate) const PARSER_FAILED: &str = "parserFailed";

impl HostDocs {
    pub(crate) fn new(shell: Rc<dyn Shell>) -> Rc<RefCell<HostDocs>> {
        Rc::new_cyclic(|me| {
            RefCell::new(HostDocs {
                shell,
                docs: BTreeMap::new(),
                hosted: Rc::default(),
                next_seq: 1,
                untracked: Vec::new(),
                warnings: Vec::new(),
                acknowledgements: Vec::new(),
                flush_waiters: Vec::new(),
                late: Rc::default(),
                held_flushes: Rc::default(),
                me: me.clone(),
            })
        })
    }

    pub(crate) fn flush_entry(this: &Rc<RefCell<HostDocs>>) -> FlushEntry {
        FlushEntry { docs: Rc::downgrade(this), held: this.borrow().held_flushes.clone() }
    }

    /// Installs these documents as `app`'s document host.
    pub(crate) fn connect(this: &Rc<RefCell<HostDocs>>, app: &mut SheetApp, version: u32) {
        let hosted = this.borrow().hosted.clone();
        app.document_host = Some(Rc::new(Hook { docs: Rc::downgrade(this), hosted, shares: shell_shares(version) }));
    }

    /// `open(doc)`: opens it as a new tab, replacing the one with the same `docId` (the shell
    /// reloads the latest version after a conflict), and reports `opened` once.
    pub(crate) fn open(&mut self, app: &mut SheetApp, request: OpenRequest) {
        let OpenRequest { doc_id, version, title, format, bytes, access, preserve_view } = request;
        if doc_id.is_empty() {
            log::warn!("erpHost.open without a docId");
            return;
        }
        let refused = if !format.eq_ignore_ascii_case(FORMAT) {
            Some((host_docs::HOST_UNSUPPORTED, UNSUPPORTED))
        } else if bytes.is_none() {
            Some((host_docs::HOST_UNREADABLE, PARSER_FAILED))
        } else {
            None
        };
        let (Some(bytes), None) = (bytes, refused) else {
            let (text, reason) = refused.unwrap_or((host_docs::HOST_UNREADABLE, PARSER_FAILED));
            app.message = Some((app.product_name.into(), text.into()));
            self.shell.opened(&doc_id, false, Some(reason));
            return;
        };
        let previous = self.docs.iter().find(|(_, d)| d.doc_id == doc_id).map(|(uid, _)| *uid);
        let kept = previous.filter(|_| preserve_view).map(|old| KeptView::of(app, old));
        // At the open limit the old copy goes first, or the latest version couldn't open.
        if let Some(old) = previous
            && app.session.documents().len() >= gridcraft_engine::MAX_DOCUMENTS
        {
            close_now(app, old);
        }
        match app.open_hosted(&document_name(&title), &bytes) {
            Ok(uid) => {
                let replaced = previous.and_then(|old| self.forget(old));
                if let Some(old) = previous {
                    close_now(app, old);
                }
                if let Some(old) = &replaced {
                    log::info!("erpHost: reloaded a document, version {:?} -> {version:?}", old.version);
                }
                if let Some(kept) = kept {
                    kept.restore(app, uid);
                }
                app.set_document_access(uid, access);
                let Some(d) = app.session.documents().iter().find(|d| d.uid == uid) else { return };
                let mut doc = HostDoc {
                    doc_id: doc_id.clone(),
                    version,
                    format: FORMAT.into(),
                    acked: d.saved.clone(),
                    acked_revision: d.revision,
                    unacked: Vec::new(),
                    latest: d.wb.clone(),
                    revision: d.revision,
                    dirty: false,
                    drops_macros: false,
                    may_drop_macros: false,
                    in_flight: None,
                    queued: None,
                    stopped: None,
                    timing: Timing::default(),
                    seen_revision: d.revision,
                    reported_dirty: replaced.is_some_and(|old| old.reported_dirty),
                    access,
                };
                doc.refresh(app, uid);
                self.docs.insert(uid, doc);
                self.hosted.borrow_mut().insert(uid);
                self.shell.opened(&doc_id, true, None);
            }
            Err(error) => {
                let reason = match error {
                    HostOpenError::Password | HostOpenError::Unsupported => UNSUPPORTED,
                    HostOpenError::TooMany | HostOpenError::Unreadable => PARSER_FAILED,
                };
                self.shell.opened(&doc_id, false, Some(reason));
            }
        }
    }

    /// Applies one of the shell's calls that needs the app.
    pub(crate) fn apply(&mut self, app: &mut SheetApp, request: Request) {
        match request {
            Request::Open(request) => self.open(app, request),
            Request::Mode(doc_id, access) => self.set_mode(app, &doc_id, access),
            Request::User(name) => app.session.author = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
        }
    }

    /// `setMode(docId, mode, canDownload)`: the same document and its undo history, new access.
    /// A downgrade commits the open editor first; the shell's next `flush` saves what it held.
    pub(crate) fn set_mode(&mut self, app: &mut SheetApp, doc_id: &str, access: DocAccess) {
        let Some((&uid, doc)) = self.docs.iter_mut().find(|(_, d)| d.doc_id == doc_id) else { return };
        doc.access = access;
        app.set_document_access(uid, access);
        doc.refresh(app, uid);
    }

    fn forget(&mut self, uid: u64) -> Option<HostDoc> {
        self.hosted.borrow_mut().remove(&uid);
        self.docs.remove(&uid)
    }

    /// Ctrl+S / "Lưu" (rule 9); also allowed after a conflict or failure, since the user asked.
    fn manual_save(&mut self, app: &mut SheetApp, uid: u64, drop_macros: bool) {
        if app.session.active().is_some_and(|d| d.uid == uid) {
            app.store_view_state();
        }
        let Some(doc) = self.docs.get_mut(&uid).filter(|d| !d.access.read_only) else { return };
        doc.may_drop_macros |= drop_macros;
        doc.refresh(app, uid);
        if doc.held_for_macros() {
            app.ask_drop_macros_for_host(uid);
            return;
        }
        self.request(uid, Reason::Manual, Some(app));
    }

    /// Starts a save of `uid`'s newest revision, or queues it behind the one in flight.
    fn request(&mut self, uid: u64, reason: Reason, app: Option<&mut SheetApp>) {
        let Some(doc) = self.docs.get_mut(&uid) else { return };
        if doc.in_flight.is_some() {
            if doc.needs_save() {
                doc.queued = doc.queued.max(Some(reason));
            }
            doc.timing.started();
            return;
        }
        if !doc.needs_save() {
            return;
        }
        let Some((written, report)) = self.send(uid, reason) else { return };
        match app {
            Some(app) => app.track_host_save(uid, written, report),
            None => self.untracked.push((uid, written, report)),
        }
    }

    fn send(&mut self, uid: u64, reason: Reason) -> Option<(Arc<Workbook>, DeliveryReport)> {
        let seq = self.next_seq;
        let doc = self.docs.get_mut(&uid)?;
        let written = doc.latest.clone();
        let output = match gridcraft_engine::io::save_with_warnings(&written, &format!("x.{}", doc.format)) {
            Ok(output) => output,
            Err(e) => {
                log::error!("erpHost: couldn't encode a document: {e}");
                return None;
            }
        };
        self.next_seq += 1;
        let report = DeliveryReport::default();
        doc.in_flight = Some(InFlight { seq, revision: doc.revision, written: written.clone(), report: report.clone() });
        doc.timing.started();
        if !output.warnings.is_empty() {
            self.warnings.push((uid, output.warnings));
        }
        let reply = Reply { docs: self.me.clone(), late: self.late.clone(), uid, seq };
        self.shell.save(&doc.doc_id, output.bytes, doc.revision, reason, reply);
        Some((written, report))
    }

    fn settle(&mut self, uid: u64, seq: u64, result: SaveResult) {
        let Some(doc) = self.docs.get_mut(&uid) else { return };
        if doc.in_flight.as_ref().is_none_or(|f| f.seq != seq) {
            return;
        }
        let Some(flight) = doc.in_flight.take() else { return };
        let delivery = match result {
            SaveResult::Saved { version } => {
                doc.acknowledge(flight.revision, flight.written.clone(), version);
                Delivery::Delivered
            }
            SaveResult::Conflict => {
                doc.stopped = Some(Stop::Conflict);
                doc.queued = None;
                Delivery::Cancelled
            }
            SaveResult::Offline | SaveResult::Failed { retryable: true, .. } => {
                doc.unacked.push((flight.revision, flight.written.clone()));
                if doc.unacked.len() > UNACKED_KEPT {
                    doc.unacked.remove(0);
                }
                Delivery::Cancelled
            }
            SaveResult::Failed { retryable: false, message } => {
                doc.stopped = Some(Stop::Failed);
                doc.queued = None;
                Delivery::Failed(if message.trim().is_empty() { host_docs::HOST_SAVE_FAILED.into() } else { message })
            }
        };
        flight.report.settle(delivery);
        if let Some(reason) = doc.queued.take() {
            self.request(uid, reason, None);
        }
        self.wake_flush();
    }

    /// `flush()`: saves every document with unacknowledged edits; `done` runs once each of them
    /// has been handed to the shell.
    pub(crate) fn flush(&mut self, done: Box<dyn FnOnce()>) {
        let uids: Vec<u64> = self.docs.iter().filter(|(_, d)| !d.idle()).map(|(uid, _)| *uid).collect();
        for uid in uids {
            self.request(uid, Reason::Flush, None);
        }
        self.flush_waiters.push(done);
        self.wake_flush();
    }

    /// `acknowledged()`: the shell's own retry stored a save this side was told was `offline` or a
    /// retryable `failed`. Unknown documents, revisions never sent and stale revisions are ignored.
    pub(crate) fn acknowledged(&mut self, doc_id: &str, revision: u64, version: Option<u64>) {
        let Some((&uid, doc)) = self.docs.iter_mut().find(|(_, d)| d.doc_id == doc_id) else { return };
        let Some(written) = doc.unacked.iter().find(|(r, _)| *r == revision).map(|(_, wb)| wb.clone()) else { return };
        if doc.acknowledge(revision, written.clone(), version) {
            self.acknowledgements.push((uid, written));
        }
    }

    fn wake_flush(&mut self) {
        if self.docs.values().all(|d| d.queued.is_none()) {
            for done in std::mem::take(&mut self.flush_waiters) {
                done();
            }
        }
    }

    /// Call at the end of every frame. Returns when (ms from `now`) an AutoSave falls due.
    pub(crate) fn after_frame(&mut self, app: &mut SheetApp, now: f64) -> Option<f64> {
        let late = std::mem::take(&mut *self.late.borrow_mut());
        for (uid, seq, result) in late {
            self.settle(uid, seq, result);
        }
        for (uid, written, report) in std::mem::take(&mut self.untracked) {
            app.track_host_save(uid, written, report);
        }
        for (uid, warnings) in std::mem::take(&mut self.warnings) {
            app.show_draft_warnings(uid, &warnings);
        }
        for (uid, written) in std::mem::take(&mut self.acknowledgements) {
            // A newer `saved` settled since: marking this older snapshot would undo it.
            if self.docs.get(&uid).is_some_and(|d| Arc::ptr_eq(&d.acked, &written)) {
                app.session.mark_saved(uid, &written);
            }
        }
        let open: BTreeSet<u64> = app.session.documents().iter().map(|d| d.uid).collect();
        let closed: Vec<u64> = self.docs.keys().filter(|uid| !open.contains(uid)).copied().collect();
        for uid in closed {
            if let Some(doc) = self.forget(uid) {
                self.shell.closed(&doc.doc_id);
            }
        }
        let editing = app.editing_in_place().then(|| app.session.active().map(|d| d.uid)).flatten();
        let mut due = Vec::new();
        for (uid, doc) in &mut self.docs {
            doc.refresh(app, *uid);
            if doc.revision != doc.seen_revision {
                doc.seen_revision = doc.revision;
                doc.timing.edited(now);
            }
            // A read-only document can only turn clean (its downgrade edits saved by `flush`).
            if doc.dirty != doc.reported_dirty {
                doc.reported_dirty = doc.dirty;
                self.shell.dirty(&doc.doc_id, doc.dirty);
            }
            if !doc.auto_saves() || editing == Some(*uid) || !doc.timing.due(now) {
                continue;
            }
            if doc.needs_save() || doc.in_flight.is_some() {
                due.push(*uid);
            } else {
                // Undone back to the acknowledged state: nothing to save.
                doc.timing.started();
            }
        }
        let held = std::mem::take(&mut *self.held_flushes.borrow_mut());
        for done in held {
            self.flush(done);
        }
        for uid in due {
            if app.session.active().is_some_and(|d| d.uid == uid) {
                app.store_view_state();
                if let Some(doc) = self.docs.get_mut(&uid) {
                    doc.refresh(app, uid);
                }
            }
            self.request(uid, Reason::Auto, Some(app));
        }
        // The cell being edited repaints on its own input; waking for it would spin.
        self.docs.iter().filter(|(uid, d)| d.auto_saves() && editing != Some(**uid)).filter_map(|(_, d)| d.timing.due_in(now)).reduce(f64::min)
    }

    #[cfg(test)]
    pub(crate) fn stopped(&self, uid: u64) -> Option<Stop> {
        self.docs.get(&uid).and_then(|d| d.stopped)
    }

    #[cfg(test)]
    pub(crate) fn version(&self, uid: u64) -> Option<u64> {
        self.docs.get(&uid).and_then(|d| d.version)
    }
}

/// Where a reloaded document was and what it showed (`preserveView`).
struct KeptView {
    index: Option<usize>,
    /// The tab the user was on, when not the reloaded document.
    active: Option<u64>,
    view: Option<ViewSnapshot>,
}

impl KeptView {
    fn of(app: &SheetApp, old: u64) -> KeptView {
        let active = app.session.active().map(|d| d.uid).filter(|uid| *uid != old);
        KeptView { index: app.document_index(Some(old)), active, view: app.view_snapshot(old) }
    }

    /// Puts the newer version `uid` in the old one's tab, showing what it showed.
    fn restore(self, app: &mut SheetApp, uid: u64) {
        if let (Some(index), Some(now)) = (self.index, app.document_index(Some(uid))) {
            app.session.move_document(now, index.min(app.session.documents().len().saturating_sub(1)));
        }
        if let Some(i) = self.active.and_then(|other| app.document_index(Some(other))) {
            app.session.set_active(i);
        }
        if let Some(view) = &self.view {
            app.restore_view(uid, view);
        }
    }
}

/// Closes open document `uid` without asking: the shell replaced it with another version.
fn close_now(app: &mut SheetApp, uid: u64) {
    if let Some(i) = app.document_index(Some(uid)) {
        app.session.close_document(i);
    }
}
