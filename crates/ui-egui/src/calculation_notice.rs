use crate::{Notice, NoticeKind, SheetApp};

pub const MESSAGE: &str = "Could not calculate {count} cells. Open Error Checking for details.";

impl SheetApp {
    fn remove_calculation_notice_line(&mut self) {
        if let Some(line) = self.calculation_notice_line.take()
            && let Some(notice) = &mut self.notice
            && notice.text.lines().any(|text| text == line)
        {
            notice.text = notice.text.lines().filter(|text| *text != line).collect::<Vec<_>>().join("\n");
            if notice.text.is_empty() {
                self.notice = None;
            } else {
                notice.action = None;
            }
        }
    }

    pub(crate) fn clear_calculation_notice_if_resolved(&mut self) {
        if self.session.active().is_none_or(|doc| doc.calc.not_calculated.is_empty()) {
            self.remove_calculation_notice_line();
        }
    }

    pub(crate) fn show_calculation_notice(&mut self, force: bool) {
        self.calculation_notice_seen.retain(|uid| self.session.documents().iter().any(|d| d.uid == *uid));
        let Some(doc) = self.session.active() else { return };
        let count = doc.calc.not_calculated.len();
        if count == 0 {
            self.remove_calculation_notice_line();
            return;
        }
        if !force && !self.calculation_notice_seen.insert(doc.uid) {
            return;
        }
        self.calculation_notice_seen.insert(doc.uid);
        self.remove_calculation_notice_line();
        let text = self.ui.language.tr(MESSAGE).replace("{count}", &count.to_string());
        self.calculation_notice_line = Some(text.clone());
        if let Some(notice) = &mut self.notice {
            if !notice.text.lines().any(|line| line == text) {
                notice.text = format!("{}\n{text}", self.ui.language.tr(&notice.text));
            }
            notice.action = Some(("Error Checking".into(), "errorChecking".into()));
        } else {
            self.notice = Some(Notice { text, kind: NoticeKind::Warning, action: Some(("Error Checking".into(), "errorChecking".into())) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn calculation_and_legacy_messages_are_translated_in_every_catalogue() {
        for lang in Language::ALL.into_iter().filter(|lang| *lang != Language::En) {
            for text in [
                MESSAGE,
                gridcraft_engine::calc::recalc::NotCalculated::Unparsed.reason(),
                gridcraft_engine::calc::recalc::NotCalculated::Dependent.reason(),
                gridcraft_engine::legacy::OPENED_MACROS,
                gridcraft_engine::legacy::BUTTONS,
            ] {
                assert_ne!(lang.tr(text), text, "{lang:?}: {text}");
            }
        }
    }
}
