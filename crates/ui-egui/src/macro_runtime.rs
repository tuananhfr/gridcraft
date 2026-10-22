use crate::{SheetApp, dialogs::Dialog, widgets::tr};
use gridcraft_engine::macro_host::{MacroOptions, MacroTask};
use serde_json::{Value as Json, json};

#[derive(Default)]
pub(crate) struct State {
    task: Option<MacroTask>,
    consent: Option<(u64, String)>,
    prompt: Option<(String, Json)>,
    input: String,
    feedback: Option<Json>,
}

fn text<'a>(value: &'a Json, key: &str) -> &'a str {
    value.get(key).and_then(Json::as_str).unwrap_or_default()
}

fn begin(app: &mut SheetApp, name: String) {
    if app.macro_runtime.task.is_some() {
        return;
    }
    let Ok(doc) = app.session.doc() else { return };
    let uid = doc.uid;
    let previous_run = doc.macro_state.log.last().cloned();
    let options = MacroOptions { wall_ms: u64::from(app.ui.macro_time_limit_seconds.clamp(1, 300)) * 1000, ..Default::default() };
    match app.session.start_macro_interactive(&name, options) {
        Ok(task) => {
            app.macro_runtime.feedback = None;
            app.macro_runtime.task = Some(task);
        }
        Err(error) => {
            if error.to_string().contains("Macro consent is required") {
                // Consume only the consent request; the Macro dialog remains open behind the modal.
                let requests = app.session.take_ui_requests();
                app.session
                    .ui_requests
                    .extend(requests.into_iter().filter(|r| !matches!(r, gridcraft_engine::UiRequest::Dialog(n, _) if n == "macro.consent")));
                app.macro_runtime.consent = Some((uid, name));
                return;
            }
            let entry = app.session.doc().ok().and_then(|d| d.macro_state.log.last()).filter(|entry| Some(*entry) != previous_run.as_ref()).cloned();
            app.macro_runtime.feedback = Some(entry.unwrap_or_else(|| json!({"error":{"message":error.to_string()}})));
        }
    }
}

pub(crate) fn step(app: &mut SheetApp, ctx: &egui::Context) {
    let Some(mut task) = app.macro_runtime.task.take() else { return };
    match app.session.step_macro(&mut task) {
        Ok(Some(summary)) => {
            app.macro_runtime.prompt = None;
            if summary.get("rolledBack").and_then(Json::as_bool) == Some(true) {
                app.macro_runtime.feedback = Some(summary);
            }
            app.after_engine();
        }
        Ok(None) => {
            if let Some((name, params)) = task.take_prompt() {
                app.macro_runtime.input = text(&params, "default").to_string();
                app.macro_runtime.prompt = Some((name, params));
            }
            app.macro_runtime.task = Some(task);
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        Err(error) => {
            app.macro_runtime.prompt = None;
            app.macro_runtime.feedback = Some(json!({"rolledBack":true,"error":{"message":error.to_string()}}));
        }
    }
}

fn running_controls(app: &mut SheetApp, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.label(tr(ui, "Macro running"));
        if ui.button(tr(ui, "Stop")).clicked()
            && let Some(task) = &app.macro_runtime.task
        {
            task.cancel();
        }
    });
}

fn entry(ui: &mut egui::Ui, value: &Json) {
    ui.label(text(value, "macro"));
    let status = match text(value, "status") {
        "completed" => "Macro|Completed",
        "cancelled" => "Macro|Cancelled",
        "refused" => "Macro refused",
        "failed" => "Macro failed",
        _ => "Macro running",
    };
    ui.strong(tr(ui, status));
    for (key, label) in [("start", "Macro|Start"), ("end", "Macro|End"), ("steps", "Steps"), ("cellsChanged", "Cells changed")] {
        if let Some(value) = value.get(key).filter(|v| !v.is_null()) {
            ui.label(format!("{}: {value}", tr(ui, label)));
        }
    }
    if value.get("rolledBack").and_then(Json::as_bool) == Some(true) {
        ui.label(tr(ui, "Macro changes were rolled back; the workbook is unchanged."));
    }
    if let Some(error) = value.get("error").filter(|e| !e.is_null()) {
        let runtime_level = text(error, "message").strip_prefix("Unsupported macro level ").and_then(|s| s.split_once(": "));
        if let Some((level, reason)) = runtime_level {
            ui.label(tr(ui, "Compatibility level {level} is not supported.").replace("{level}", level));
            ui.label(reason);
        } else if let Some(level) = error.get("level") {
            ui.label(tr(ui, "Compatibility level {level} is not supported.").replace("{level}", &level.to_string()));
        } else {
            ui.label(tr(ui, "Macro execution stopped."));
        }
        ui.label(format!("{} · {} {}", text(error, "module"), tr(ui, "Macro|Line"), error.get("line").unwrap_or(&Json::Null)));
        ui.label(tr(ui, text(error, "message")));
        if let Some(reasons) = error.get("reasons").and_then(Json::as_array) {
            for reason in reasons {
                ui.label(format!("{} {}: {}", tr(ui, "Macro|Line"), reason.get("line").unwrap_or(&Json::Null), text(reason, "detail")));
            }
        }
    }
    if let Some(messages) = value.get("messages").and_then(Json::as_array) {
        for message in messages {
            ui.label(tr(ui, message.as_str().unwrap_or_default()));
        }
    }
}

