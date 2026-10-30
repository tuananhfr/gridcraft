//! The JavaScript side of `erpHost`: the handlers given to `erpHost.ready`, and the calls the
//! fork makes on it. A shell exception or an odd value is logged and read as offline, never raised.

use std::cell::RefCell;
use std::rc::Rc;

use gridcraft_ui_egui::SheetApp;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use super::docs::{FlushEntry, HostDocs, Reply};
use super::protocol::{API_VERSIONS, Mode, OpenRequest, Reason, Request, SaveResult, Shell};

/// The shell lets the page go once this has passed, whatever is still being saved.
const FLUSH_LIMIT_MS: i32 = 10_000;

/// `window.erpHost` and its version when this fork speaks it, checked once at start-up.
pub(crate) fn detect() -> Option<(JsValue, u32)> {
    let window = web_sys::window()?;
    let host = js_sys::Reflect::get(&window, &JsValue::from_str("erpHost")).ok()?;
    if !host.is_object() {
        return None;
    }
    let version = js_sys::Reflect::get(&host, &JsValue::from_str("version")).ok()?.as_f64()?;
    API_VERSIONS.into_iter().find(|v| f64::from(*v) == version).map(|v| (host, v))
}

/// `acknowledged(docId, revision, version)` calls, applied at the end of the next frame.
type Acks = Rc<RefCell<Vec<(String, u64, Option<u64>)>>>;

type Requests = Rc<RefCell<Vec<Request>>>;

pub(crate) struct ErpHost {
    docs: Rc<RefCell<HostDocs>>,
    requests: Requests,
    acks: Acks,
}

impl ErpHost {
    /// Makes `app` save the shell's documents to it and registers `open`, `flush` and `acknowledged`.
    pub(crate) fn install(host: JsValue, version: u32, app: &mut SheetApp, ctx: &egui::Context) -> ErpHost {
        let docs = HostDocs::new(Rc::new(JsShell { host: host.clone(), ctx: ctx.clone() }));
        HostDocs::connect(&docs, app, version);
        let requests = Requests::default();
        let acks = Acks::default();
        if let Err(e) = ready(&host, version, &docs, &requests, &acks, ctx) {
            log::error!("erpHost.ready failed: {e:?}");
        }
        ErpHost { docs, requests, acks }
    }

    /// Call before the app's `logic`: opening inside the shell's call would run outside a frame.
    pub(crate) fn open_requested(&self, app: &mut SheetApp) {
        let requests = std::mem::take(&mut *self.requests.borrow_mut());
        let Ok(mut docs) = self.docs.try_borrow_mut() else {
            let mut queue = self.requests.borrow_mut();
            let later = std::mem::replace(&mut *queue, requests);
            queue.extend(later);
            return;
        };
        for request in requests {
            docs.apply(app, request);
        }
    }

    /// Call at the end of every frame.
    pub(crate) fn after_frame(&self, app: &mut SheetApp, ctx: &egui::Context) {
        let Ok(mut docs) = self.docs.try_borrow_mut() else { return };
        for (doc_id, revision, version) in std::mem::take(&mut *self.acks.borrow_mut()) {
            docs.acknowledged(&doc_id, revision, version);
        }
        if let Some(ms) = docs.after_frame(app, js_sys::Date::now()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(ms.ceil() as u64));
        }
    }
}

