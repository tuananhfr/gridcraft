//! Whether a document's save is in flight or failed, for the status bar: "Saving…" while the
//! browser's save picker or write is pending, "Save failed" (reason on hover and click) until
//! the next successful save or edit.

use std::collections::HashMap;
use std::sync::Arc;

use gridcraft_engine::model::Workbook;

use crate::SheetApp;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveState {
    Idle,
    Saving,
    Failed(String),
}

/// Failed saves by document uid: the reason and the workbook as it was then, so an edit (a
/// different workbook) clears it.
#[derive(Debug, Default)]
pub struct SaveFailures(HashMap<u64, (String, Arc<Workbook>)>);

impl SaveFailures {
    pub fn record(&mut self, uid: u64, wb: Arc<Workbook>, reason: String) {
        self.0.insert(uid, (reason, wb));
    }

    pub fn clear(&mut self, uid: u64) {
        self.0.remove(&uid);
    }

    fn reason(&self, uid: u64, wb: &Arc<Workbook>) -> Option<&str> {
        self.0.get(&uid).filter(|(_, at)| Arc::ptr_eq(at, wb)).map(|(r, _)| r.as_str())
    }
}

impl SheetApp {
    /// The save state of open document `uid`.
    pub fn save_state(&self, uid: u64) -> SaveState {
        if self.pending_saves.iter().any(|p| p.uid == uid) {
            return SaveState::Saving;
        }
        match self.session.documents().iter().find(|d| d.uid == uid).and_then(|d| self.save_failures.reason(uid, &d.wb)) {
            Some(reason) => SaveState::Failed(reason.to_string()),
            None => SaveState::Idle,
        }
    }

    /// Notes how a save of the active document went: a failure until the next successful save
    /// or edit; success once nothing is left unsaved.
    pub(crate) fn note_save_result(&mut self, result: &Result<serde_json::Value, String>) {
        let Some(d) = self.session.active() else { return };
        match result {
            // The read-only notice already says why; nothing was attempted.
            Err(e) if SheetApp::access_notice(e).is_some() => {}
            Err(e) => self.save_failures.record(d.uid, d.wb.clone(), crate::clean_error(e)),
            Ok(_) if !d.is_dirty() => self.save_failures.clear(d.uid),
            Ok(_) => {}
        }
    }
}

/// The status bar's save state: "Saving…" while any save is in flight (Save All may be saving
/// another document), else the active document's failure; nothing when idle.
pub(crate) fn status_label(app: &mut SheetApp, ui: &mut egui::Ui, t: &crate::theme::Tokens) {
    let Some(uid) = app.session.active().map(|d| d.uid) else { return };
    let lang = app.ui.language;
    let saving: Vec<&str> = app.pending_saves.iter().map(|p| p.name.as_str()).collect();
    let state = if saving.is_empty() { app.save_state(uid) } else { SaveState::Saving };
    match state {
        SaveState::Idle => {}
        SaveState::Saving => {
            ui.add_space(12.0);
            let names = saving.join("\n");
            ui.label(egui::RichText::new(lang.tr("Saving…")).font(crate::theme::ui_font(12.5)).color(t.text_dim)).on_hover_text(names);
        }
        SaveState::Failed(reason) => {
            ui.add_space(12.0);
            let reason = app.product_message(&reason);
            let label = egui::Label::new(egui::RichText::new(lang.tr("Save failed")).font(crate::theme::ui_font(12.5)).color(t.danger))
                .sense(egui::Sense::click());
            if ui.add(label).on_hover_text(&reason).clicked() {
                app.message = Some(("Save failed".into(), reason));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> SheetApp {
        let mut s = gridcraft_engine::Session::new();
        s.new_workbook();
        SheetApp::new(s, Default::default())
    }

    #[test]
    fn a_failure_lasts_until_an_edit_or_a_successful_save() {
        let mut app = app();
        app.run("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
        let uid = app.session.active().unwrap().uid;
        app.note_save_result(&Err("file.save: disk full".into()));
        assert_eq!(app.save_state(uid), SaveState::Failed("file.save: disk full".into()));
        app.run("cell.set", json!({"cell": "A2", "input": "2"})).unwrap();
        assert_eq!(app.save_state(uid), SaveState::Idle, "an edit clears it");

        app.note_save_result(&Err("disk full".into()));
        let d = app.session.active_mut().unwrap();
        d.saved = d.wb.clone();
        app.note_save_result(&Ok(json!({"path": "x.xlsx"})));
        assert_eq!(app.save_state(uid), SaveState::Idle, "a successful save clears it");
    }
}
