use crate::{dialogs::Dialog, widgets::tr};
use serde_json::{Value as Json, json};

pub const SUMMARY: &str = "{modules} modules · {procedures} procedures · {udfs} UDFs · Events: {events} · Signed: {signed} · Locked: {locked}";
pub const LEVELS: [&str; 4] = ["Runnable with a macro runner", "Uses Windows API", "Uses COM (ETABS, AutoCAD, Access…)", "ActiveX / UserForm"];

fn text<'a>(value: &'a Json, key: &str) -> &'a str {
    value.get(key).and_then(Json::as_str).unwrap_or_default()
}
fn list<'a>(value: &'a Json, key: &str) -> &'a [Json] {
    value.get(key).and_then(Json::as_array).map(Vec::as_slice).unwrap_or_default()
}

fn button_kind(ui: &egui::Ui, button: &Json) -> String {
    let kind = text(button, "kind");
    tr(ui, if kind == "FormControl" { "Form Control" } else { kind }).to_string()
}

fn level_badge(ui: &mut egui::Ui, level: u64) -> egui::Response {
    let tokens = crate::theme::Tokens::get(ui.ctx());
    let color = match level {
        1 => tokens.accent,
        2 => tokens.notice_warn_border,
        3 => tokens.danger,
        _ => tokens.text_dim,
    };
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| ui.label(egui::RichText::new(tr(ui, "Macro|Level {level}").replace("{level}", &level.to_string())).color(color)))
        .inner
}

