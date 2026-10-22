#![cfg(not(target_arch = "wasm32"))]

//! Native update notices use a fake service and never contact a server or run installers.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use egui_kittest::{Harness, kittest::Queryable};
use gridcraft_ui_egui::{
    SheetApp,
    updates::{CheckOutcome, Offer, PreviousResult, RELEASES_PAGE, UpdateHook},
};

struct Fake {
    page_only: bool,
    windows_msi: bool,
    auto: AtomicBool,
    calls: AtomicUsize,
    forced: AtomicBool,
    skipped: Mutex<Option<String>>,
    downloads: AtomicUsize,
    installed: AtomicBool,
    installs: AtomicUsize,
    fail_download: AtomicBool,
    wait_download: AtomicBool,
    check_error: AtomicBool,
    up_to_date: AtomicBool,
    previous: Mutex<Option<PreviousResult>>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            page_only: false,
            windows_msi: false,
            auto: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
            forced: AtomicBool::new(false),
            skipped: Mutex::new(None),
            downloads: AtomicUsize::new(0),
            installed: AtomicBool::new(false),
            installs: AtomicUsize::new(0),
            fail_download: AtomicBool::new(false),
            wait_download: AtomicBool::new(false),
            check_error: AtomicBool::new(false),
            up_to_date: AtomicBool::new(false),
            previous: Mutex::new(None),
        }
    }
}

impl UpdateHook for Fake {
    fn check(&self, forced: bool) -> Result<CheckOutcome, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.forced.store(forced, Ordering::SeqCst);
        if self.check_error.load(Ordering::SeqCst) {
            return Err("test check failure".into());
        }
        if self.up_to_date.load(Ordering::SeqCst) {
            return Ok(CheckOutcome::UpToDate);
        }
        Ok(CheckOutcome::Available(Offer {
            version: "99.0.0".into(),
            notes_url: format!("{RELEASES_PAGE}/tag/erpcons-v99.0.0"),
            download_page_only: self.page_only,
            windows_msi: self.windows_msi,
        }))
    }
    fn download(&self, progress: &dyn Fn(u64, u64), cancel: &AtomicBool) -> Result<(), String> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        progress(5, 10);
        while self.wait_download.load(Ordering::SeqCst) && !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.fail_download.load(Ordering::SeqCst) {
            return Err("test download failure".into());
        }
        if cancel.load(Ordering::Relaxed) { Err("cancelled".into()) } else { Ok(()) }
    }
    fn install_and_restart(&self) -> Result<(), String> {
        self.installs.fetch_add(1, Ordering::SeqCst);
        self.installed.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn skip(&self, version: &str) -> Result<(), String> {
        *self.skipped.lock().unwrap() = Some(version.into());
        Ok(())
    }
    fn auto_check(&self) -> bool {
        self.auto.load(Ordering::SeqCst)
    }
    fn set_auto_check(&self, on: bool) -> Result<(), String> {
        self.auto.store(on, Ordering::SeqCst);
        Ok(())
    }
    fn take_previous_result(&self) -> Option<PreviousResult> {
        self.previous.lock().unwrap().take()
    }
}

fn app() -> SheetApp {
    let mut session = gridcraft_engine::Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Default::default());
    app.ui.language = gridcraft_ui_egui::i18n::Language::En;
    app.product_name = "Bảng tính – ERPCons";
    app
}

fn harness(fake: Arc<Fake>) -> Harness<'static, SheetApp> {
    let mut app = app();
    app.update_hook = Some(fake);
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    )
}

fn edit(app: &mut SheetApp) {
    app.run("cell.set", serde_json::json!({"cell":"A1", "input":"draft"})).unwrap();
}

