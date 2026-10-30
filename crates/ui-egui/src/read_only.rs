//! What the interface stops offering on a document its page host made read-only or closed to
//! downloads, or that the app's edit lock covers ([`DocAccess`]). The engine refuses the commands
//! themselves; this keeps editors, drags and menus from starting what would only be refused.

use gridcraft_engine::doc_access::{COMMENTS_LATER, DocAccess, NO_DOWNLOAD, READ_ONLY};

use crate::SheetApp;

/// Ribbon tabs that keep their reading controls on a read-only document, greying the rest item
/// by item; the others only edit.
pub(crate) const READING_TABS: &[&str] = &["Home", "Review", "View"];

/// Dialogs that only read the document, still offered when it is read-only.
const READING_DIALOGS: &[&str] = &["find", "goTo", "goToSpecial", "zoom", "accessibility", "statistics", "evaluateFormula"];

impl SheetApp {
    /// The active document can't be changed.
    pub fn read_only(&self) -> bool {
        self.session.active().is_some_and(|d| d.access.locked())
    }

    /// The active document's content may leave the editor (Save As, Export, Print, Copy).
    pub fn can_download(&self) -> bool {
        self.session.active().is_none_or(|d| !d.access.no_download)
    }

    /// Whether engine command `id` would run on the active document now.
    pub fn allows(&self, id: &str) -> bool {
        self.session.command_allowed(id).is_ok()
    }

    /// Whether a control that runs `id` (a command id, or `dialog:<name>`) is offered under the
    /// active document's access. Only access counts: a command's own conditions keep showing their
    /// message when clicked, as before.
    pub fn offers(&self, id: &str) -> bool {
        let Some(access) = self.session.active().map(|d| d.access).filter(|a| !a.is_full()) else { return true };
        match id.strip_prefix("dialog:") {
            Some(dialog) => !access.locked() || READING_DIALOGS.contains(&dialog),
            None => gridcraft_engine::find_command(id).is_none_or(|spec| access.refusal(spec).is_none()),
        }
    }

    /// Applies the host's access to open document `uid`. Turning it read-only first commits text
    /// still held in a cell, formula-bar or text-box editor on it as one undo step: the host
    /// contract never discards typed edits.
    pub fn set_document_access(&mut self, uid: u64, access: DocAccess) {
        // The edit lock is the app's, not the host's to lift.
        let access = DocAccess { edit_lock: self.session.edit_lock(), ..access };
        let active = self.session.active().is_some_and(|d| d.uid == uid);
        if access.locked() && active {
            self.commit_held_edits(uid);
        }
        let Some(d) = self.session.document_mut(uid) else { return };
        d.access = access;
        if !access.locked() {
            self.reopen_held_edit();
            return;
        }
        if active {
            self.stop_edit_gestures();
        }
    }

    /// Drops the gestures that would go on editing the active document once it is locked.
    pub(crate) fn stop_edit_gestures(&mut self) {
        self.session.format_painter = None;
        self.grid.drag = crate::grid::Drag::None;
        self.grid.filter_menu = None;
        self.grid.list_picker = None;
        self.grid.renaming_tab = None;
    }

    /// Commits the editors open on active document `uid`. A cell edit the workbook refuses (a
    /// Stop validation rule, a formula that doesn't parse) keeps its usual message and is held
    /// for [`SheetApp::reopen_held_edit`] instead of being lost.
    pub(crate) fn commit_held_edits(&mut self, uid: u64) {
        if self.editor.is_some() && !self.commit_edit(0, 0, false, false) {
            log::warn!("read-only switch: the open cell edit was refused; kept until the document is editable");
            if let Some(edit) = self.editor.take() {
                self.held_edits.insert(uid, edit);
            }
            self.cancel_edit();
        }
        if self.text_box_editor.is_some()
            && let Err(e) = self.commit_text_box_edit()
        {
            log::warn!("read-only switch: the open text box edit was dropped: {e}");
            self.cancel_text_box_edit();
        }
    }

    /// Reopens the cell editor a read-only switch closed, with its text, once its document is the
    /// active one and editable again; edits of documents closed since are forgotten.
    pub(crate) fn reopen_held_edit(&mut self) {
        if self.held_edits.is_empty() {
            return;
        }
        let open: Vec<u64> = self.session.documents().iter().map(|d| d.uid).collect();
        self.held_edits.retain(|uid, _| open.contains(uid));
        let Some(uid) = self.session.active().filter(|d| !d.access.locked()).map(|d| d.uid) else { return };
        // Not yet shown: the next frame's document switch would close the editor again.
        if self.editing_in_place() || self.shown_doc.is_some_and(|shown| shown != uid) {
            return;
        }
        let Some(mut edit) = self.held_edits.remove(&uid) else { return };
        let Some(d) = self.session.active_mut() else { return };
        if edit.sheet >= d.wb.sheets.len() {
            return;
        }
        d.wb_switch_sheet(edit.sheet);
        d.selection = gridcraft_engine::Selection::at(edit.cell);
        edit.request_focus = true;
        self.session.mode = if edit.enter_mode { gridcraft_engine::Mode::Enter } else { gridcraft_engine::Mode::Edit };
        self.editor = Some(edit);
        self.grid.ensure_visible = true;
    }

    /// The notice for a refusal caused by the document's access or the edit lock, untranslated.
    pub(crate) fn access_notice(&self, error: &str) -> Option<&'static str> {
        [READ_ONLY, COMMENTS_LATER, NO_DOWNLOAD].into_iter().chain(self.session.edit_lock()).find(|why| error.ends_with(why))
    }

    /// Refuses starting an edit of a locked document with the usual notice.
    pub(crate) fn refuse_read_only_edit(&mut self) -> bool {
        let Some(access) = self.session.active().map(|d| d.access).filter(|a| a.locked()) else { return false };
        let notice = access.read_only_notice();
        self.toast = Some((notice.into(), crate::now_ms()));
        self.note_edit_refusal(notice);
        true
    }
}
