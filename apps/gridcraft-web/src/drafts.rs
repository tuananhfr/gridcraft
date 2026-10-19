//! Draft autosave bridge to the ERPCons office shell's `window.erpDrafts` (`put`, `remove`,
//! `ready`). The shell stores the drafts and owns every user-facing text; without it (GridCraft
//! opened on its own) nothing happens. When to put is decided by [`drafts::Planner`].

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gridcraft_engine::model::Workbook;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::drafts::{self, Planner};
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
    /// Put by a flush since the last frame: (uid, workbook, encoding ms).
    flushed: Vec<(u64, Arc<Workbook>, f64)>,
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
                if format.as_string().as_deref() != Some(drafts::FORMAT) {
                    log::warn!("erpDrafts restore of {name}: not an {} draft ({format:?})", drafts::FORMAT);
                    return;
                }
                let Ok(bytes) = bytes.dyn_into::<js_sys::Uint8Array>() else {
                    log::warn!("erpDrafts restore of {name}: bytes are not a Uint8Array");
                    return;
                };
                match inbox.try_borrow_mut() {
                    Ok(mut queue) => {
                        queue.push(Restore { key, name, bytes: bytes.to_vec() });
                        ctx.request_repaint();
                    }
                    Err(e) => log::warn!("erpDrafts restore of {name} dropped: {e}"),
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
                None => log::warn!("couldn't reopen the draft {}", r.name),
            }
        }
    }

    /// Puts and removes drafts as the planner says. Call at the end of every frame.
    pub fn after_frame(&mut self, app: &SheetApp, ctx: &egui::Context) {
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
            }
        }
        let docs = app.draft_docs();
        let plan = self.planner.plan(&docs, now, app.editing_in_place());
        for key in &plan.remove {
            call(&store, "remove", &[JsValue::from_str(key)]);
        }
        for d in docs.iter().filter(|d| plan.put.contains(&d.uid)) {
            let cost = put(&store, &self.planner.key(d.uid), &d.name, &d.wb);
            self.planner.record_put(d.uid, &d.wb, now, cost);
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
fn put(store: &JsValue, key: &str, name: &str, wb: &Workbook) -> f64 {
    let start = js_sys::Date::now();
    let encoded = drafts::draft_bytes(wb);
    let cost = js_sys::Date::now() - start;
    match encoded {
        Ok(bytes) => call(
            store,
            "put",
            &[JsValue::from_str(key), JsValue::from_str(name), JsValue::from_str(drafts::FORMAT), js_sys::Uint8Array::from(&bytes[..]).into()],
        ),
        Err(e) => log::warn!("couldn't encode a draft of {name}: {e}"),
    }
    cost
}

/// Puts every document with undrafted work right away: the page may be discarded while hidden.
fn flush(shared: &RefCell<Shared>) {
    let Some(store) = store() else { return };
    let Ok(mut shared) = shared.try_borrow_mut() else { return };
    for p in std::mem::take(&mut shared.pending) {
        let cost = put(&store, &p.key, &p.name, &p.wb);
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