fn wait_for(h: &mut Harness<'static, SheetApp>, label: &str) {
    for _ in 0..200 {
        h.run_steps(2);
        if h.query_by_label_contains(label).is_some() {
            h.run_steps(3);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.get_by_label_contains(label);
}

#[test]
fn available_at_start_later_hides_and_forced_check_restores_notice() {
    let fake = Arc::new(Fake::default());
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Bảng tính – ERPCons 99.0.0 is available");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert!(!fake.forced.load(Ordering::SeqCst));
    h.get_by_label("Remind me later").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("is available").is_none());
    h.state_mut().check_for_updates();
    wait_for(&mut h, "Bảng tính – ERPCons 99.0.0 is available");
    assert!(fake.forced.load(Ordering::SeqCst));
}

#[test]
fn skip_passes_version_and_hides_notice() {
    let fake = Arc::new(Fake::default());
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Skip this version");
    h.get_by_label("Skip this version").click();
    h.run_steps(3);
    assert_eq!(fake.skipped.lock().unwrap().as_deref(), Some("99.0.0"));
    assert!(h.query_by_label_contains("is available").is_none());
}

#[test]
fn download_page_only_opens_notes_without_downloading() {
    let fake = Arc::new(Fake { page_only: true, ..Fake::default() });
    let mut h = harness(fake.clone());
    let opened = Arc::new(Mutex::new(None));
    let urls = opened.clone();
    h.state_mut().services.open_url = Some(Box::new(move |url| *urls.lock().unwrap() = Some(url.to_string())));
    wait_for(&mut h, "Download new version");
    assert!(h.query_by_label("Update now").is_none());
    h.get_by_label("Download new version").click();
    h.run_steps(3);
    assert_eq!(opened.lock().unwrap().as_deref(), Some(format!("{RELEASES_PAGE}/tag/erpcons-v99.0.0").as_str()));
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 0);
}

#[test]
fn download_failure_offers_retry_and_success_launches_installer() {
    let fake = Arc::new(Fake::default());
    fake.fail_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "test download failure");
    h.get_by_label("Retry");
    assert!(!fake.installed.load(Ordering::SeqCst));
    fake.fail_download.store(false, Ordering::SeqCst);
    h.get_by_label("Retry").click();
    for _ in 0..400 {
        h.run_steps(2);
        if fake.installed.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(fake.installed.load(Ordering::SeqCst));
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);
}

#[test]
fn previous_results_are_shown_and_failure_retry_forces_check() {
    for (previous, label, failed) in [
        (PreviousResult::Installed { version: "1.2.3".into() }, "Updated to version 1.2.3", false),
        (PreviousResult::Failed { version: "1.2.3".into(), reason: "test reason".into() }, "Update to 1.2.3 failed: test reason", true),
    ] {
        let fake = Arc::new(Fake::default());
        fake.auto.store(false, Ordering::SeqCst);
        *fake.previous.lock().unwrap() = Some(previous);
        let mut h = harness(fake.clone());
        wait_for(&mut h, label);
        assert!(fake.previous.lock().unwrap().is_none());
        assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
        if failed {
            h.get_by_label("Retry").click();
            wait_for(&mut h, "is available");
            assert!(fake.forced.load(Ordering::SeqCst));
        }
    }
}

#[test]
fn unsaved_document_cancel_aborts_update_without_discarding_work() {
    let fake = Arc::new(Fake::default());
    let mut h = harness(fake.clone());
    edit(h.state_mut());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    h.run_steps(3);
    h.get_by_label_contains("Do you want to save the changes");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert!(h.state().session.documents().iter().any(|doc| doc.is_dirty()));
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 0);
    assert!(!fake.installed.load(Ordering::SeqCst));
}

#[test]
fn forced_checks_report_up_to_date_and_errors_when_auto_is_off() {
    let fake = Arc::new(Fake::default());
    fake.auto.store(false, Ordering::SeqCst);
    fake.up_to_date.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    h.run_steps(3);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    h.state_mut().check_for_updates();
    wait_for(&mut h, "Bảng tính – ERPCons is up to date.");
    fake.check_error.store(true, Ordering::SeqCst);
    h.state_mut().check_for_updates();
    wait_for(&mut h, "test check failure");
    h.get_by_label("Retry");
}

#[test]
fn cancel_download_keeps_app_open_and_never_installs() {
    let fake = Arc::new(Fake::default());
    fake.wait_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Cancel");
    h.get_by_label("Cancel").click();
    wait_for(&mut h, "Update now");
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 1);
    assert!(!fake.installed.load(Ordering::SeqCst));
}

#[test]
fn auto_check_checkbox_updates_the_service_preference() {
    let fake = Arc::new(Fake::default());
    fake.auto.store(false, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    h.run_steps(4);
    h.state_mut().run("ui.ribbonTab", serde_json::json!({"tab": "View"})).unwrap();
    h.run_steps(3);
    h.get_by_label("Automatically check for updates").click();
    h.run_steps(3);
    assert!(fake.auto.load(Ordering::SeqCst));
}

#[test]
fn update_notice_allows_document_shortcuts() {
    let mut h = harness(Arc::new(Fake::default()));
    edit(h.state_mut());
    wait_for(&mut h, "Update now");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);
    assert!(!h.state().session.documents().iter().any(|doc| doc.is_dirty()));
}

