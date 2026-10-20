//! The browser shell: web `Services`, drag-and-drop, and the eframe web runner.

use std::cell::Cell;
use std::rc::Rc;

use gridcraft_engine::Session;
use gridcraft_ui_egui::{Delivery, DeliveryReport, Inbox, Services, SheetApp};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;

const CANVAS_ID: &str = "gridcraft_canvas";
const LOADING_ID: &str = "gridcraft_loading";
pub(crate) const SAVE_FAILED: &str = "The browser could not save the file. Please try again.";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let mut options = eframe::WebOptions::default();
        if query().contains("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        // `?host=parent`: a same-origin page framing GridCraft (such as a Nextcloud app) opens
        // workbooks in it and stores what it saves (`host`).
        let bridge = if query_param("host").as_deref() == Some("parent") { crate::host::Bridge::new() } else { None };
        let app_bridge = bridge.clone();
        let dirty = Rc::new(Cell::new(false));
        // In host mode the parent hears `gridcraft:dirty` and guards leaving the page itself.
        if bridge.is_none()
            && let Err(e) = guard_unload(dirty.clone())
        {
            log::error!("no unsaved-changes guard: {e}");
        }
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    let inbox: Inbox = Inbox::default();
                    let mut session = Session::new();
                    // In host mode the parent sends the workbook to edit; no sample meanwhile.
                    let q = if app_bridge.is_some() { String::new() } else { query() };
                    if let Some(name) = q.split(['?', '&']).find_map(|kv| kv.strip_prefix("sample=")) {
                        let _ = session.execute("file.new", serde_json::json!({"sample": name}));
                    } else if q.contains("sample") {
                        let _ = session.execute("file.new", serde_json::json!({"sample": "sales"}));
                    }
                    let clipboard = crate::clipboard::ClipboardBridge::install(cc.egui_ctx.clone()).ok();
                    let mut services = services(inbox.clone(), cc.egui_ctx.clone());
                    if let Some(clipboard) = &clipboard {
                        clipboard.connect(&mut services);
                    }
                    if let Some(bridge) = &app_bridge {
                        bridge.connect(&mut services, inbox.clone(), cc.egui_ctx.clone());
                    }
                    let mut app = SheetApp::new(session, services);
                    if clipboard.is_none() {
                        app.toast = Some(("The browser cannot read the clipboard — use Ctrl+V".into(), gridcraft_ui_egui::now_ms()));
                    }
                    app.ui.system_theme = true;
                    app.ui.language = crate::language::initial();
                    app.ui.region = crate::region::initial();
                    let language = app.ui.language;
                    let region = app.ui.region;
                    SheetApp::setup_context_for_language(&cc.egui_ctx, false, app.ui.language);
                    if let Some(bridge) = &app_bridge {
                        if let Some(author) = query_param("author") {
                            crate::host::Bridge::set_author(&mut app, &author);
                        }
                        bridge.post_ready();
                    }
                    // Drafts are the office shell's; an embedding parent stores what it opened itself.
                    let drafts = app_bridge.is_none().then(|| crate::drafts::Drafts::install(&cc.egui_ctx));
                    Ok(Box::new(WebShell { app, inbox, bridge: app_bridge, clipboard, dirty, drafts, language, region }))
                }),
            )
            .await;
        if let (Err(e), Some(bridge)) = (&result, &bridge) {
            bridge.post_failed(&format!("{e:?}"));
        }
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>GridCraft failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
            }
        }
    });
}

fn query() -> String {
    web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
}

fn query_param(key: &str) -> Option<String> {
    web_sys::UrlSearchParams::new_with_str(&query()).ok()?.get(key)
}

struct WebShell {
    app: SheetApp,
    inbox: Inbox,
    bridge: Option<crate::host::Bridge>,
    clipboard: Option<crate::clipboard::ClipboardBridge>,
    /// Whether any open workbook has unsaved changes, for the `beforeunload` guard.
    dirty: Rc<Cell<bool>>,
    /// Draft autosave to the office shell's `window.erpDrafts`; `None` in host mode.
    drafts: Option<crate::drafts::Drafts>,
    /// The last interface language written to the site-wide preference.
    language: gridcraft_ui_egui::i18n::Language,
    /// The last regional format choice written to its preference (`None` follows the language).
    region: Option<gridcraft_ui_egui::region::Region>,
}

