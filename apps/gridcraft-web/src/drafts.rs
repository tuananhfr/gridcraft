//! Draft autosave bridge to the ERPCons office shell's `window.erpDrafts` (`put`, `remove`,
//! `ready`). The shell stores the drafts and owns every user-facing text; without it (GridCraft
//! opened on its own) nothing happens. When to put is decided by [`drafts::Planner`].

use std::cell::RefCell;
use std::collections::BTreeMap;

use std::rc::Rc;
use std::sync::Arc;

use crate::draft_state::{Message, States};

use gridcraft_engine::model::Workbook;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::drafts::{self, Planner};
use gridcraft_ui_egui::log_redact::file_label;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

/// A document with work its last draft lacks, as the hide-time flush needs it.
struct Pending {
    uid: u64,
    key: String,
    name: String,
    wb: Arc<Workbook>,
}

/// Shared with the page-hide listeners, which can't reach into the eframe app.
#[derive(Default)]
struct Shared {
    /// Refreshed at the end of every frame.
    pending: Vec<Pending>,
    tracking: Rc<RefCell<Tracking>>,
    /// Put by a flush since the last frame: (uid, workbook, encoding ms).
    flushed: Vec<(u64, Arc<Workbook>, f64)>,
}

#[derive(Default)]
struct Tracking {
    states: States,
    seen: BTreeMap<String, (Arc<Workbook>, u64)>,
}

fn send(store: &JsValue, messages: Vec<Message>) {
    let Ok(function) = js_sys::Reflect::get(store, &JsValue::from_str("state")) else { return };
    let Some(function) = function.dyn_ref::<js_sys::Function>() else { return };
    for m in messages {
        let value = js_sys::Object::new();
        let fields = [
            ("docId", JsValue::from_str(&m.doc_id)),
            ("name", JsValue::from_str(&m.name)),
            ("revision", JsValue::from_f64(m.revision as f64)),
            ("dirty", JsValue::from_bool(m.dirty)),
            ("pending", JsValue::from_bool(m.pending)),
            ("committed", JsValue::from_bool(m.committed)),
            ("failed", JsValue::from_bool(m.failed)),
        ];
        if fields.into_iter().all(|(key, v)| js_sys::Reflect::set(&value, &JsValue::from_str(key), &v).is_ok()) {
            let _ = function.call1(store, &value);
        }
    }
}

/// A draft the shell asked to reopen.
struct Restore {
    key: String,
    name: String,
    bytes: Vec<u8>,
}

pub struct Drafts {
    planner: Planner,
    shared: Rc<RefCell<Shared>>,
    restores: Rc<RefCell<Vec<Restore>>>,
    /// The `restore(key, name, format, bytes)` callback handed to `erpDrafts.ready`.
    restore_fn: JsValue,
    ready_sent: bool,
}

impl Drafts {
    pub fn install(ctx: &egui::Context) -> Drafts {
        let shared = Rc::new(RefCell::new(Shared::default()));
        let restores = Rc::new(RefCell::new(Vec::new()));
        let inbox = restores.clone();
        let ctx = ctx.clone();
        // Only queued here: opening inside the shell's call would run outside a frame.
        let restore_fn =
            Closure::<dyn FnMut(JsValue, JsValue, JsValue, JsValue)>::new(move |key: JsValue, name: JsValue, format: JsValue, bytes: JsValue| {
                let (Some(key), Some(name)) = (key.as_string(), name.as_string()) else {
                    log::warn!("erpDrafts restore without a key or name");
                    return;
                };
                if !format.as_string().is_some_and(|f| drafts::FORMATS.contains(&f.as_str())) {
                    log::warn!("erpDrafts restore of a {}: not a workbook draft ({format:?})", file_label(&name, None));
                    return;
                }
                let Ok(bytes) = bytes.dyn_into::<js_sys::Uint8Array>() else {
                    log::warn!("erpDrafts restore of a {}: bytes are not a Uint8Array", file_label(&name, None));
                    return;
                };
                match inbox.try_borrow_mut() {
                    Ok(mut queue) => {
                        queue.push(Restore { key, name, bytes: bytes.to_vec() });
                        ctx.request_repaint();
                    }
                    Err(e) => log::warn!("erpDrafts restore of a {} dropped: {e}", file_label(&name, Some(bytes.length() as usize))),
                }
            })
            .into_js_value();
        if let Err(e) = listen_for_hide(shared.clone()) {
            log::warn!("drafts won't be flushed when the page hides: {e}");
        }
        Drafts { planner: Planner::default(), shared, restores, restore_fn, ready_sent: false }
    }

