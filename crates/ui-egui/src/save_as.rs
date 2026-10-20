//! Saving the workbook itself under a name: the desktop save picker or a browser download. The
//! workbook keeps its kind (an `.xlsm` stays one, VBA project and all), and a save that would
//! leave a VBA project out asks first.

use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::Dialog;
use crate::widgets::tr;

/// The dialog that asks before a save leaves the workbook's macros out.
pub const DROP_MACROS: &str = "dropMacros";
pub const DROP_MACROS_TITLE: &str = "Save Without Macros?";
pub const DROP_MACROS_TEXT: &str = "This workbook contains VBA macros. An Excel Workbook (.xlsx) can't hold them, so the saved file won't have them. To keep them, save it as an Excel Macro-Enabled Workbook (.xlsm).";
pub const SAVE_AS_XLSM: &str = "Save as .xlsm instead";
pub const SAVE_AS_XLSX: &str = "Save anyway as .xlsx";

impl SheetApp {
    /// Save As for the workbook. Params: `format` (`xlsx` or `xlsm`, the type chosen in File ›
    /// Save As; otherwise the workbook's own), `name` (what the picker suggests), `path` (a desktop
    /// file already picked) and `dropMacros` (the user agreed to leave the VBA project out).
    pub(crate) fn save_workbook_as(&mut self, params: &Json) {
        let Some(d) = self.session.active() else { return };
        let title = d.display_title();
        let chosen = params.get("format").and_then(Json::as_str).filter(|f| matches!(*f, "xlsx" | "xlsm"));
        let format = chosen.unwrap_or_else(|| gridcraft_engine::io::workbook_ext(&title));
        let may_drop = params.get("dropMacros").and_then(Json::as_bool) == Some(true);
        let drops = |name: &str| !may_drop && gridcraft_engine::io::drops_macros(&d.wb, name);
        if let Some(pick) = &self.services.pick_save {
            let path = match params.get("path").and_then(Json::as_str) {
                Some(path) => path.to_string(),
                None => {
                    let suggested = match (params.get("name").and_then(Json::as_str), chosen) {
                        (Some(name), _) => name.to_string(),
                        (None, Some(format)) => format!("{}.{format}", stem(&title)),
                        (None, None) => title.clone(),
                    };
                    let Some(path) = pick(&suggested) else { return };
                    if std::path::Path::new(&path).extension().is_none() { format!("{path}.{format}") } else { path }
                }
            };
            if drops(&path) {
                self.open_dialog(DROP_MACROS, json!({"path": path}));
            } else {
                self.run_or_alert("file.save", json!({"path": path}));
            }
        } else if self.services.download.is_some() {
            if drops(&format!("{}.{format}", stem(&title))) {
                self.open_dialog(DROP_MACROS, json!({}));
                return;
            }
            match self.run("file.saveBytes", json!({"format": format})) {
                Ok(r) => {
                    if let Some(b) = r.get("base64").and_then(Json::as_str).and_then(gridcraft_engine::io::base64_decode) {
                        let name = r.get("name").and_then(Json::as_str).unwrap_or("Book.xlsx").to_string();
                        self.download_workbook(&name, &b);
                    }
                }
                Err(e) => {
                    self.note_save_result(&Err(e.clone()));
                    self.alert(e);
                }
            }
        }
    }

    /// File › Save As offers `.xlsm`: the workbook has a VBA project or is macro-enabled.
    pub(crate) fn offers_xlsm(&self) -> bool {
        self.session.active().is_some_and(|d| d.wb.vba_project.is_some() || gridcraft_engine::io::workbook_ext(&d.display_title()) != "xlsx")
    }
}

fn stem(name: &str) -> &str {
    std::path::Path::new(name).file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or("Book1")
}

/// The [`DROP_MACROS`] dialog's body: save as `.xlsm` instead, save without the macros, or cancel.
pub(crate) fn drop_macros_body(app: &mut SheetApp, ui: &mut egui::Ui, d: &Dialog, open: &mut bool) {
    ui.label(tr(ui, DROP_MACROS_TEXT));
    ui.add_space(10.0);
    let path = d.values.get("path").and_then(Json::as_str).map(str::to_string);
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr(ui, SAVE_AS_XLSM)).clicked() {
            *open = false;
            let mut params = json!({"format": "xlsm"});
            // The desktop picker comes back with the name just picked, now ending in .xlsm.
            if let Some(path) = &path {
                params["name"] = json!(std::path::Path::new(path).with_extension("xlsm").to_string_lossy());
            }
            app.save_workbook_as(&params);
        }
        if ui.button(tr(ui, SAVE_AS_XLSX)).clicked() {
            *open = false;
            let mut params = json!({"format": "xlsx", "dropMacros": true});
            if let Some(path) = &path {
                params["path"] = json!(path);
            }
            app.save_workbook_as(&params);
        }
        if ui.button(tr(ui, "Cancel")).clicked() {
            *open = false;
        }
    });
}
