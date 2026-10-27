//! Share… (web, with the office shell): a copy of the workbook goes to the system share sheet
//! (Zalo, mail, …) as the bytes Save would download. It is an export: the workbook's saved
//! state doesn't change.

use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::save_as::DROP_MACROS;

pub const SHARE: &str = "Share…";

impl SheetApp {
    /// Whether the host can share files, so the Share menu offers [`SHARE`].
    pub fn can_share(&self) -> bool {
        self.services.share.is_some()
    }

    /// Shares the active workbook in its own kind (an `.xlsm` stays one, an `.xls` becomes
    /// `.xlsx`), asking first when that leaves its macros out. Params as for Save As: `format`
    /// (`xlsx` or `xlsm`) and `dropMacros`. Must run inside the click that asked for it;
    /// `by_pointer` false (Enter, Space, a screen reader) downloads instead.
    pub(crate) fn share_workbook(&mut self, params: &Json, by_pointer: bool) {
        if self.services.share.is_none() || !self.commit_edit(0, 0, false, false) {
            return;
        }
        let Some(d) = self.session.active() else { return };
        let title = d.display_title();
        let format = params.get("format").and_then(Json::as_str).filter(|f| matches!(*f, "xlsx" | "xlsm"));
        let format = format.unwrap_or_else(|| gridcraft_engine::io::workbook_ext(&title)).to_string();
        let may_drop = params.get("dropMacros").and_then(Json::as_bool) == Some(true);
        let stem = std::path::Path::new(&title).file_stem().and_then(|s| s.to_str()).unwrap_or("Book1");
        if !may_drop && gridcraft_engine::io::drops_macros(&d.wb, &format!("{stem}.{format}")) {
            self.open_dialog(DROP_MACROS, json!({"share": true}));
            return;
        }
        let encoded = match self.run("file.saveBytes", json!({"format": format})) {
            Ok(r) => r,
            Err(e) => {
                self.alert(e);
                return;
            }
        };
        let Some(bytes) = encoded.get("base64").and_then(Json::as_str).and_then(gridcraft_engine::io::base64_decode) else { return };
        let name = encoded.get("name").and_then(Json::as_str).unwrap_or("Book.xlsx");
        // egui reports keyboard and accessibility activation a frame later, outside the browser's
        // user activation, where the share sheet would be refused.
        let shared = by_pointer && self.services.share.as_ref().is_some_and(|share| share(name, &bytes));
        if !shared && let Some(download) = &self.services.download {
            // A copy, like an export: its delivery never marks the workbook saved.
            download(name, &bytes, crate::DeliveryReport::default());
        }
    }
}