    /// Opens the drafts the shell asked to restore. Call before the app's `logic`.
    pub fn open_restored(&mut self, app: &mut SheetApp) {
        let restores = self.restores.try_borrow_mut().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        for r in restores {
            let opened = app.open_draft(&r.name, &r.bytes).and_then(|uid| app.session.documents().iter().find(|d| d.uid == uid));
            match opened {
                Some(d) => self.planner.adopt(d.uid, &r.key, &d.wb, js_sys::Date::now()),
                None => log::warn!("couldn't reopen a draft {}", file_label(&r.name, Some(r.bytes.len()))),
            }
        }
    }

    /// Puts and removes drafts as the planner says. Call at the end of every frame.
    pub fn after_frame(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        // Looked up every frame: the shell may be absent, and a missing one is never cached.
        let Some(store) = store() else { return };
        if !self.ready_sent {
            self.ready_sent = true;
            call(&store, "ready", std::slice::from_ref(&self.restore_fn));
        }
        let now = js_sys::Date::now();
        if let Ok(mut shared) = self.shared.try_borrow_mut() {
            for (uid, wb, cost) in shared.flushed.drain(..) {
                self.planner.record_put(uid, &wb, now, cost);
                let name = app.session.documents().iter().find(|d| d.uid == uid).map(|d| drafts::draft_name(&d.display_title())).unwrap_or_default();
                app.show_draft_warnings(uid, &gridcraft_engine::io::save_warnings(&wb, &name));
            }
        }
        let docs = app.draft_docs();
        let tracking = match self.shared.try_borrow() {
            Ok(shared) => shared.tracking.clone(),
            Err(_) => return,
        };
        if let Ok(mut t) = tracking.try_borrow_mut() {
            let active = app.session.active().map(|d| self.planner.key(d.uid));
            // Select the new tab before clearing closed background documents.
            for d in &docs {
                let key = self.planner.key(d.uid);
                let revision = app.session.documents().iter().find(|doc| doc.uid == d.uid).map_or(0, |doc| doc.revision);
                let edits = t.seen.get(&key).map_or(0, |(wb, previous)| revision.saturating_sub(*previous).max(u64::from(!Arc::ptr_eq(wb, &d.wb))));
                // A restored workbook already has a device draft even though it is unsaved to disk.
                let unsaved = if t.seen.contains_key(&key) { d.unsaved } else { self.planner.is_pending(d) };
                t.seen.insert(key.clone(), (d.wb.clone(), revision));
                send(&store, t.states.observe(&key, &d.name, edits, unsaved));
            }
            send(&store, t.states.switch(active.as_deref()));
            let closed: Vec<_> = t.seen.keys().filter(|key| !docs.iter().any(|d| self.planner.key(d.uid) == **key)).cloned().collect();
            for key in closed {
                t.seen.remove(&key);
                send(&store, t.states.close(&key));
            }
        }
        let plan = self.planner.plan(&docs, now, app.editing_in_place());
        for key in &plan.remove {
            call(&store, "remove", &[JsValue::from_str(key)]);
        }
        for d in docs.iter().filter(|d| plan.put.contains(&d.uid)) {
            let cost = put(&store, &self.planner.key(d.uid), &d.name, &d.wb, &tracking);
            self.planner.record_put(d.uid, &d.wb, now, cost);
            app.show_draft_warnings(d.uid, &gridcraft_engine::io::save_warnings(&d.wb, &d.name));
        }
        if plan.waiting {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
        if let Ok(mut shared) = self.shared.try_borrow_mut() {
            shared.pending = docs
                .iter()
                .filter(|d| self.planner.is_pending(d))
                .map(|d| Pending { uid: d.uid, key: self.planner.key(d.uid), name: d.name.clone(), wb: d.wb.clone() })
                .collect();
        }
    }
}

/// `window.erpDrafts`, when the shell installed it.
fn store() -> Option<JsValue> {
    let window = web_sys::window()?;
    let store = js_sys::Reflect::get(&window, &JsValue::from_str("erpDrafts")).ok()?;
    store.is_object().then_some(store)
}

/// Calls `erpDrafts.<method>(...args)`; a JS exception is logged, never raised.
fn call(store: &JsValue, method: &str, args: &[JsValue]) {
    let Some(f) = js_sys::Reflect::get(store, &JsValue::from_str(method)).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok()) else {
        log::warn!("erpDrafts.{method} is not a function");
        return;
    };
    if let Err(e) = f.apply(store, &args.iter().collect::<js_sys::Array>()) {
        log::warn!("erpDrafts.{method} failed: {e:?}");
    }
}