impl eframe::App for WebShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        for f in dropped {
            let inbox = self.inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped.xlsx".into());
                match f.bytes_async().await {
                    Ok(bytes) => {
                        inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                        ctx.request_repaint();
                    }
                    Err(e) => log::error!("couldn't read dropped file {name}: {e}"),
                }
            });
        }
        if let Some(drafts) = &mut self.drafts {
            drafts.open_restored(&mut self.app);
        }
        self.app.logic(ctx);
        if let Some(clipboard) = &self.clipboard {
            clipboard.show_notice(&mut self.app);
        }
        if self.app.ui.language != self.language {
            self.language = self.app.ui.language;
            crate::language::remember(self.language);
        }
        if self.app.ui.region != self.region {
            self.region = self.app.ui.region;
            crate::region::remember(self.region);
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(clipboard) = &self.clipboard {
            clipboard.raw_input(raw, &mut self.app);
        }
        self.app.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
        if let Some(bridge) = &self.bridge {
            bridge.sync(&mut self.app);
        }
        if let Some(drafts) = &mut self.drafts {
            drafts.after_frame(&self.app, ui.ctx());
        }
        self.dirty.set(self.app.session.documents().iter().any(|d| d.is_dirty()));
    }
}

/// Closing, reloading or leaving the tab with unsaved changes asks first. Browsers show their own
/// confirmation (a page can only ask for it), so there is no Save button: Cancel, then File › Save.
fn guard_unload(dirty: Rc<Cell<bool>>) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let on_unload = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(move |e: web_sys::BeforeUnloadEvent| {
        if dirty.get() {
            e.prevent_default();
            // Older browsers ask only when `returnValue` is set.
            e.set_return_value("unsaved changes");
        }
    });
    window.add_event_listener_with_callback("beforeunload", on_unload.as_ref().unchecked_ref()).map_err(|e| format!("{e:?}"))?;
    // The listener lives as long as the page.
    on_unload.forget();
    Ok(())
}

fn services(inbox: Inbox, ctx: egui::Context) -> Services {
    let open_inbox = inbox.clone();
    let picture_ctx = ctx.clone();
    let save_ctx = ctx.clone();
    Services {
        open_async: Some(Box::new(move || {
            let inbox = open_inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = rfd::AsyncFileDialog::new()
                    .add_filter("Spreadsheets", &["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv", "txt", "json"])
                    .pick_file()
                    .await
                else {
                    return;
                };
                let bytes = file.read().await;
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push((file.file_name(), bytes));
                ctx.request_repaint();
            });
        })),
        pick_picture_async: Some(Box::new(move |inbox| {
            let ctx = picture_ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = rfd::AsyncFileDialog::new().add_filter("Pictures (PNG, JPEG)", &["png", "jpg", "jpeg"]).pick_file().await else {
                    return;
                };
                let bytes = file.read().await;
                inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((file.file_name(), bytes));
                ctx.request_repaint();
            });
        })),
        download: Some(Box::new(move |name: &str, bytes: &[u8], report: DeliveryReport| {
            if let Err(e) = download(name, bytes, report.clone(), save_ctx.clone()) {
                log::error!("download of {name} failed: {e}");
                report.settle(Delivery::Failed(SAVE_FAILED.into()));
            }
        })),
        print: Some(Box::new(|bytes: &[u8]| {
            if let Err(e) = print_pdf(bytes) {
                log::error!("print failed: {e}");
            }
        })),
        open_url: Some(Box::new(|url: &str| {
            if let Some(w) = web_sys::window()
                && let gridcraft_engine::links::LinkKind::External(url) = gridcraft_engine::links::classify(url)
            {
                let _ = w.open_with_url_and_target(&url, "_blank");
            }
        })),
        inbox: Some(inbox),
        editor_closed: Some(Box::new(crate::closed::notify)),
        ..Default::default()
    }
}

