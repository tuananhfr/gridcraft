//! Closing several documents at once (the tab strip's Close All / Close Others, quitting the
//! desktop app): one dialog lists every unsaved one with Save All / Don't Save / Cancel. Save All
//! saves them one after another, each finishing before the next starts, and closes them once all
//! are saved. A browser save picker needs a click of its own, so after one has written its file
//! the dialog stays up and the next click saves the next document. A cancel or failure stops
//! there and closes nothing.

use std::collections::VecDeque;

use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::Dialog;
use crate::widgets::tr;

/// What follows once the documents are dealt with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterClose {
    /// Close them.
    Close,
    /// Quit the app (desktop), leaving them open meanwhile.
    Quit,
    #[cfg(not(target_arch = "wasm32"))]
    Update,
}

impl AfterClose {
    fn id(self) -> &'static str {
        match self {
            AfterClose::Close => "close",
            AfterClose::Quit => "quit",
            #[cfg(not(target_arch = "wasm32"))]
            AfterClose::Update => "update",
        }
    }

    fn parse(id: &str) -> AfterClose {
        #[cfg(not(target_arch = "wasm32"))]
        if id == "update" {
            return AfterClose::Update;
        }
        if id == "quit" { AfterClose::Quit } else { AfterClose::Close }
    }
}

/// Save All in progress.
#[derive(Debug)]
pub struct CloseQueue {
    targets: Vec<u64>,
    to_save: VecDeque<u64>,
    saved: Vec<u64>,
    /// The document whose save the host hasn't answered yet (the browser's save picker).
    waiting: Option<u64>,
    then: AfterClose,
}

impl SheetApp {
    /// Closes documents `uids` (or quits), asking once about all the unsaved ones among them.
    pub fn close_documents(&mut self, uids: Vec<u64>, then: AfterClose) {
        if !self.commit_edit(0, 0, false, false) {
            return;
        }
        crate::paper_view::commit(self);
        if self.defer_close(uids.clone(), then) {
            return;
        }
        let unsaved: Vec<u64> = uids.iter().copied().filter(|&uid| self.is_unsaved(uid)).collect();
        if unsaved.is_empty() {
            return self.finish_close(&uids, then);
        }
        self.open_dialog("closeMany", json!({"uids": uids, "unsaved": unsaved, "then": then.id()}));
    }

    /// Every open document except `keep`.
    pub fn close_others(&mut self, keep: u64) {
        let uids = self.session.documents().iter().map(|d| d.uid).filter(|&uid| uid != keep).collect();
        self.close_documents(uids, AfterClose::Close);
    }

    pub fn close_all(&mut self) {
        let uids = self.session.documents().iter().map(|d| d.uid).collect();
        self.close_documents(uids, AfterClose::Close);
    }

    /// Desktop: the window is asked to close. `true` when it may close now; otherwise the
    /// unsaved documents are asked about first and the window closes once they are dealt with.
    pub fn confirm_quit(&mut self) -> bool {
        if self.quit_confirmed {
            return true;
        }
        if self.close_queue.is_none() {
            let uids = self.session.documents().iter().map(|d| d.uid).collect();
            self.close_documents(uids, AfterClose::Quit);
        }
        self.quit_confirmed
    }

    /// A Save All is in progress.
    pub fn saving_all(&self) -> bool {
        self.close_queue.is_some()
    }

    fn is_unsaved(&self, uid: u64) -> bool {
        self.session.documents().iter().any(|d| d.uid == uid && d.is_dirty())
    }

    fn close_by_uid(&mut self, uid: u64, force: bool) {
        if let Some(i) = self.document_index(Some(uid)) {
            self.run_or_alert("file.close", json!({"index": i, "force": force}));
        }
    }

    fn finish_close(&mut self, uids: &[u64], then: AfterClose) {
        match then {
            #[cfg(not(target_arch = "wasm32"))]
            AfterClose::Update => {
                self.updates.awaiting_save = false;
                self.updates.continue_requested = true;
            }
            AfterClose::Close => {
                for &uid in uids {
                    self.close_by_uid(uid, true);
                }
                self.note_active_document();
            }
            AfterClose::Quit => {
                self.quit_confirmed = true;
                self.quit_requested = true;
            }
        }
    }

    fn save_all(&mut self, targets: Vec<u64>, unsaved: Vec<u64>, then: AfterClose) {
        self.close_queue = Some(CloseQueue { targets, to_save: unsaved.into(), saved: Vec::new(), waiting: None, then });
        self.run_close_queue(true);
    }

    /// Per frame: carries Save All on once the host has answered a save picker. The dialog
    /// closed meanwhile (Esc, its close button) is a Cancel.
    pub(crate) fn advance_close_queue(&mut self) {
        if self.close_queue.is_none() {
            return;
        }
        if !self.dialog.as_ref().is_some_and(|d| d.name == "closeMany") {
            self.close_queue = None;
            return;
        }
        self.run_close_queue(false);
    }

