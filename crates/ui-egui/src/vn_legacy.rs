//! A dismissible conversion offer for each opened legacy-font workbook.
use crate::{Notice, NoticeKind, SheetApp};

pub const MESSAGE: &str = "This workbook uses legacy Vietnamese fonts. Convert its text to Unicode for correct display.";
pub const ACTION: &str = "Convert to Unicode";

pub(crate) fn has_legacy_fonts(wb: &gridcraft_engine::model::Workbook) -> bool {
    wb.sheets.iter().any(|sheet| {
        sheet.cells.iter().any(|(pos, _)| gridcraft_engine::vn_legacy::encoding(&wb.styles.get(sheet.style_id(pos)).font.name).is_some())
    })
}

impl SheetApp {
    pub(crate) fn show_vn_legacy_notice(&mut self) {
        self.vn_notice_pending.retain(|uid| self.session.documents().iter().any(|doc| doc.uid == *uid));
        let Some(doc) = self.session.active() else { return };
        if !self.vn_notice_pending.contains(&doc.uid) || self.notice.is_some() {
            return;
        }
        let legacy = has_legacy_fonts(&doc.wb);
        self.vn_notice_pending.remove(&doc.uid);
        if legacy {
            self.notice =
                Some(Notice { text: MESSAGE.into(), kind: NoticeKind::Info, action: Some((ACTION.into(), "convertVietnameseLegacy".into())) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_engine::{
        Session,
        core::{CellRef, Value},
    };
    use std::sync::Arc;
    #[test]
    fn notice_is_only_for_legacy_fonts_and_dismissal_lasts() {
        let mut session = Session::new();
        session.new_workbook();
        let mut app = SheetApp::new(session, crate::Services::default());
        let uid = app.session.active().unwrap().uid;
        app.vn_notice_pending.insert(uid);
        app.show_vn_legacy_notice();
        assert!(app.notice.is_none());
        let d = app.session.doc_mut().unwrap();
        let wb = Arc::make_mut(&mut d.wb);
        let style = wb.styles.derive(Default::default(), |s| s.font.name = ".VnTime".into());
        let sheet = Arc::make_mut(wb.sheets.first_mut().unwrap());
        sheet.set_value(CellRef::new(0, 0), Value::from("b\u{b6}ng"));
        sheet.set_style(CellRef::new(0, 0), style);
        app.vn_notice_pending.insert(uid);
        app.show_vn_legacy_notice();
        assert_eq!(app.notice.as_ref().unwrap().action.as_ref().unwrap().0, ACTION);
        app.notice = None;
        app.show_vn_legacy_notice();
        assert!(app.notice.is_none());
        app.vn_notice_pending.insert(uid);
        app.show_vn_legacy_notice();
        app.run("review.convertVietnameseLegacy", serde_json::json!({})).unwrap();
        app.show_vn_legacy_notice();
        assert!(app.notice.is_none());
    }

    #[test]
    fn new_strings_are_translated() {
        for text in [MESSAGE, ACTION, "Convert TCVN3/VNI to Unicode", "Convert TCVN3 to Unicode", "Convert VNI to Unicode"] {
            assert_ne!(crate::i18n::Language::Vi.tr(text), text);
        }
    }

    #[test]
    fn opening_legacy_xls_offers_conversion_and_converts_its_cell_font() {
        fn rec(id: u16, payload: &[u8]) -> Vec<u8> {
            let mut bytes = id.to_le_bytes().to_vec();
            bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
            bytes.extend_from_slice(payload);
            bytes
        }
        fn u16s(values: &[u16]) -> Vec<u8> {
            values.iter().flat_map(|n| n.to_le_bytes()).collect()
        }
        fn font(name: &str, flags: u16, weight: u16) -> Vec<u8> {
            let mut payload = u16s(&[240, flags, 0x7FFF, weight, 0]);
            payload.extend_from_slice(&[1, 0, 0, 0, name.len() as u8, 0]);
            payload.extend_from_slice(name.as_bytes());
            rec(0x0031, &payload)
        }
        let globals = |pos: u32| {
            let mut bytes = rec(0x0809, &u16s(&[0x0600, 5, 0, 0, 0, 0, 0, 0]));
            bytes.extend(font("Calibri", 0, 400));
            bytes.extend(font(".VnTime", 0x000A, 700));
            for ifnt in [0, 1] {
                let mut xf = u16s(&[ifnt, 0, 0]);
                xf.extend_from_slice(&[0; 14]);
                bytes.extend(rec(0x00E0, &xf));
            }
            let mut bound = pos.to_le_bytes().to_vec();
            bound.extend_from_slice(&[0, 0, 1, 0, b'S']);
            bytes.extend(rec(0x0085, &bound));
            bytes.extend(rec(0x000A, &[]));
            bytes
        };
        let mut stream = globals(globals(0).len() as u32);
        stream.extend(rec(0x0809, &u16s(&[0x0600, 0x10, 0, 0, 0, 0, 0, 0])));
        let source = "b\u{b6}ng c\u{ad}\u{ea}ng \u{ae}\u{e9}";
        let units: Vec<u16> = source.encode_utf16().collect();
        let mut label = u16s(&[0, 0, 1, units.len() as u16]);
        label.push(1);
        label.extend(u16s(&units));
        stream.extend(rec(0x0204, &label));
        stream.extend(rec(0x000A, &[]));
        let bytes = gridcraft_xlsx::testing::compound_file(&[("Workbook", &stream)]);
        let mut app = SheetApp::new(Session::new(), crate::Services::default());
        let opened = app.run("file.open", serde_json::json!({"name": "legacy.xls", "base64": gridcraft_engine::io::base64_encode(&bytes)})).unwrap();
        let pos = CellRef::new(0, 0);
        let before = app.session.doc().unwrap().wb.clone();
        assert_eq!(before.styles.get(before.sheets[0].style_id(pos)).font.name, ".VnTime");
        assert!(has_legacy_fonts(&before));
        app.show_open_warnings(&opened);
        app.show_vn_legacy_notice();
        assert!(app.vn_notice_pending.contains(&app.session.doc().unwrap().uid));
        app.run("ui.notice.dismiss", serde_json::json!({})).unwrap();
        app.show_vn_legacy_notice();
        assert_eq!(app.notice.as_ref().unwrap().action, Some((ACTION.into(), "convertVietnameseLegacy".into())));
        let result = app.run("review.convertVietnameseLegacy", serde_json::json!({})).unwrap();
        assert_eq!(result["convertedCells"], 1);
        let wb = &app.session.doc().unwrap().wb;
        assert_eq!(wb.sheets[0].value(pos), Value::from("b\u{1ea3}ng c\u{1b0}\u{1edd}ng \u{111}\u{1ed9}"));
        let converted = wb.styles.get(wb.sheets[0].style_id(pos));
        assert_eq!(converted.font.name, "Calibri");
        assert!(converted.font.bold && converted.font.italic && converted.font.strike);
        assert_eq!(converted.font.size, 12.0);
        assert_eq!(converted.font.underline, gridcraft_engine::model::Underline::Single);
        let saved = gridcraft_xlsx::write_xlsx(wb).unwrap();
        let (back, _) = gridcraft_xlsx::read_xlsx(&saved).unwrap();
        assert_eq!(back.sheets[0].value(pos), wb.sheets[0].value(pos));
        assert_eq!(back.styles.get(back.sheets[0].style_id(pos)).font, converted.font);
        assert_eq!(app.session.doc().unwrap().undo.len(), 1);
        app.run("edit.undo", serde_json::json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().wb.as_ref(), before.as_ref());
        app.show_vn_legacy_notice();
        assert!(app.notice.is_none());
    }
}
