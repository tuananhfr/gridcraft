//! Calls into the ERPCons office shell's optional `window` functions (File Access v3): sharing a
//! copy through the system share sheet and reporting what became of each dropped file. Without the
//! shell (trunk serve, other hosts) nothing is offered and nothing is reported.

use gridcraft_ui_egui::log_redact::file_label;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;

/// The shell's `window.<name>` function, when it installed one.
fn function(name: &str) -> Option<(web_sys::Window, js_sys::Function)> {
    let window = web_sys::window()?;
    let f = js_sys::Reflect::get(&window, &JsValue::from_str(name)).ok()?.dyn_into::<js_sys::Function>().ok()?;
    Some((window, f))
}

/// `Services::share` when `erpCanShareFiles(mime)` says the browser can share an `.xlsx`, which
/// Chromium refuses. Asked once: the answer depends on the browser and the embedding only.
pub fn share_service() -> Option<Box<dyn Fn(&str, &[u8]) -> bool>> {
    let (window, can_share) = function("erpCanShareFiles")?;
    let mime = JsValue::from_str(crate::web::mime_for("Book.xlsx"));
    if can_share.call1(&window, &mime).ok()?.as_bool() != Some(true) {
        return None;
    }
    Some(Box::new(share))
}

/// `erpShare(bytes, name, mime)`, called inside the click so the browser still sees the user's
/// gesture; `false` (the shell is gone, or declined) leaves the download to the caller.
fn share(name: &str, bytes: &[u8]) -> bool {
    let Some((window, f)) = function("erpShare") else { return false };
    let args = [js_sys::Uint8Array::from(bytes).into(), JsValue::from_str(name), JsValue::from_str(crate::web::mime_for(name))];
    match f.apply(&window, &args.iter().collect::<js_sys::Array>()) {
        Ok(taken) => taken.as_bool() == Some(true),
        Err(e) => {
            log::warn!("erpShare failed for a {}: {e:?}", file_label(name, Some(bytes.len())));
            false
        }
    }
}

/// `erpIntakeResult(name, size, ok)` for a file dropped on the page, once the app has opened or
/// refused it.
pub fn intake_result(name: &str, size: usize, ok: bool) {
    let Some((window, f)) = function("erpIntakeResult") else { return };
    if let Err(e) = f.call3(&window, &JsValue::from_str(name), &JsValue::from_f64(size as f64), &JsValue::from_bool(ok)) {
        log::warn!("erpIntakeResult failed: {e:?}");
    }
}
