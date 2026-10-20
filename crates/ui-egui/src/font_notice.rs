//! One substitution notice per opened workbook, after font discovery and binding.

use std::collections::HashSet;

use crate::{Notice, NoticeKind, SheetApp, fonts, i18n::Language};

pub const MESSAGE: &str = "The fonts {fonts} are not available on this device and are displayed using similar fonts.";
pub const MORE: &str = "and {count} more";

pub fn format_message(language: Language, names: &[String]) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    let mut listed = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > 3 {
        listed.push(' ');
        listed.push_str(&language.tr(MORE).replace("{count}", &(names.len() - 3).to_string()));
    }
    Some(language.tr(MESSAGE).replace("{fonts}", &listed))
}

fn substituted(names: Vec<String>, available: impl Fn(&str) -> bool) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut missing: Vec<_> = names
        .into_iter()
        .filter(|name| !name.trim().is_empty() && seen.insert(name.to_lowercase()) && !available(name))
        .collect();
    missing.sort_by_key(|name| name.to_lowercase());
    missing
}

impl SheetApp {
    pub(crate) fn show_font_notice(&mut self) {
        if !fonts::scan_done() || !self.fonts_ready || self.fonts_rebind {
            return;
        }
        if self.session.active().is_some_and(|doc| self.fonts_seen != (doc.uid, doc.revision)) {
            return;
        }
        self.finish_font_notice(|name| fonts::is_bound(&fonts::role_key(name, false, false)));
    }

    fn finish_font_notice(&mut self, available: impl Fn(&str) -> bool) {
        self.font_notice_pending.retain(|uid| self.session.documents().iter().any(|doc| doc.uid == *uid));
        let Some(uid) = self.session.active().map(|doc| doc.uid) else { return };
        if !self.font_notice_pending.remove(&uid) {
            return;
        }
        let missing = substituted(self.used_fonts(), available);
        let Some(text) = format_message(self.ui.language, &missing) else { return };
        if let Some(notice) = &mut self.notice {
            // Import and security warnings must remain visible alongside font substitutions.
            notice.text = format!("{}\n{text}", self.ui.language.tr(&notice.text));
        } else {
            self.notice = Some(Notice { text, kind: NoticeKind::Info, action: None });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_zero_one_three_and_five_fonts() {
        let fonts: Vec<String> = ["A", "B", "C", "D", "E"].map(String::from).into();
        assert_eq!(format_message(Language::Vi, &[]), None);
        for (count, listed) in [(1, "A"), (3, "A, B, C"), (5, "A, B, C và 2 khác")] {
            assert_eq!(
                format_message(Language::Vi, &fonts[..count]),
                Some(format!("Các phông {listed} chưa có trên máy này nên được hiển thị bằng phông tương tự."))
            );
        }
    }

    #[test]
    fn filters_available_empty_and_duplicate_names() {
        let names = ["", "Present", "Missing", "missing", "Other"].map(String::from).into();
        assert_eq!(substituted(names, |name| name == "Present"), ["Missing", "Other"]);
    }

    #[test]
    fn notice_is_once_per_open_and_absent_when_all_fonts_are_available() {
        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        let mut app = SheetApp::new(session, crate::Services::default());
        let uid = app.session.active().unwrap().uid;
        app.font_notice_pending.insert(uid);
        app.finish_font_notice(|_| true);
        assert!(app.notice.is_none());
        app.font_notice_pending.insert(uid);
        app.finish_font_notice(|_| false);
        assert!(app.notice.is_some());
        app.notice = None;
        app.finish_font_notice(|_| false);
        assert!(app.notice.is_none(), "dismissing the notice must last for this open");
        app.font_notice_pending.insert(uid);
        app.finish_font_notice(|_| false);
        assert!(app.notice.is_some(), "another open may notify again");
    }

    #[test]
    fn font_notice_preserves_existing_warning_and_action() {
        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        let mut app = SheetApp::new(session, crate::Services::default());
        app.ui.language = Language::Vi;
        app.notice = Some(Notice { text: "Warning".into(), kind: NoticeKind::Warning, action: Some(("Details".into(), "macros".into())) });
        app.font_notice_pending.insert(app.session.active().unwrap().uid);
        app.finish_font_notice(|_| false);
        let notice = app.notice.as_ref().unwrap();
        assert!(notice.text.contains("Các phông"));
        assert!(notice.text.contains('\n'));
        assert!(matches!(notice.kind, NoticeKind::Warning));
        assert_eq!(notice.action, Some(("Details".into(), "macros".into())));
    }

    #[test]
    fn all_catalogs_translate_font_and_legacy_messages() {
        for language in Language::ALL.into_iter().filter(|language| *language != Language::En) {
            for message in [MESSAGE, MORE, gridcraft_engine::legacy::OPENED, gridcraft_engine::legacy::FAILED_TITLE, gridcraft_engine::legacy::FAILED] {
                assert_ne!(language.tr(message), message, "{language:?}: {message}");
            }
        }
    }
}