#[test]
fn update_notice_screenshots_in_both_themes() {
    let Some(dir) = std::env::var_os("GRIDCRAFT_UPDATE_SCREENSHOTS") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for theme in ["light", "dark"] {
        let mut h = harness(Arc::new(Fake::default()));
        h.state_mut().ui.language = gridcraft_ui_egui::i18n::Language::Vi;
        h.state_mut().ui.dark = theme == "dark";
        h.state_mut().ui.system_theme = false;
        wait_for(&mut h, "Đã có Bảng tính – ERPCons");
        h.render().unwrap().save(dir.join(format!("update-{theme}.png"))).unwrap();
    }
}

#[test]
fn edits_made_during_download_are_guarded_before_install() {
    let fake = Arc::new(Fake::default());
    fake.wait_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Cancel");
    edit(h.state_mut());
    fake.wait_download.store(false, Ordering::SeqCst);
    wait_for(&mut h, "Do you want to save the changes");
    assert!(!fake.installed.load(Ordering::SeqCst));
    h.get_by_label("Cancel").click();
    h.run_steps(5);
    assert!(h.state().session.documents().iter().any(|doc| doc.is_dirty()));
    assert!(h.state().dialog.is_none());
    assert!(!fake.installed.load(Ordering::SeqCst));
}

#[test]
fn discarding_unsaved_changes_allows_download() {
    let fake = Arc::new(Fake::default());
    fake.fail_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    edit(h.state_mut());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Do you want to save the changes");
    h.get_by_label("Don't Save").click();
    wait_for(&mut h, "test download failure");
    assert!(h.state().session.documents().iter().any(|doc| doc.is_dirty()));
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 1);
}

#[test]
fn windows_msi_ready_notice_explains_uac_before_installing() {
    let fake = Arc::new(Fake { windows_msi: true, ..Fake::default() });
    let mut h = harness(fake.clone());
    h.state_mut().ui.language = gridcraft_ui_egui::i18n::Language::Vi;
    wait_for(&mut h, "Cập nhật ngay");
    h.get_by_label("Cập nhật ngay").click();
    wait_for(&mut h, "Ứng dụng sẽ đóng, Windows sẽ hỏi quyền cài đặt — hãy chọn Yes — rồi ứng dụng tự mở lại");
    assert!(!fake.installed.load(Ordering::SeqCst));
}

#[test]
fn windows_msi_waits_for_click_and_installs_once() {
    let fake = Arc::new(Fake { windows_msi: true, ..Fake::default() });
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Install now");
    std::thread::sleep(std::time::Duration::from_millis(5100));
    h.run_steps(5);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
    h.get_by_label("Install now").click();
    h.get_by_label("Install now").click();
    h.run_steps(8);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 1);
}

#[test]
fn windows_msi_cancel_clears_ready_and_forced_check_downloads_again() {
    let fake = Arc::new(Fake { windows_msi: true, ..Fake::default() });
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Install now");
    h.get_by_label("Cancel").click();
    h.run_steps(5);
    assert!(h.query_by_label("Install now").is_none());
    h.state_mut().check_for_updates();
    wait_for(&mut h, "Update now");
    assert!(fake.forced.load(Ordering::SeqCst));
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Install now");
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
    drop(h);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
}

#[test]
fn windows_msi_install_click_guards_edits_made_during_download() {
    let fake = Arc::new(Fake { windows_msi: true, ..Fake::default() });
    fake.wait_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Cancel");
    edit(h.state_mut());
    fake.wait_download.store(false, Ordering::SeqCst);
    wait_for(&mut h, "Install now");
    assert!(h.state().dialog.is_none());
    h.get_by_label("Install now").click();
    wait_for(&mut h, "Do you want to save the changes");
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
    h.get_by_label("Cancel").click();
    h.run_steps(8);
    assert!(h.state().session.documents().iter().any(|doc| doc.is_dirty()));
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
}

#[test]
fn closing_app_while_msi_waits_never_installs() {
    let fake = Arc::new(Fake { windows_msi: true, ..Fake::default() });
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Install now");
    assert!(h.state_mut().confirm_quit());
    h.run_steps(8);
    drop(h);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
}

#[test]
fn non_msi_still_auto_installs_after_download() {
    let fake = Arc::new(Fake::default());
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "The app will close and reopen after updating");
    assert!(h.query_by_label("Install now").is_none());
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
    for _ in 0..400 {
        h.run_steps(2);
        if fake.installs.load(Ordering::SeqCst) > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(fake.installs.load(Ordering::SeqCst), 1);
}