pub(crate) fn controls(app: &mut SheetApp, ui: &mut egui::Ui, d: &Dialog) {
    if app.macro_runtime.task.is_some() {
        running_controls(app, ui);
    } else {
        let procedure = d.result.as_ref().and_then(|v| v.get("procedures")).and_then(Json::as_array).and_then(|p| p.get(d.list_index));
        let runnable =
            procedure.is_some_and(|p| p.get("runnableShape").and_then(Json::as_bool) == Some(true) && p.get("event").is_none_or(Json::is_null));
        if ui.add_enabled(runnable, egui::Button::new(tr(ui, "Macro|Run"))).clicked()
            && let Some(p) = procedure
        {
            begin(app, format!("{}.{}", text(p, "module"), text(p, "name")));
        }
    }
    if app.session.doc().ok().is_some_and(|d| d.macro_state.consent) && ui.button(tr(ui, "Revoke macro consent")).clicked() {
        app.run_or_alert("macros.consent", json!({"allow":false}));
    }
    egui::CollapsingHeader::new(tr(ui, "Macro buttons")).show(ui, |ui| {
        if let Some(buttons) = d.result.as_ref().and_then(|v| v.get("buttons")).and_then(Json::as_array) {
            for button in buttons {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{} · {}", text(button, "sheet"), text(button, "caption")));
                    let enabled = app.macro_runtime.task.is_none() && button.get("exists").and_then(Json::as_bool) == Some(true);
                    if ui.add_enabled(enabled, egui::Button::new(tr(ui, "Macro|Run"))).clicked() {
                        begin(app, text(button, "macro").to_string());
                    }
                });
            }
        }
    });
    egui::CollapsingHeader::new(tr(ui, "Run log")).show(ui, |ui| {
        let runs = app.session.doc().map(|d| d.macro_state.log.clone()).unwrap_or_default();
        egui::ScrollArea::vertical().id_salt("macro_run_log").max_height(140.0).show(ui, |ui| {
            for run in runs.iter().rev() {
                entry(ui, run);
                ui.separator();
            }
        });
    });
}

