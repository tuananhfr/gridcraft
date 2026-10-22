//! Native update notices. Network and installer operations belong to the desktop host.

pub const RELEASES_PAGE: &str = "https://github.com/tuananhfr/gridcraft/releases";

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver},
};

use crate::{SheetApp, close_many::AfterClose, i18n::Language, theme};

/// Display metadata only; the native adapter retains the verified offer and download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub notes_url: String,
    pub download_page_only: bool,
    pub windows_msi: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    Quiet,
    UpToDate,
    Available(Offer),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviousResult {
    Installed { version: String },
    Failed { version: String, reason: String },
}

/// Blocking native operations; errors are translated by the application adapter.
pub trait UpdateHook: Send + Sync {
    fn set_language(&self, _language: Language) {}
    fn check(&self, forced: bool) -> Result<CheckOutcome, String>;
    fn download(&self, progress: &dyn Fn(u64, u64), cancel: &AtomicBool) -> Result<(), String>;
    fn install_and_restart(&self) -> Result<(), String>;
    fn skip(&self, version: &str) -> Result<(), String>;
    fn auto_check(&self) -> bool;
    fn set_auto_check(&self, on: bool) -> Result<(), String>;
    fn take_previous_result(&self) -> Option<PreviousResult>;
}

enum Event {
    Checked(Result<CheckOutcome, String>, bool),
    Downloaded(Result<(), String>),
}

#[derive(Default)]
pub(crate) struct Updates {
    pub(crate) open: bool,
    ctx: Option<egui::Context>,
    pub(crate) awaiting_save: bool,
    pub(crate) continue_requested: bool,
    started: bool,
    queued_check: bool,
    offer: Option<Offer>,
    worker: Option<Receiver<Event>>,
    downloading: bool,
    ready: bool,
    ready_at: Option<f64>,
    previous_until: Option<f64>,
    message: Option<String>,
    retry_check: bool,
    retry_download: bool,
    previous: Option<(String, bool)>,
    cancel: Arc<AtomicBool>,
    received: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

impl SheetApp {
    /// Forced checks bypass the native service's throttle, preference and skipped version.
    pub fn check_for_updates(&mut self) {
        self.start_update_check(true);
    }