fn ready(host: &JsValue, version: u32, docs: &Rc<RefCell<HostDocs>>, requests: &Requests, acks: &Acks, ctx: &egui::Context) -> Result<(), JsValue> {
    let queue = requests.clone();
    let repaint = ctx.clone();
    let open = Closure::<dyn FnMut(JsValue)>::new(move |doc: JsValue| {
        queue.borrow_mut().push(Request::Open(read_doc(&doc, version)));
        repaint.request_repaint();
    })
    .into_js_value();
    let entry = HostDocs::flush_entry(docs);
    let repaint = ctx.clone();
    let flush = Closure::<dyn FnMut() -> JsValue>::new(move || flush_promise(&entry, &repaint).into()).into_js_value();
    let queue = acks.clone();
    let repaint = ctx.clone();
    let acknowledged = Closure::<dyn FnMut(JsValue, JsValue, JsValue)>::new(move |doc_id: JsValue, revision: JsValue, version: JsValue| {
        match (doc_id.as_string(), whole_number(&revision)) {
            (Some(doc_id), Some(revision)) => {
                queue.borrow_mut().push((doc_id, revision, whole_number(&version)));
                repaint.request_repaint();
            }
            _ => log::warn!("erpHost.acknowledged: ignored an odd call"),
        }
    })
    .into_js_value();
    let handlers = js_sys::Object::new();
    js_sys::Reflect::set(&handlers, &JsValue::from_str("open"), &open)?;
    js_sys::Reflect::set(&handlers, &JsValue::from_str("flush"), &flush)?;
    js_sys::Reflect::set(&handlers, &JsValue::from_str("acknowledged"), &acknowledged)?;
    if version >= 2 {
        let queue = requests.clone();
        let repaint = ctx.clone();
        let set_mode =
            Closure::<dyn FnMut(JsValue, JsValue, JsValue)>::new(move |doc_id: JsValue, mode: JsValue, can_download: JsValue| {
                match doc_id.as_string() {
                    Some(doc_id) => {
                        let access = Mode::parse(mode.as_string().as_deref()).access(can_download.as_bool() != Some(false));
                        queue.borrow_mut().push(Request::Mode(doc_id, access));
                        repaint.request_repaint();
                    }
                    None => log::warn!("erpHost.setMode: ignored an odd call"),
                }
            })
            .into_js_value();
        let queue = requests.clone();
        let repaint = ctx.clone();
        let set_user = Closure::<dyn FnMut(JsValue)>::new(move |user: JsValue| {
            queue.borrow_mut().push(Request::User(user_name(&user)));
            repaint.request_repaint();
        })
        .into_js_value();
        js_sys::Reflect::set(&handlers, &JsValue::from_str("setMode"), &set_mode)?;
        js_sys::Reflect::set(&handlers, &JsValue::from_str("setUser"), &set_user)?;
    }
    let ready = js_sys::Reflect::get(host, &JsValue::from_str("ready"))?.dyn_into::<js_sys::Function>()?;
    ready.call1(host, &handlers)?;
    Ok(())
}

/// Saves run synchronously here: `pagehide` may be the page's last chance, and it runs no frames.
fn flush_promise(entry: &FlushEntry, ctx: &egui::Context) -> js_sys::Promise {
    js_sys::Promise::new(&mut |resolve: js_sys::Function, _reject: js_sys::Function| {
        let settle = resolve.clone();
        let done: Box<dyn FnOnce()> = Box::new(move || {
            let _ = settle.call0(&JsValue::UNDEFINED);
        });
        if !entry.flush(done) {
            ctx.request_repaint();
        }
        let limit = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        });
        if let Some(window) = web_sys::window()
            && let Err(e) = window.set_timeout_with_callback_and_timeout_and_arguments_0(limit.unchecked_ref(), FLUSH_LIMIT_MS)
        {
            log::warn!("erpHost.flush has no time limit: {e:?}");
        }
    })
}

/// `setUser`'s `{id, name}` name; `None` for null.
fn user_name(user: &JsValue) -> Option<String> {
    if !user.is_object() {
        return None;
    }
    js_sys::Reflect::get(user, &JsValue::from_str("name")).ok()?.as_string()
}