#[test]
fn windows_msi_waiting_notice_screenshot() {
    let Some(dir) = std::env::var_os("GRIDCRAFT_UPDATE_SCREENSHOTS") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fake = Arc::new(Fake { windows_msi: true, ..Fake::default() });
    let mut h = harness(fake);
    h.state_mut().ui.language = gridcraft_ui_egui::i18n::Language::Vi;
    h.state_mut().ui.dark = false;
    h.state_mut().ui.system_theme = false;
    wait_for(&mut h, "Cập nhật ngay");
    h.get_by_label("Cập nhật ngay").click();
    wait_for(&mut h, "Cài đặt ngay");
    h.render().unwrap().save(dir.join("update-msi-waiting-light.png")).unwrap();
}

#[test]
fn every_update_label_is_translated_in_every_interface_language() {
    let keys = [
        "The update check stopped unexpectedly",
        "Could not start updater: {reason}",
        "Updated to version {version}",
        "Update to {version} failed: {reason}",
        "Bảng tính – ERPCons is up to date.",
        "Automatically check for updates",
        "Retry",
        "Close",
        "Bảng tính – ERPCons {version} is available",
        "Cancel",
        "Checking for a newer version…",
        "Download new version",
        "Update now",
        "Install now",
        "Remind me later",
        "Skip this version",
        "What's new",
        "The updater is busy; try again shortly",
        "The updater failed; restart the app and try again",
        "The app will close, Windows will ask for installation permission — choose Yes — then the app will reopen automatically",
        "The app will close and reopen after updating",
        "Check for updates",
        "Help",
        "Update network unavailable: {detail}",
        "Update server returned HTTP {detail}",
        "The update signature is invalid",
        "The update manifest is invalid: {detail}",
        "The downloaded update failed verification",
        "Could not write the update: {detail}",
        "This installation cannot update automatically",
        "Permission denied while updating; check folder permissions",
        "Update cancelled",
    ];
    for language in gridcraft_ui_egui::i18n::Language::ALL {
        if language == gridcraft_ui_egui::i18n::Language::En {
            continue;
        }
        for key in keys {
            assert_ne!(language.tr(key), key, "{}: {key}", language.code());
            for placeholder in ["{version}", "{reason}", "{detail}"] {
                assert_eq!(key.contains(placeholder), language.tr(key).contains(placeholder));
            }
        }
    }
}

#[test]
fn save_all_saves_every_workbook_before_downloading() {
    let fake = Arc::new(Fake::default());
    fake.fail_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    edit(h.state_mut());
    h.state_mut().run("file.new", serde_json::json!({})).unwrap();
    edit(h.state_mut());
    let saved = Arc::new(AtomicUsize::new(0));
    let writes = saved.clone();
    h.state_mut().services.download = Some(Box::new(move |_, _, report| {
        writes.fetch_add(1, Ordering::SeqCst);
        report.settle(gridcraft_ui_egui::Delivery::Delivered);
    }));
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Do you want to save the changes");
    h.get_by_label("Save All").click();
    wait_for(&mut h, "test download failure");
    assert_eq!(saved.load(Ordering::SeqCst), 2);
    assert!(!h.state().session.documents().iter().any(|doc| doc.is_dirty()));
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 1);
}

#[test]
fn cancelled_save_aborts_update_and_keeps_edits() {
    let fake = Arc::new(Fake::default());
    let mut h = harness(fake.clone());
    edit(h.state_mut());
    h.state_mut().services.download = Some(Box::new(|_, _, report| report.settle(gridcraft_ui_egui::Delivery::Cancelled)));
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Do you want to save the changes");
    h.get_by_label("Save All").click();
    h.run_steps(5);
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 0);
    assert!(!fake.installed.load(Ordering::SeqCst));
    assert!(h.state().session.documents().iter().any(|doc| doc.is_dirty()));
}

#[test]
fn update_commands_force_check_and_store_the_preference() {
    let fake = Arc::new(Fake::default());
    fake.auto.store(false, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    h.run_steps(4);
    assert_eq!(h.state_mut().run("app.autoCheckUpdates", serde_json::json!({})).unwrap()["on"], false);
    h.state_mut().run("app.autoCheckUpdates", serde_json::json!({"on": true})).unwrap();
    assert!(fake.auto.load(Ordering::SeqCst));
    h.state_mut().run("app.checkForUpdates", serde_json::json!({})).unwrap();
    wait_for(&mut h, "Update now");
    assert!(fake.forced.load(Ordering::SeqCst));
}