    fn start_update_check(&mut self, forced: bool) {
        let language = self.ui.language;
        let Some(hook) = self.update_hook.clone() else {
            self.open_update_url(RELEASES_PAGE);
            return;
        };
        if self.updates.worker.is_some() || self.updates.ready {
            self.updates.open |= forced;
            self.updates.queued_check |= forced && !self.updates.downloading && !self.updates.ready;
            return;
        }
        self.updates.open = forced;
        self.updates.offer = None;
        self.updates.message = None;
        self.updates.retry_check = false;
        self.updates.retry_download = false;
        let (tx, rx) = mpsc::channel();
        let ctx = self.updates.ctx.clone();
        hook.set_language(language);
        let spawned = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook.check(forced)))
                .unwrap_or_else(|_| Err(language.tr("The update check stopped unexpectedly").into()));
            let _ = tx.send(Event::Checked(result, forced));
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        match spawned {
            Ok(_) => self.updates.worker = Some(rx),
            Err(error) => {
                self.updates.message = Some(format_text(language.tr("Could not start updater: {reason}"), &[("reason", &error.to_string())]));
                self.updates.retry_check = true;
                self.updates.open = forced;
            }
        }
    }

    pub(crate) fn poll_updates(&mut self, ctx: &egui::Context) {
        let language = self.ui.language;
        self.updates.ctx = Some(ctx.clone());
        if let Some(hook) = &self.update_hook {
            hook.set_language(language);
        }
        if std::mem::take(&mut self.updates.continue_requested) {
            self.continue_update(ctx);
        }
        if self.updates.awaiting_save && self.dialog.is_none() && !self.saving_all() {
            self.updates.awaiting_save = false;
            self.cancel_update_prompt();
        }
        if !self.updates.started {
            self.updates.started = true;
            if let Some(hook) = self.update_hook.clone() {
                if let Some(previous) = hook.take_previous_result() {
                    self.updates.previous = Some(match previous {
                        PreviousResult::Installed { version } => {
                            self.updates.previous_until = Some(crate::now_ms() + 6000.0);
                            (format_text(language.tr("Updated to version {version}"), &[("version", &version)]), false)
                        }
                        PreviousResult::Failed { version, reason } => {
                            (format_text(language.tr("Update to {version} failed: {reason}"), &[("version", &version), ("reason", &reason)]), true)
                        }
                    });
                }
                if hook.auto_check() {
                    self.start_update_check(false);
                }
            }
        }
        let Some(rx) = &self.updates.worker else { return };
        let event = match rx.try_recv() {
            Ok(event) => event,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.updates.worker = None;
                self.updates.message = Some(language.tr("The update check stopped unexpectedly").into());
                self.updates.retry_download = self.updates.downloading;
                self.updates.retry_check = !self.updates.downloading;
                self.updates.downloading = false;
                self.updates.open = true;
                return;
            }
        };
        self.updates.worker = None;
        if self.updates.queued_check && matches!(event, Event::Checked(..)) {
            self.updates.queued_check = false;
            self.start_update_check(true);
            return;
        }
        match event {
            Event::Checked(Ok(CheckOutcome::Available(offer)), _) => {
                self.updates.offer = Some(offer);
                self.updates.open = true;
            }
            Event::Checked(Ok(CheckOutcome::UpToDate | CheckOutcome::Quiet), forced) => {
                self.updates.open = forced;
                if forced {
                    self.updates.message = Some(language.tr("Bảng tính – ERPCons is up to date.").into());
                }
            }
            Event::Checked(Err(error), forced) => {
                self.updates.open = forced;
                self.updates.message = Some(error);
                self.updates.retry_check = true;
            }
            Event::Downloaded(result) => {
                self.updates.downloading = false;
                if self.updates.cancel.load(Ordering::Relaxed) {
                    self.updates.message = None;
                    self.updates.ready = false;
                } else {
                    match result {
                        Ok(()) => {
                            self.updates.ready = true;
                            self.updates.ready_at =
                                if self.updates.offer.as_ref().is_some_and(|offer| offer.windows_msi) { None } else { Some(crate::now_ms() + 800.0) };
                            let key = if self.updates.offer.as_ref().is_some_and(|offer| offer.windows_msi) {
                                "The app will close, Windows will ask for installation permission — choose Yes — then the app will reopen automatically"
                            } else {
                                "The app will close and reopen after updating"
                            };
                            self.updates.message = Some(language.tr(key).into());
                        }
                        Err(error) => {
                            self.updates.message = Some(error);
                            self.updates.retry_download = true;
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn cancel_update_prompt(&mut self) {
        self.updates.ready_at = None;
        self.updates.ready = false;
        self.updates.message = None;
    }

    pub(crate) fn request_update(&mut self, ctx: &egui::Context) {
        ctx.request_repaint();
        if self.printing.busy() || self.dialog.is_some() || self.updates.awaiting_save {
            return;
        }
        self.updates.awaiting_save = true;
        let uids = self.session.documents().iter().map(|doc| doc.uid).collect();
        self.close_documents(uids, AfterClose::Update);
    }

    pub(crate) fn continue_update(&mut self, ctx: &egui::Context) {
        let Some(hook) = self.update_hook.clone() else { return };
        if self.updates.ready {
            self.updates.ready = false;
            match hook.install_and_restart() {
                Ok(()) => {
                    self.updates.open = false;
                    self.quit_confirmed = true;
                    self.quit_requested = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(error) => {
                    self.updates.ready = false;
                    self.updates.message = Some(error);
                    self.updates.retry_download = true;
                }
            }
            return;
        }
        if self.updates.worker.is_some() || self.updates.offer.is_none() {
            return;
        }
        self.updates.message = None;
        self.updates.retry_download = false;
        self.updates.downloading = true;
        self.updates.cancel = Arc::new(AtomicBool::new(false));
        self.updates.received.store(0, Ordering::Relaxed);
        self.updates.total.store(0, Ordering::Relaxed);
        let cancel = self.updates.cancel.clone();
        let received = self.updates.received.clone();
        let total = self.updates.total.clone();
        let ctx = ctx.clone();
        let language = self.ui.language;
        hook.set_language(language);
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("update-download".into()).spawn(move || {
            let progress = |bytes, size| {
                received.store(bytes, Ordering::Relaxed);
                total.store(size, Ordering::Relaxed);
                ctx.request_repaint();
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook.download(&progress, &cancel)))
                .unwrap_or_else(|_| Err(language.tr("The update check stopped unexpectedly").into()));
            let _ = tx.send(Event::Downloaded(result));
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.updates.worker = Some(rx),
            Err(error) => {
                self.updates.downloading = false;
                self.updates.message = Some(format_text(language.tr("Could not start updater: {reason}"), &[("reason", &error.to_string())]));
                self.updates.retry_download = true;
            }
        }
    }

    fn skip_update(&mut self) {
        if let Some(hook) = &self.update_hook
            && let Some(offer) = &self.updates.offer
        {
            match hook.skip(&offer.version) {
                Ok(()) => self.updates.open = false,
                Err(error) => self.updates.message = Some(error),
            }
        }
    }
}

pub(crate) fn preferences(ui: &mut egui::Ui, app: &mut SheetApp) {
    let language = app.ui.language;
    if let Some(hook) = app.update_hook.clone() {
        let mut on = hook.auto_check();
        if ui.add_enabled(app.updates.worker.is_none(), egui::Checkbox::new(&mut on, language.tr("Automatically check for updates"))).changed() {
            app.run_or_alert("app.autoCheckUpdates", serde_json::json!({"on": on}));
        }
    }
}

pub(crate) fn dialog(app: &mut SheetApp, ctx: &egui::Context) {
    let language = app.ui.language;
    if let Some(until) = app.updates.previous_until {
        let remaining = std::time::Duration::from_millis((until - crate::now_ms()).max(0.0).ceil() as u64);
        if remaining.is_zero() {
            app.updates.previous = None;
            app.updates.previous_until = None;
        } else {
            ctx.request_repaint_after(remaining);
        }
    }
    if !app.updates.open && app.updates.previous.is_none() {
        return;
    }
    let mut later = false;
    let mut skip = false;
    let mut update = false;
    let mut retry = false;
    let mut notes = None;
    let mut dismiss_previous = false;
    let t = theme::Tokens::get(ctx);
    egui::Area::new(egui::Id::new("native-update-notice"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width((ctx.content_rect().width() - 48.0).clamp(120.0, 420.0));
                if let Some((message, failed)) = &app.updates.previous {
                    ui.label(message);
                    ui.horizontal_wrapped(|ui| {
                        if *failed && ui.button(language.tr("Retry")).clicked() {
                            retry = true;
                            dismiss_previous = true;
                        }
                        dismiss_previous |= ui.button(language.tr("Close")).clicked();
                    });
                }
                if !app.updates.open {
                    return;
                }
                if let Some(offer) = &app.updates.offer {
                    ui.label(
                        egui::RichText::new(format_text(language.tr("Bảng tính – ERPCons {version} is available"), &[("version", &offer.version)]))
                            .strong(),
                    );
                }
                if let Some(message) = &app.updates.message {
                    ui.label(egui::RichText::new(message).color(t.text));
                }
                if app.updates.downloading {
                    let received = app.updates.received.load(Ordering::Relaxed);
                    let total = app.updates.total.load(Ordering::Relaxed);
                    let fraction = if total == 0 { 0.0 } else { (received as f64 / total as f64).clamp(0.0, 1.0) as f32 };
                    ui.add(egui::ProgressBar::new(fraction).show_percentage());
                    if ui.button(language.tr("Cancel")).clicked() {
                        app.updates.cancel.store(true, Ordering::Relaxed);
                    }
                } else if app.updates.worker.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(language.tr("Checking for a newer version…"));
                    });
                } else {
                    ui.horizontal_wrapped(|ui| {
                        if app.updates.retry_check || app.updates.retry_download {
                            retry = ui.button(language.tr("Retry")).clicked();
                            later = ui.button(language.tr("Close")).clicked();
                        } else if app.updates.ready {
                            if app.updates.ready_at.is_none() {
                                if app.updates.offer.as_ref().is_some_and(|offer| offer.windows_msi) {
                                    if !app.updates.awaiting_save {
                                        update = ui
                                            .add(
                                                egui::Button::new(egui::RichText::new(language.tr("Install now")).color(t.window).strong())
                                                    .fill(t.accent)
                                                    .corner_radius(16),
                                            )
                                            .clicked();
                                        later = ui.button(language.tr("Cancel")).clicked();
                                    }
                                } else {
                                    update = ui.button(language.tr("Retry")).clicked();
                                }
                            }
                        } else if let Some(offer) = &app.updates.offer {
                            if offer.download_page_only {
                                if ui.button(language.tr("Download new version")).clicked() {
                                    notes = Some(offer.notes_url.clone());
                                }
                            } else {
                                update = ui
                                    .add(
                                        egui::Button::new(egui::RichText::new(language.tr("Update now")).color(t.window).strong())
                                            .fill(t.accent)
                                            .corner_radius(16),
                                    )
                                    .clicked();
                            }
                            later = ui.button(language.tr("Remind me later")).clicked();
                            skip = ui.button(language.tr("Skip this version")).clicked();
                        } else if !app.updates.ready {
                            later = ui.button(language.tr("Close")).clicked();
                        }
                    });
                    if let Some(offer) = &app.updates.offer
                        && !app.updates.ready
                        && ui.link(language.tr("What's new")).clicked()
                    {
                        notes = Some(offer.notes_url.clone());
                    }
                }
            });
        });
    if dismiss_previous {
        app.updates.previous = None;
    }
    if let Some(url) = notes {
        app.open_update_url(&url);
    }
    if later {
        if app.updates.ready {
            app.cancel_update_prompt();
        }
        app.updates.open = false;
    }
    if skip {
        app.skip_update();
    }
    if retry {
        if app.updates.retry_download && !dismiss_previous {
            app.request_update(ctx);
        } else {
            app.start_update_check(true);
        }
    }
    if update {
        app.request_update(ctx);
    }
    // New edits made during download must pass the save guard too.
    if app.updates.ready
        && !app.updates.awaiting_save
        && let Some(at) = app.updates.ready_at
    {
        let remaining = std::time::Duration::from_millis((at - crate::now_ms()).max(0.0).ceil() as u64);
        if remaining.is_zero() {
            app.updates.ready_at = None;
            app.request_update(ctx);
        } else {
            ctx.request_repaint_after(remaining);
        }
    }
}

fn format_text(template: &str, values: &[(&str, &str)]) -> String {
    values.iter().fold(template.to_string(), |text, (key, value)| text.replace(&format!("{{{key}}}"), value))
}

impl SheetApp {
    fn open_update_url(&self, url: &str) {
        if let Some(open) = &self.services.open_url {
            open(url);
        } else if let Some(ctx) = &self.updates.ctx {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }
}