    /// Saves what is left one document at a time. `clicked`: the user just asked for it, so a
    /// save may open a browser save picker.
    fn run_close_queue(&mut self, clicked: bool) {
        let Some(mut q) = self.close_queue.take() else { return };
        let mut may_start = clicked;
        loop {
            if let Some(uid) = q.waiting {
                if self.pending_saves.iter().any(|p| p.uid == uid) {
                    self.close_queue = Some(q);
                    return;
                }
                q.waiting = None;
                // Cancelled or failed: stop, closing nothing.
                if self.is_unsaved(uid) {
                    return self.end_close_dialog();
                }
                q.saved.push(uid);
                // Browsers refuse a save picker the user didn't click for: the next one waits.
                may_start = false;
                continue;
            }
            let Some(&uid) = q.to_save.front() else {
                // What is left is saved; anything edited meanwhile still asks.
                let rest: Vec<u64> = q.targets.iter().copied().filter(|&uid| self.document_index(Some(uid)).is_some()).collect();
                self.end_close_dialog();
                if q.then == AfterClose::Close {
                    for uid in rest {
                        self.close_by_uid(uid, false);
                    }
                    self.note_active_document();
                } else {
                    self.finish_close(&rest, q.then);
                }
                return;
            };
            if !self.is_unsaved(uid) {
                q.to_save.pop_front();
                q.saved.push(uid);
                continue;
            }
            if !may_start {
                self.close_queue = Some(q);
                return;
            }
            q.to_save.pop_front();
            self.in_document(Some(uid), |app| app.run_or_alert("file.save", json!({})));
            // A plain browser download reports at once: those all go in one click.
            self.settle_saves();
            if self.pending_saves.iter().any(|p| p.uid == uid) {
                q.waiting = Some(uid);
                continue;
            }
            if self.is_unsaved(uid) {
                return self.end_close_dialog();
            }
            q.saved.push(uid);
        }
    }

    /// Save All is over: the dialog goes, unless its body is showing it (it closes itself then).
    fn end_close_dialog(&mut self) {
        self.close_queue = None;
        if self.dialog.as_ref().is_some_and(|d| d.name == "closeMany") {
            self.dialog = None;
        }
    }
}

/// The "closeMany" dialog's body. During Save All on the web it stays up: each row says how far
/// its document has got and the main button names the document its next click saves.
pub(crate) fn dialog_body(app: &mut SheetApp, ui: &mut egui::Ui, d: &Dialog, open: &mut bool) {
    let uids = |key: &str| -> Vec<u64> {
        d.values.get(key).and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_u64).collect()).unwrap_or_default()
    };
    let targets = uids("uids");
    let unsaved: Vec<u64> = uids("unsaved").into_iter().filter(|&uid| app.document_index(Some(uid)).is_some()).collect();
    let then = AfterClose::parse(d.values.get("then").and_then(Json::as_str).unwrap_or(""));
    let (saved, saving) = app.close_queue.as_ref().map(|q| (q.saved.clone(), q.waiting)).unwrap_or_default();
    let rows: Vec<(u64, String)> =
        unsaved.iter().filter_map(|&uid| app.session.documents().iter().find(|doc| doc.uid == uid).map(|doc| (uid, doc.display_title()))).collect();
    ui.label(tr(ui, "Do you want to save the changes you made to these {count} documents?").replace("{count}", &rows.len().to_string()));
    ui.add_space(4.0);
    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
        for (uid, title) in &rows {
            let row = if saved.contains(uid) {
                tr(ui, "{name} — Saved").replace("{name}", title)
            } else if saving == Some(*uid) {
                tr(ui, "{name} — Saving…").replace("{name}", title)
            } else {
                title.clone()
            };
            ui.label(format!("• {row}"));
        }
    });
    ui.add_space(8.0);
    let next = rows.iter().find(|(uid, _)| !saved.contains(uid) && saving != Some(*uid)).map(|(_, title)| title);
    let save_label = match next {
        Some(title) if !saved.is_empty() || saving.is_some() => tr(ui, "Save “{name}”").replace("{name}", title),
        _ => tr(ui, "Save All").to_string(),
    };
    // Its picker's answer carries on; nothing else may start meanwhile, but Cancel stops the rest.
    let idle = saving.is_none();
    ui.horizontal(|ui| {
        if ui.add_enabled(idle, egui::Button::new(save_label)).clicked() && app.commit_edit(0, 0, false, false) {
            if app.close_queue.is_some() {
                app.run_close_queue(true);
            } else {
                app.save_all(targets.clone(), unsaved.clone(), then);
            }
            *open = app.close_queue.is_some();
            ui.ctx().request_repaint();
        }
        if ui.add_enabled(idle, egui::Button::new(tr(ui, "Don't Save"))).clicked() {
            *open = false;
            app.close_queue = None;
            app.finish_close(&targets, then);
            ui.ctx().request_repaint();
        }
        if ui.button(tr(ui, "Cancel")).clicked() {
            *open = false;
            app.close_queue = None;
        }
    });
}
