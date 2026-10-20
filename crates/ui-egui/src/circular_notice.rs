use std::sync::Arc;

use gridcraft_engine::{circular, model::Workbook};

use crate::{Notice, NoticeKind, SheetApp};

pub const MESSAGE: &str = "This formula creates a circular reference, so the cell displays #CIRC!. Edit the formula so it does not refer to itself.";

impl SheetApp {
    pub(crate) fn circular_notice_after_edit(&mut self, before: Option<(u64, Arc<Workbook>)>) {
        if let Some((uid, wb)) = before
            && self.session.active().is_some_and(|doc| doc.uid == uid && !Arc::ptr_eq(&wb, &doc.wb) && circular::created_by_edit(&wb, doc))
        {
            self.circular_notice_pending.remove(&uid);
            self.append_circular_notice();
        }
    }

    pub(crate) fn show_circular_open_notice(&mut self) {
        self.circular_notice_pending.retain(|uid| self.session.documents().iter().any(|doc| doc.uid == *uid));
        let Some(doc) = self.session.active() else { return };
        if self.circular_notice_pending.remove(&doc.uid) && circular::has_circular_reference(&doc.wb) {
            self.append_circular_notice();
        }
    }

    fn append_circular_notice(&mut self) {
        let text = self.ui.language.tr(MESSAGE);
        if let Some(notice) = &mut self.notice {
            let existing = self.ui.language.tr(&notice.text);
            if !existing.lines().any(|line| line == text || line == MESSAGE) {
                notice.text = format!("{existing}\n{text}");
            }
        } else {
            self.notice = Some(Notice { text: MESSAGE.into(), kind: NoticeKind::Info, action: None });
        }
    }
}

#[cfg(test)]
mod tests {
    use gridcraft_engine::Session;
    use serde_json::json;

    use super::*;
    use crate::i18n::Language;

    fn app() -> SheetApp {
        let mut session = Session::new();
        session.new_workbook();
        let mut app = SheetApp::new(session, Default::default());
        app.ui.language = Language::Vi;
        app
    }

    #[test]
    fn circular_edit_notifies_once_and_acyclic_edits_do_not_notify() {
        let mut app = app();
        app.run("cell.set", json!({"cell": "A1", "input": "=1+2"})).unwrap();
        assert!(app.notice.is_none());
        app.run("cell.set", json!({"cell": "A1", "input": "=A1"})).unwrap();
        assert_eq!(
            app.ui.language.tr(&app.notice.as_ref().unwrap().text),
            "Công thức này tạo ra tham chiếu vòng tròn nên ô hiển thị #CIRC!. Hãy sửa công thức để không tự tham chiếu đến chính nó."
        );
        app.run("ui.notice.dismiss", json!({})).unwrap();
        app.run("formulas.calculateNow", json!({})).unwrap();
        app.show_circular_open_notice();
        app.run("cell.set", json!({"cell": "B1", "input": "=2+3"})).unwrap();
        assert!(app.notice.is_none());
        app.run("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
        app.run("cell.set", json!({"cell": "A1", "input": "=A1"})).unwrap();
        assert!(app.notice.is_some(), "a later edit may notify again");
    }

    #[test]
    fn committing_an_in_cell_or_formula_bar_edit_shows_the_notice() {
        for from_formula_bar in [false, true] {
            let mut app = app();
            app.begin_edit(Some("=1+2".into()), from_formula_bar);
            assert!(app.commit_edit(0, 0, false, false));
            assert!(app.notice.is_none());
            app.begin_edit(Some("=A1".into()), from_formula_bar);
            assert!(app.commit_edit(0, 0, false, false));
            assert!(app.notice.is_some());
            app.run("ui.notice.dismiss", json!({})).unwrap();
            app.show_circular_open_notice();
            assert!(app.notice.is_none());
        }
    }

    #[test]
    fn one_range_edit_with_multiple_cycles_has_one_notice() {
        let mut app = app();
        app.run("range.setValues", json!({"range": "A1", "values": [["=A1", "=B1"]]})).unwrap();
        let notice = app.notice.as_ref().unwrap();
        assert_eq!(notice.text.lines().count(), 1);
        assert!(matches!(notice.kind, NoticeKind::Info));
        app.run("ui.notice.dismiss", json!({})).unwrap();
        app.run("formulas.calculateNow", json!({})).unwrap();
        assert!(app.notice.is_none());
    }

    #[test]
    fn an_acyclic_workbook_opens_without_a_circular_notice() {
        let mut app = app();
        app.run("cell.set", json!({"cell": "A1", "input": "=1+2"})).unwrap();
        let saved = app.run("file.saveBytes", json!({})).unwrap();
        let result = app.run("file.open", json!({"name": "Acyclic.xlsx", "base64": saved["base64"]})).unwrap();
        app.after_open(None, &result);
        app.show_circular_open_notice();
        assert!(app.notice.is_none());
    }

    #[test]
    fn opened_cycle_notifies_once_and_preserves_other_warnings() {
        let mut app = app();
        app.run("cell.set", json!({"cell": "A1", "input": "=A1"})).unwrap();
        let saved = app.run("file.saveBytes", json!({})).unwrap();
        let result = app.run("file.open", json!({"name": "Circular.xlsx", "base64": saved["base64"]})).unwrap();
        app.after_open(None, &result);
        app.notice = Some(Notice { text: "Warning".into(), kind: NoticeKind::Warning, action: Some(("Details".into(), "macros".into())) });
        app.show_circular_open_notice();
        let notice = app.notice.as_ref().unwrap();
        assert!(notice.text.contains(Language::Vi.tr(MESSAGE)));
        assert!(matches!(notice.kind, NoticeKind::Warning));
        assert!(notice.action.is_some());
        app.notice = None;
        app.show_circular_open_notice();
        assert!(app.notice.is_none());
    }

    #[test]
    fn all_catalogs_translate_the_circular_notice() {
        for language in Language::ALL.into_iter().filter(|language| *language != Language::En) {
            assert_ne!(language.tr(MESSAGE), MESSAGE, "{language:?}");
        }
    }
}