fn read_doc(doc: &JsValue, version: u32) -> OpenRequest {
    let get = |key: &str| js_sys::Reflect::get(doc, &JsValue::from_str(key)).ok();
    let text = |key: &str| get(key).and_then(|v| v.as_string()).unwrap_or_default();
    let bytes = get("bytes").and_then(|v| {
        if let Some(array) = v.dyn_ref::<js_sys::Uint8Array>() {
            Some(array.to_vec())
        } else if v.is_instance_of::<js_sys::ArrayBuffer>() {
            Some(js_sys::Uint8Array::new(&v).to_vec())
        } else {
            None
        }
    });
    let doc_version = get("version").as_ref().and_then(whole_number);
    // v1 has no access modes: the document stays at full access.
    let v2 = |key: &str| get(key).filter(|v| version >= 2 && !v.is_undefined() && !v.is_null());
    let mode = Mode::parse(v2("mode").and_then(|v| v.as_string()).as_deref());
    let access = mode.access(v2("canDownload").and_then(|v| v.as_bool()) != Some(false));
    let preserve_view = v2("preserveView").and_then(|v| v.as_bool()) == Some(true);
    OpenRequest { doc_id: text("docId"), version: doc_version, title: text("title"), format: text("format"), bytes, access, preserve_view }
}

fn whole_number(value: &JsValue) -> Option<u64> {
    value.as_f64().filter(|v| v.is_finite() && *v >= 0.0).map(|v| v as u64)
}

struct JsShell {
    host: JsValue,
    ctx: egui::Context,
}

impl JsShell {
    fn call(&self, method: &str, args: &[JsValue]) -> Option<JsValue> {
        let function = js_sys::Reflect::get(&self.host, &JsValue::from_str(method)).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok());
        let Some(function) = function else {
            log::warn!("erpHost.{method} is not a function");
            return None;
        };
        match function.apply(&self.host, &args.iter().collect::<js_sys::Array>()) {
            Ok(value) => Some(value),
            Err(e) => {
                log::warn!("erpHost.{method} failed: {e:?}");
                None
            }
        }
    }
}

impl Shell for JsShell {
    fn save(&self, doc_id: &str, bytes: Vec<u8>, revision: u64, reason: Reason, reply: Reply) {
        let options = js_sys::Object::new();
        let fields = [("revision", JsValue::from_f64(revision as f64)), ("reason", JsValue::from_str(reason.as_str()))];
        if !fields.into_iter().all(|(key, value)| js_sys::Reflect::set(&options, &JsValue::from_str(key), &value).is_ok()) {
            log::warn!("erpHost.save: couldn't build its options");
        }
        let result = self.call("save", &[JsValue::from_str(doc_id), js_sys::Uint8Array::from(&bytes[..]).into(), options.into()]);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let outcome = match result.map(|v| v.dyn_into::<js_sys::Promise>()) {
                Some(Ok(promise)) => match wasm_bindgen_futures::JsFuture::from(promise).await {
                    Ok(value) => save_result(&value),
                    Err(e) => {
                        log::warn!("erpHost.save was rejected: {e:?}");
                        SaveResult::Offline
                    }
                },
                Some(Err(_)) => {
                    log::warn!("erpHost.save returned no promise");
                    SaveResult::Offline
                }
                None => SaveResult::Offline,
            };
            reply.resolve(outcome);
            ctx.request_repaint();
        });
    }

    fn dirty(&self, doc_id: &str, dirty: bool) {
        self.call("dirty", &[JsValue::from_str(doc_id), JsValue::from_bool(dirty)]);
    }

    fn opened(&self, doc_id: &str, ok: bool, reason: Option<&str>) {
        let mut args = vec![JsValue::from_str(doc_id), JsValue::from_bool(ok)];
        args.extend(reason.map(JsValue::from_str));
        self.call("opened", &args);
    }

    fn closed(&self, doc_id: &str) {
        self.call("closed", &[JsValue::from_str(doc_id)]);
    }
}

/// A resolved `SaveResult`, read as JSON so an unexpected shape can't trap.
fn save_result(value: &JsValue) -> SaveResult {
    let json = js_sys::JSON::stringify(value).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok());
    json.map_or(SaveResult::Offline, |json| SaveResult::from_json(&json))
}