/// Encodes `wb` and puts it under `key`; returns the encoding time in ms.
fn put(store: &JsValue, key: &str, name: &str, wb: &Workbook, tracking: &Rc<RefCell<Tracking>>) -> f64 {
    let revision = tracking.try_borrow_mut().ok().and_then(|mut t| {
        send(store, t.states.checkpoint(key));
        t.states.revision(key)
    });
    let start = js_sys::Date::now();
    let encoded = drafts::draft_bytes(wb, name);
    let cost = js_sys::Date::now() - start;
    let result = encoded.map_err(JsValue::from).and_then(|bytes| {
        let f = js_sys::Reflect::get(store, &JsValue::from_str("put"))?.dyn_into::<js_sys::Function>()?;
        f.call4(
            store,
            &JsValue::from_str(key),
            &JsValue::from_str(name),
            &JsValue::from_str(drafts::draft_format(name)),
            &js_sys::Uint8Array::from(&bytes[..]),
        )
    });
    let state_available = js_sys::Reflect::get(store, &JsValue::from_str("state")).ok().is_some_and(|v| v.is_function());
    if state_available && let Some(revision) = revision {
        let store = store.clone();
        let tracking = tracking.clone();
        let key = key.to_owned();
        wasm_bindgen_futures::spawn_local(async move {
            let success = match result {
                Ok(value) => match value.dyn_into::<js_sys::Promise>() {
                    Ok(promise) => wasm_bindgen_futures::JsFuture::from(promise).await.ok().and_then(|v| v.as_bool()) == Some(true),
                    Err(_) => false,
                },
                Err(_) => false,
            };
            if let Ok(mut t) = tracking.try_borrow_mut() {
                send(&store, t.states.complete(&key, revision, success));
            }
        });
    } else if let Err(e) = result {
        log::warn!("couldn't put a draft of a {}: {e:?}", file_label(name, None));
    }
    cost
}

/// Puts every document with undrafted work right away: the page may be discarded while hidden.
fn flush(shared: &RefCell<Shared>) {
    let Some(store) = store() else { return };
    let Ok(mut shared) = shared.try_borrow_mut() else { return };
    for p in std::mem::take(&mut shared.pending) {
        let cost = put(&store, &p.key, &p.name, &p.wb, &shared.tracking);
        shared.flushed.push((p.uid, p.wb, cost));
    }
}

fn listen_for_hide(shared: Rc<RefCell<Shared>>) -> Result<(), String> {
    let js = |e: JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let on_pagehide = shared.clone();
    let pagehide = Closure::<dyn FnMut()>::new(move || flush(&on_pagehide));
    window.add_event_listener_with_callback("pagehide", pagehide.as_ref().unchecked_ref()).map_err(js)?;
    // The listeners live as long as the page.
    pagehide.forget();
    let doc = document.clone();
    let visibility = Closure::<dyn FnMut()>::new(move || {
        if doc.visibility_state() == web_sys::VisibilityState::Hidden {
            flush(&shared);
        }
    });
    document.add_event_listener_with_callback("visibilitychange", visibility.as_ref().unchecked_ref()).map_err(js)?;
    visibility.forget();
    Ok(())
}
