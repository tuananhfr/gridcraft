//! Optional notification owned by the embedding ERP shell.

use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;

pub fn notify() {
    let Some(window) = web_sys::window() else { return };
    let callback = js_sys::Reflect::get(&window, &JsValue::from_str("erpEditorClosed"))
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok());
    let Some(callback) = callback else {
        return;
    };
    if let Err(error) = callback.call0(&window) {
        log::warn!("erpEditorClosed failed: {error:?}");
    }
}
