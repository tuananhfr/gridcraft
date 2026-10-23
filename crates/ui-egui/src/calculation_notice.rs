use crate::{Notice, NoticeKind, SheetApp};

pub const MESSAGE: &str = "Could not calculate {count} cells. Open Error Checking for details.";

fn udf_notice(doc: &gridcraft_engine::DocState) -> Option<serde_json::Value> {
    let analysis = gridcraft_engine::macros::project(&doc.wb)?;
    for (si, sh) in doc.wb.sheets.iter().enumerate() {
        for (c, cell) in sh.cells.iter() {
            if let Some(error) = gridcraft_engine::macros::udf_runtime_diagnostic(doc, si, c) {
                return Some(error);
            }
            if cell.value.as_error() == Some(gridcraft_engine::core::CellError::Name)
                && let Some(error) = gridcraft_engine::macros::udf_diagnostic_with(Some(&analysis), &doc.wb, si, cell)
            {
                return Some(error);
            }
        }
    }
    None
}

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
        if self.session.active().is_none_or(|doc| doc.calc.not_calculated.is_empty() && udf_notice(doc).is_none()) {
            self.remove_calculation_notice_line();
        }
    }

    pub(crate) fn show_calculation_notice(&mut self, force: bool) {
        self.calculation_notice_seen.retain(|uid| self.session.documents().iter().any(|d| d.uid == *uid));
        let Some(doc) = self.session.active() else { return };
        let count = doc.calc.not_calculated.len();
        let udf = if count == 0 && (force || !self.calculation_notice_seen.contains(&doc.uid)) { udf_notice(doc) } else { None };
        if count == 0 && udf.is_none() {
            self.calculation_notice_seen.insert(doc.uid);
            if force {
                self.remove_calculation_notice_line();
            }
            return;
        }
        if !force && !self.calculation_notice_seen.insert(doc.uid) {
            return;
        }
        self.calculation_notice_seen.insert(doc.uid);
        let enable =
            !doc.macro_state.consent && udf.as_ref().is_some_and(|e| e.get("cause").and_then(serde_json::Value::as_str) == Some("udfUnavailable"));
        self.remove_calculation_notice_line();
        let action =
            if enable { ("Enable macros to calculate".into(), "udfConsent".into()) } else { ("Error Checking".into(), "errorChecking".into()) };
        let text = udf
            .map(|error| crate::macro_dialog::diagnostic_text(self.ui.language, &error))
            .unwrap_or_else(|| self.ui.language.tr(MESSAGE).replace("{count}", &count.to_string()));
        self.calculation_notice_line = Some(text.clone());
        if let Some(notice) = &mut self.notice {
            if !notice.text.lines().any(|line| line == text) {
                notice.text = format!("{}\n{text}", self.ui.language.tr(&notice.text));
            }
            notice.action = Some(action.clone());
        } else {
            self.notice = Some(Notice { text, kind: NoticeKind::Warning, action: Some(action.clone()) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn macro_notices_explain_existing_and_missing_udfs_and_clear_when_resolved() {
        use gridcraft_vba::testing::{FixtureModule, fixture};
        use serde_json::json;
        use std::sync::Arc;
        for (formula, expected) in [
            ("=_xludf.Twice(2)", "Hàm VBA tự định nghĩa `Twice` (Module1, dòng 1) — Bảng tính chưa chạy macro nên chưa tính được"),
            ("=_xludf.Gone()", "Không tìm thấy hàm `GONE` trong macro của tệp"),
        ] {
            let mut session = gridcraft_engine::Session::new();
            session.new_workbook();
            Arc::make_mut(&mut session.active_mut().unwrap().wb).vba_project = Some(
                fixture(
                    &[FixtureModule::new("Module1", "Module", "Public Function Twice(x As Double) As Double\nTwice = x * 2\nEnd Function").unwrap()],
                    1252,
                    "",
                    &[],
                )
                .unwrap(),
            );
            session.execute("cell.set", json!({"cell": "A1", "input": formula})).unwrap();
            let mut app = SheetApp::new(session, Default::default());
            app.ui.language = Language::Vi;
            app.show_calculation_notice(true);
            assert_eq!(app.notice.as_ref().unwrap().text, expected);
            app.notice = None;
            app.show_calculation_notice(false);
            assert!(app.notice.is_none());
            app.run("cell.set", json!({"cell": "A1", "input": "2"})).unwrap();
            app.show_calculation_notice(true);
            assert!(app.notice.is_none());
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn udf_notice_action_opens_existing_consent_without_running_a_sub() {
        use gridcraft_vba::testing::{FixtureModule, fixture};
        use serde_json::json;
        use std::sync::Arc;
        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        Arc::make_mut(&mut session.doc_mut().unwrap().wb).vba_project = Some(
            fixture(&[FixtureModule::new("Module1", "Module", "Public Function Twice(x)\nTwice=x*2\nEnd Function").unwrap()], 1252, "", &[]).unwrap(),
        );
        session.execute("cell.set", json!({"cell":"A1", "input":"=Twice(3)"})).unwrap();
        let mut app = SheetApp::new(session, Default::default());
        app.ui.language = Language::Vi;
        app.show_calculation_notice(true);
        assert_eq!(app.notice.as_ref().unwrap().action, Some(("Enable macros to calculate".into(), "udfConsent".into())));
        use egui_kittest::kittest::Queryable;
        let screenshots = std::env::var_os("GRIDCRAFT_UDF_SCREENSHOTS");
        let builder = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0));
        let builder = if screenshots.is_some() { builder.wgpu() } else { builder };
        let mut h = builder.build_ui_state(
            |ui, app: &mut SheetApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
        h.run_steps(6);
        if let Some(dir) = &screenshots {
            std::fs::create_dir_all(dir).unwrap();
            h.render().unwrap().save(std::path::Path::new(dir).join("udf-consent-off-vi.png")).unwrap();
        }
        h.get_by_label("Bật macro để tính").click();
        h.run_steps(3);
        assert!(h.query_by_label("Cho phép hàm VBA trong công thức?").is_some());
        if let Some(dir) = &screenshots {
            h.render().unwrap().save(std::path::Path::new(dir).join("udf-consent-modal-vi.png")).unwrap();
        }
        h.get_by_label("Cho phép hàm VBA").click();
        h.run_steps(4);
        assert!(h.state().notice.is_none());
        assert!(h.state().session.doc().unwrap().macro_state.log.is_empty());
        assert_eq!(
            h.state().session.doc().unwrap().wb.sheet(0).unwrap().value(gridcraft_engine::core::CellRef::new(0, 0)),
            gridcraft_engine::core::Value::Number(6.0)
        );
        if let Some(dir) = &screenshots {
            h.render().unwrap().save(std::path::Path::new(dir).join("udf-consent-on-vi.png")).unwrap();
        }
        for language in crate::i18n::Language::ALL.into_iter().filter(|l| *l != crate::i18n::Language::En) {
            for key in ["Enable macros to calculate", gridcraft_engine::macros::UDF_RUNTIME, gridcraft_engine::macros::UDF_LIMIT] {
                assert_ne!(language.tr(key), key);
            }
        }
    }

    #[test]
    fn calculation_and_legacy_messages_are_translated_in_every_catalogue() {
        for lang in Language::ALL.into_iter().filter(|lang| *lang != Language::En) {
            for text in [
                MESSAGE,
                gridcraft_engine::io::LEGACY_SIGNATURE,
                gridcraft_engine::io::DROPPED_SIGNATURE,
                gridcraft_engine::io::UNSIGNED_MACROS,
                gridcraft_engine::io::REMOVED_MACROS,
                gridcraft_engine::io::REMOVED_SIGNED_MACROS,
                "too many VBA signature parts",
                "too many macro buttons",
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