/// Opens `bytes` (a PDF) as an object URL in a hidden iframe and calls the iframe's own
/// `print()` once it has finished loading — the standard way to drive the browser's native print
/// dialog on a PDF without a download or a popup window the browser might block. The blob URL and
/// the iframe are both cleaned up once the print dialog closes, so repeated prints don't leak
/// them.
fn print_pdf(bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;

    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/pdf");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;

    let iframe: web_sys::HtmlIFrameElement = document.create_element("iframe").map_err(js)?.dyn_into().map_err(|_| "not an iframe")?;
    iframe.style().set_property("display", "none").map_err(js)?;
    document.body().ok_or("no body")?.append_child(&iframe).map_err(js)?;

    // `onload` fires once the PDF has actually rendered inside the iframe;
    // printing before that would show a blank page. Set the handler BEFORE
    // `src` so a fast/cached load can't fire before we're listening.
    let iframe_for_load = iframe.clone();
    let url_for_cleanup = url.clone();
    let on_load = wasm_bindgen::closure::Closure::once_into_js(move || {
        let Some(content_window) = iframe_for_load.content_window() else { return };
        let _ = content_window.print();
        // `afterprint` fires once the print dialog (or print preview) closes, whether the user
        // printed or cancelled — the right moment to release the blob URL and drop the iframe.
        let iframe_for_cleanup = iframe_for_load.clone();
        let on_after_print = wasm_bindgen::closure::Closure::once_into_js(move || {
            web_sys::Url::revoke_object_url(&url_for_cleanup).ok();
            iframe_for_cleanup.remove();
        });
        let _ = content_window.add_event_listener_with_callback("afterprint", on_after_print.unchecked_ref());
    });
    iframe.set_onload(Some(on_load.unchecked_ref()));
    iframe.set_src(&url);
    Ok(())
}

fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("xlsm") => "application/vnd.ms-excel.sheet.macroEnabled.12",
        Some("xlsb") => "application/vnd.ms-excel.sheet.binary.macroEnabled.12",
        Some("ods") => "application/vnd.oasis.opendocument.spreadsheet",
        Some("csv") => "text/csv",
        Some("tsv" | "txt") => "text/plain",
        Some("pdf") => "application/pdf",
        Some("html" | "htm") => "text/html",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}