fn parameters(ui: &egui::Ui, procedure: &Json) -> String {
    list(procedure, "params")
        .iter()
        .map(|p| {
            let optional = if p.get("optional").and_then(Json::as_bool) == Some(true) { tr(ui, "Optional") } else { "" };
            let default = p.get("default").and_then(Json::as_str).map(|s| format!(" = {s}")).unwrap_or_default();
            format!("{optional} {}: {}{default}", text(p, "name"), p.get("type_name").and_then(Json::as_str).unwrap_or("Variant"))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn diagnostic_text(language: crate::i18n::Language, error: &Json) -> String {
    let reason = language.tr(text(error, "reason"));
    if matches!(text(error, "cause"), "udfUnavailable" | "missingUdf") {
        reason
            .replace("{name}", text(error, "symbol"))
            .replace("{module}", text(error, "module"))
            .replace("{line}", &error.get("line").and_then(Json::as_u64).unwrap_or(0).to_string())
    } else if text(error, "cause") == "vbaReadError" {
        format!("{reason}: {}", text(error, "detail"))
    } else if text(error, "symbol").is_empty() {
        reason.to_string()
    } else {
        format!("{reason}: {}", text(error, "symbol"))
    }
}

pub fn show(ui: &mut egui::Ui, d: &mut Dialog) {
    let empty = Json::Null;
    let data = d.result.as_ref().unwrap_or(&empty);
    if let Some(error) = data.get("error").filter(|e| !e.is_null()) {
        ui.label(tr(ui, text(error, "message")));
        ui.label(text(error, "reason"));
        return;
    }
    let modules = list(data, "modules");
    let procedures = list(data, "procedures");
    let yes_no = |value| tr(ui, if value { "Macro|Yes" } else { "Macro|No" });
    let summary = tr(ui, SUMMARY)
        .replace("{modules}", &modules.len().to_string())
        .replace("{procedures}", &procedures.len().to_string())
        .replace("{udfs}", &procedures.iter().filter(|p| p.get("udf").and_then(Json::as_bool) == Some(true)).count().to_string())
        .replace("{events}", yes_no(procedures.iter().any(|p| p.get("event").is_some_and(|e| !e.is_null()))))
        .replace("{signed}", yes_no(data.get("signed").and_then(Json::as_bool).unwrap_or(false)))
        .replace("{locked}", yes_no(data.get("locked").and_then(Json::as_bool).unwrap_or(false)));
    ui.strong(text(data, "name"));
    ui.label(summary);
    let list_height = (ui.ctx().content_rect().height() * 0.25).clamp(200.0, 240.0);
    let list_view = egui::ScrollArea::vertical()
        .id_salt("macro_procedures")
        .animated(false)
        .min_scrolled_height(list_height)
        .max_height(list_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, procedure) in procedures.iter().enumerate() {
                let label = format!("{} — {}", text(procedure, "name"), text(procedure, "module"));
                if ui.add(egui::Button::selectable(d.list_index == index, label).wrap()).clicked() {
                    d.list_index = index;
                    d.values.remove("code");
                }
                ui.horizontal_wrapped(|ui| {
                    let level = procedure.get("compatibility").and_then(|c| c.get("level")).and_then(Json::as_u64).unwrap_or(1);
                    let label = LEVELS.get(level.saturating_sub(1) as usize).copied().unwrap_or(LEVELS[0]);
                    level_badge(ui, level).on_hover_text(tr(ui, label));
                    if procedure.get("udf").and_then(Json::as_bool) == Some(true) {
                        ui.weak("UDF");
                    }
                    if procedure.get("event").is_some_and(|e| !e.is_null()) {
                        ui.weak(tr(ui, "Event — automatic execution is not supported"));
                    }
                    ui.add(egui::Label::new(format!("({})", parameters(ui, procedure))).wrap());
                });
            }
        });
    ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("gridcraft.macros.procedures.rect"), list_view.inner_rect));
    egui::ScrollArea::vertical().id_salt("macro_analysis").animated(false).max_height(160.0).show(ui, |ui| {
        for warning in list(data, "warnings") {
            ui.label(tr(ui, warning.as_str().unwrap_or_default()));
        }
        for script in list(data, "scripts") {
            ui.label(format!("{} · {}", text(script, "name"), script.get("steps").and_then(Json::as_u64).unwrap_or(0)));
        }
        if procedures.is_empty() && list(data, "scripts").is_empty() {
            ui.label(tr(ui, "This workbook has no scripts or macros."));
        }
        if let Some(procedure) = procedures.get(d.list_index) {
            ui.separator();
            ui.label(format!("{} · {}", text(procedure, "kind"), text(procedure, "scope")));
            let params = parameters(ui, procedure);
            ui.label(format!("{}: ({params})", tr(ui, "Parameters")));
            if let Some(returns) = procedure.get("returns").and_then(Json::as_str) {
                ui.label(format!("{}: {returns}", tr(ui, "Returns")));
            }
            if procedure.get("udf").and_then(Json::as_bool) == Some(true) {
                ui.label("UDF");
            }
            if let Some(event) = procedure.get("event").and_then(Json::as_str) {
                ui.label(format!("{}: {event}", tr(ui, "Event")));
            }
            if let Some(compatibility) = procedure.get("compatibility") {
                let level = compatibility.get("level").and_then(Json::as_u64).unwrap_or(1);
                let label = LEVELS.get(level.saturating_sub(1) as usize).copied().unwrap_or(LEVELS[0]);
                ui.strong(format!("{level} · {}", tr(ui, label)));
                for reason in list(compatibility, "reasons") {
                    ui.label(format!(
                        "{} {}: {}",
                        tr(ui, "Macro|Line"),
                        reason.get("line").and_then(Json::as_u64).unwrap_or(0),
                        text(reason, "detail")
                    ));
                }
            }
            for button in list(data, "buttons") {
                let assigned = text(button, "macro").rsplit('!').next().unwrap_or_default().trim_matches('\'');
                if assigned.eq_ignore_ascii_case(text(procedure, "name"))
                    || assigned.eq_ignore_ascii_case(&format!("{}.{}", text(procedure, "module"), text(procedure, "name")))
                {
                    ui.label(format!("{} · {} · {}", text(button, "sheet"), text(button, "caption"), button_kind(ui, button)));
                }
            }
            if ui.button(tr(ui, "View code")).clicked() {
                d.values.insert("code".into(), json!(true));
                d.values.remove("scrolled");
            }
        }
        if !list(data, "buttons").is_empty() {
            ui.separator();
            for button in list(data, "buttons") {
                ui.label(format!(
                    "{} · {} ({}) · {} → {} · {}",
                    text(button, "sheet"),
                    text(button, "caption"),
                    text(button, "name"),
                    button_kind(ui, button),
                    text(button, "macro"),
                    tr(ui, if button.get("exists").and_then(Json::as_bool) == Some(true) { "Found" } else { "Not found" })
                ));
            }
        }
        for reference in list(data, "references") {
            ui.label(format!("{} · {}", text(reference, "name"), text(reference, "libid")));
        }
    });
    if let Some(procedure) = procedures.get(d.list_index)
        && d.values.get("code").and_then(Json::as_bool) == Some(true)
        && let Some(module) = modules.iter().find(|m| text(m, "name") == text(procedure, "module"))
    {
        let line = procedure.get("line").and_then(Json::as_u64).unwrap_or(1);
        let scroll = !d.values.contains_key("scrolled");
        let source = text(module, "source");
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        let mut area =
            egui::ScrollArea::both().id_salt(("macro_source", d.list_index)).max_height(160.0).min_scrolled_height(160.0).auto_shrink([false, false]);
        if scroll {
            area = area.vertical_scroll_offset(line.saturating_sub(1) as f32 * (row_height + ui.spacing().item_spacing.y));
        }
        area.show_rows(ui, row_height, source.lines().count(), |ui, rows| {
            for (i, source) in source.lines().enumerate().skip(rows.start).take(rows.len()) {
                let selected = i as u64 + 1 == line;
                let color = crate::theme::Tokens::get(ui.ctx()).accent_soft;
                let mut label = egui::RichText::new(format!("{:>5}  {source}", i + 1)).monospace();
                if selected {
                    label = label.background_color(color);
                }
                ui.add(egui::Label::new(label).extend());
            }
        });
        d.values.insert("scrolled".into(), json!(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn macro_labels_and_diagnostics_exist_in_every_catalogue() {
        for language in Language::ALL.into_iter().filter(|l| *l != Language::En) {
            for key in [
                SUMMARY,
                LEVELS[0],
                LEVELS[1],
                LEVELS[2],
                "Macro|Yes",
                "Macro|No",
                "Form Control",
                "View code",
                "Parameters",
                "Returns",
                "Event",
                "Macro|Line",
                "Macro|Level {level}",
                "Optional",
                "Found",
                "Not found",
                gridcraft_engine::macros::UDF_UNAVAILABLE,
                gridcraft_engine::macros::UDF_MISSING,
                gridcraft_engine::macros::READ_ERROR,
            ] {
                assert_ne!(language.tr(key), key, "{language:?}: {key}");
            }
        }
        assert_eq!(Language::Vi.tr(LEVELS[3]), "ActiveX / UserForm");
        assert_eq!(Language::Vi.tr("Macro|Yes"), "Có");
        assert_eq!(Language::Vi.tr("Macro|No"), "Không");
    }

    #[test]
    fn macro_udf_diagnostic_text_is_natural_vietnamese() {
        let existing = json!({"cause": "udfUnavailable", "reason": gridcraft_engine::macros::UDF_UNAVAILABLE,
            "symbol": "X", "module": "Module1", "line": 10});
        assert_eq!(
            diagnostic_text(Language::Vi, &existing),
            "Hàm VBA tự định nghĩa `X` (Module1, dòng 10) — Bảng tính chưa chạy macro nên chưa tính được"
        );
        let missing = json!({"cause": "missingUdf", "reason": gridcraft_engine::macros::UDF_MISSING, "symbol": "X"});
        assert_eq!(diagnostic_text(Language::Vi, &missing), "Không tìm thấy hàm `X` trong macro của tệp");
    }
}