pub(crate) fn modals(app: &mut SheetApp, ctx: &egui::Context) {
    let lang = app.ui.language;
    if let Some((uid, name)) = app.macro_runtime.consent.clone() {
        if app.session.doc().ok().is_none_or(|d| d.uid != uid) {
            app.macro_runtime.consent = None;
            return;
        }
        let mut allow = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("macro_consent")).show(ctx, |ui| {
            ui.set_max_width((ctx.content_rect().width() - 48.0).clamp(220.0, 420.0));
            ui.heading(tr(ui, "Allow this macro?"));
            ui.label(&name);
            ui.label(tr(ui, "This level-1 macro can change cells. Consent lasts for this document session and can be revoked."));
            ui.label(tr(ui, "Event — automatic execution is not supported"));
            ui.horizontal_wrapped(|ui| {
                allow = ui.button(tr(ui, "Allow and run")).clicked();
                cancel = ui.button(tr(ui, "Cancel")).clicked();
            });
        });
        if allow {
            app.macro_runtime.consent = None;
            if app.run("macros.consent", json!({"allow":true})).is_ok() {
                begin(app, name);
            }
        } else if cancel {
            app.macro_runtime.consent = None;
        }
        return;
    }
    if let Some((kind, params)) = app.macro_runtime.prompt.clone() {
        let mut answer = None;
        egui::Modal::new(egui::Id::new("macro_prompt")).show(ctx, |ui| {
            ui.set_max_width((ctx.content_rect().width() - 48.0).clamp(220.0, 420.0));
            ui.heading(if text(&params, "title").is_empty() { lang.tr("Macros") } else { text(&params, "title") });
            ui.label(text(&params, "prompt"));
            if kind == "macro.inputBox" {
                ui.add(egui::TextEdit::singleline(&mut app.macro_runtime.input).desired_width(ui.available_width()).char_limit(65536));
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr(ui, "OK")).clicked() {
                        answer = Some(json!(app.macro_runtime.input));
                    }
                    if ui.button(tr(ui, "Cancel")).clicked() {
                        answer = Some(json!(""));
                    }
                });
            } else if kind == "macro.filePicker" {
                ui.label(tr(ui, "GetOpenFilename returns False until file bindings are supported"));
                if ui.button(tr(ui, "Choose file…")).clicked() {
                    if let Some(pick) = &app.services.pick_open {
                        let _ = pick();
                    }
                    answer = Some(json!(false));
                }
                if ui.button(tr(ui, "Cancel")).clicked() {
                    answer = Some(json!(false));
                }
            } else {
                let buttons = params.get("buttons").and_then(Json::as_i64).unwrap_or(0) & 7;
                let choices: &[(i32, &str)] = match buttons {
                    1 => &[(1, "OK"), (2, "Cancel")],
                    2 => &[(3, "Macro|Abort"), (4, "Retry"), (5, "Macro|Ignore")],
                    3 => &[(6, "Macro|Yes"), (7, "Macro|No"), (2, "Cancel")],
                    4 => &[(6, "Macro|Yes"), (7, "Macro|No")],
                    5 => &[(4, "Retry"), (2, "Cancel")],
                    _ => &[(1, "OK")],
                };
                ui.horizontal_wrapped(|ui| {
                    for (value, label) in choices {
                        if ui.button(tr(ui, label)).clicked() {
                            answer = Some(json!(value));
                        }
                    }
                });
            }
            if ui.button(tr(ui, "Stop")).clicked()
                && let Some(task) = &app.macro_runtime.task
            {
                task.cancel();
            }
        });
        if let Some(answer) = answer {
            if let Some(task) = &mut app.macro_runtime.task {
                task.answer_prompt(answer);
            }
            app.macro_runtime.prompt = None;
        }
        return;
    }
    if let Some(feedback) = app.macro_runtime.feedback.clone() {
        let mut close = false;
        egui::Modal::new(egui::Id::new("macro_feedback")).show(ctx, |ui| {
            ui.set_max_width((ctx.content_rect().width() - 48.0).clamp(220.0, 420.0));
            ui.heading(tr(ui, "Macros"));
            entry(ui, &feedback);
            close = ui.button(tr(ui, "OK")).clicked();
        });
        if close {
            app.macro_runtime.feedback = None;
        }
    } else if app.macro_runtime.task.is_some() && app.dialog.as_ref().is_none_or(|d| d.name != "macros") {
        egui::Window::new(lang.tr("Macros")).collapsible(false).show(ctx, |ui| running_controls(app, ui));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn macro_runtime_messages_have_all_catalogues() {
        for lang in crate::i18n::Language::ALL.into_iter().filter(|l| *l != crate::i18n::Language::En) {
            for key in [
                crate::VBA_NOTICE,
                "Macro time limit (seconds)",
                "Event — automatic execution is not supported",
                "Macro running",
                "Revoke macro consent",
                "Macro buttons",
                "Run log",
                "Macro refused",
                "Macro failed",
                "Cells changed",
                "Steps",
                "Allow this macro?",
                "This level-1 macro can change cells. Consent lasts for this document session and can be revoked.",
                "Allow and run",
                "Macro changes were rolled back; the workbook is unchanged.",
                "Compatibility level {level} is not supported.",
                "Macro execution stopped.",
                "GetOpenFilename returns False until file bindings are supported",
                "Choose file…",
                "This macro compatibility level is not supported",
                "Macro|Run",
                "Macro|Completed",
                "Macro|Cancelled",
                "Macro|Start",
                "Macro|End",
                "Macro|Abort",
                "Macro|Ignore",
            ] {
                assert_ne!(lang.tr(key), crate::i18n::Language::En.tr(key), "{lang:?}: {key}");
            }
        }
    }
}