/// Trigger a browser save dialog or download of `bytes` named after `name`, and settle `report`
/// when the outcome is known (the picker answers asynchronously; `ctx` repaints to apply it).
/// An `Err` means nothing was started and `report` is left to the caller.
fn download(name: &str, bytes: &[u8], report: DeliveryReport, ctx: egui::Context) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;

    let show_picker_fn: Option<js_sys::Function> =
        js_sys::Reflect::get(&window, &JsValue::from_str("showSaveFilePicker")).ok().and_then(|v| v.dyn_into().ok());

    if let Some(picker_fn) = show_picker_fn {
        let ext = std::path::Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("xlsx").to_ascii_lowercase();
        let dot_ext = format!(".{ext}");
        let mime = mime_for(name);

        let accept_obj = js_sys::Object::new();
        let exts_arr = js_sys::Array::new();
        exts_arr.push(&JsValue::from_str(&dot_ext));
        let _ = js_sys::Reflect::set(&accept_obj, &JsValue::from_str(mime), &exts_arr);

        let type_desc = match ext.as_str() {
            "xlsx" => "Excel Workbook (*.xlsx)",
            "xlsm" => "Excel Macro-Enabled Workbook (*.xlsm)",
            "xlsb" => "Excel Binary Workbook (*.xlsb)",
            "ods" => "OpenDocument Spreadsheet (*.ods)",
            "csv" => "CSV UTF-8 (with BOM) (*.csv)",
            "tsv" => "Tab Delimited Text (*.tsv)",
            "txt" => "Plain Text (*.txt)",
            "pdf" => "PDF Document (*.pdf)",
            "html" | "htm" => "HTML Document (*.html)",
            "json" => "JSON Document (*.json)",
            "png" => "PNG Image (*.png)",
            "jpg" | "jpeg" => "JPEG Image (*.jpeg)",
            _ => "Spreadsheet",
        };

        let type_desc = if ext == "csv" { gridcraft_ui_egui::i18n::current(&ctx).tr(type_desc) } else { type_desc };
        let type_obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&type_obj, &JsValue::from_str("description"), &JsValue::from_str(type_desc));
        let _ = js_sys::Reflect::set(&type_obj, &JsValue::from_str("accept"), &accept_obj);

        let types_arr = js_sys::Array::new();
        types_arr.push(&type_obj);

        let picker_opts = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&picker_opts, &JsValue::from_str("suggestedName"), &JsValue::from_str(name));
        let _ = js_sys::Reflect::set(&picker_opts, &JsValue::from_str("types"), &types_arr);

        let prom_val = picker_fn.call1(&window, &picker_opts).map_err(js)?;
        let prom: js_sys::Promise = prom_val.dyn_into().map_err(|_| "not a promise".to_string())?;

        let bytes_vec = bytes.to_vec();
        wasm_bindgen_futures::spawn_local(async move {
            let delivery = match wasm_bindgen_futures::JsFuture::from(prom).await {
                Ok(handle_val) => match write_file_handle(&handle_val, &bytes_vec).await {
                    Ok(()) => {
                        log::info!("Successfully saved file through save file picker");
                        Delivery::Delivered
                    }
                    Err(e) => {
                        log::error!("failed writing to file handle: {e}");
                        Delivery::Failed(SAVE_FAILED.into())
                    }
                },
                Err(e) => {
                    let err_name = js_sys::Reflect::get(&e, &JsValue::from_str("name")).ok().and_then(|v| v.as_string()).unwrap_or_default();
                    if err_name == "AbortError" {
                        log::info!("Save file dialog cancelled by user");
                        Delivery::Cancelled
                    } else {
                        log::error!("showSaveFilePicker failed ({err_name}): {e:?}");
                        Delivery::Failed(SAVE_FAILED.into())
                    }
                }
            };
            report.settle(delivery);
            ctx.request_repaint();
        });
        return Ok(());
    }

    // Fallback for browsers without showSaveFilePicker (e.g. Firefox, Safari):
    trigger_anchor_download(&window, name, bytes)?;
    report.settle(Delivery::Delivered);
    Ok(())
}

async fn write_file_handle(handle: &JsValue, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let create_fn: js_sys::Function = js_sys::Reflect::get(handle, &JsValue::from_str("createWritable"))
        .map_err(js)?
        .dyn_into()
        .map_err(|_| "createWritable is not a function".to_string())?;
    let stream_prom: js_sys::Promise = create_fn.call0(handle).map_err(js)?.dyn_into().map_err(|_| "not a promise".to_string())?;
    let stream = wasm_bindgen_futures::JsFuture::from(stream_prom).await.map_err(js)?;

    let write_fn: js_sys::Function =
        js_sys::Reflect::get(&stream, &JsValue::from_str("write")).map_err(js)?.dyn_into().map_err(|_| "write is not a function".to_string())?;
    let uint8 = js_sys::Uint8Array::from(bytes);
    let write_prom: js_sys::Promise = write_fn.call1(&stream, &uint8).map_err(js)?.dyn_into().map_err(|_| "not a promise".to_string())?;
    wasm_bindgen_futures::JsFuture::from(write_prom).await.map_err(js)?;

    let close_fn: js_sys::Function =
        js_sys::Reflect::get(&stream, &JsValue::from_str("close")).map_err(js)?.dyn_into().map_err(|_| "close is not a function".to_string())?;
    let close_prom: js_sys::Promise = close_fn.call0(&stream).map_err(js)?.dyn_into().map_err(|_| "not a promise".to_string())?;
    wasm_bindgen_futures::JsFuture::from(close_prom).await.map_err(js)?;
    Ok(())
}

fn trigger_anchor_download(window: &web_sys::Window, name: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime_for(name));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor".to_string())?;
    a.set_href(&url);
    a.set_download(name);
    a.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        web_sys::Url::revoke_object_url(&url).ok();
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000).map_err(js)?;
    Ok(())
}
