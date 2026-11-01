//! The JavaScript side of the `erpHost` comment calls (M5): `anchorOfSelection`, `showComments`,
//! `focusAnchor` and `resolveAnchors` given to `erpHost.ready`, and `erpHost.commentClicked`
//! when a mark is clicked. Only a shell whose `erpHost` has `commentClicked` gets them.

use std::cell::RefCell;
use std::rc::Rc;

use gridcraft_ui_egui::SheetApp;
use serde_json::Value as Json;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use super::comments::{self, CommentMsg};

/// The shell shows Cloud comments.
pub(crate) fn speaks_comments(host: &JsValue) -> bool {
    js_sys::Reflect::get(host, &JsValue::from_str("commentClicked")).is_ok_and(|f| f.is_function())
}

#[derive(Default)]
pub(crate) struct Shared {
    inbox: RefCell<Vec<CommentMsg>>,
    /// `resolveAnchors` calls waiting for the next frame: (threads, the promise's resolve).
    resolving: RefCell<Vec<(Json, js_sys::Function)>>,
    /// The anchor of the selection as of the last frame, as JSON text ("null" for none).
    selection: RefCell<String>,
}

pub(crate) struct Comments {
    host: JsValue,
    shared: Rc<Shared>,
}

fn json_of(value: &JsValue) -> Json {
    js_sys::JSON::stringify(value).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Json::Null)
}

fn js_of(json: &Json) -> JsValue {
    js_sys::JSON::parse(&json.to_string()).unwrap_or(JsValue::NULL)
}

impl Comments {
    pub(crate) fn new(host: JsValue) -> Comments {
        let shared = Shared::default();
        *shared.selection.borrow_mut() = "null".into();
        Comments { host, shared: Rc::new(shared) }
    }

    /// Adds the comment handlers to those given to `erpHost.ready`.
    pub(crate) fn register(&self, handlers: &js_sys::Object, ctx: &egui::Context) -> Result<(), JsValue> {
        let set = |name: &str, f: JsValue| js_sys::Reflect::set(handlers, &JsValue::from_str(name), &f).map(|_| ());
        let shared = self.shared.clone();
        let anchor = Closure::<dyn FnMut() -> JsValue>::new(move || js_sys::JSON::parse(&shared.selection.borrow()).unwrap_or(JsValue::NULL));
        set("anchorOfSelection", anchor.into_js_value())?;
        let (shared, repaint) = (self.shared.clone(), ctx.clone());
        let show = Closure::<dyn FnMut(JsValue)>::new(move |threads: JsValue| {
            shared.inbox.borrow_mut().push(CommentMsg::Show(json_of(&threads)));
            repaint.request_repaint();
        });
        set("showComments", show.into_js_value())?;
        let (shared, repaint) = (self.shared.clone(), ctx.clone());
        let focus = Closure::<dyn FnMut(JsValue, JsValue)>::new(move |anchor: JsValue, thread: JsValue| {
            shared.inbox.borrow_mut().push(CommentMsg::Focus(json_of(&anchor), thread.as_string()));
            repaint.request_repaint();
        });
        set("focusAnchor", focus.into_js_value())?;
        let (shared, repaint) = (self.shared.clone(), ctx.clone());
        let resolve = Closure::<dyn FnMut(JsValue) -> JsValue>::new(move |threads: JsValue| {
            let threads = json_of(&threads);
            let shared = shared.clone();
            let promise = js_sys::Promise::new(&mut |resolve: js_sys::Function, _reject: js_sys::Function| {
                shared.resolving.borrow_mut().push((threads.clone(), resolve));
            });
            repaint.request_repaint();
            promise.into()
        });
        set("resolveAnchors", resolve.into_js_value())?;
        Ok(())
    }

    /// Start of a frame, after the document and region calls: shows, focuses, then answers
    /// `resolveAnchors` against the content as it is now.
    pub(crate) fn deliver(&self, app: &mut SheetApp, doc: Option<u64>) {
        let messages = std::mem::take(&mut *self.shared.inbox.borrow_mut());
        comments::deliver(app, doc, messages);
        let waiting = std::mem::take(&mut *self.shared.resolving.borrow_mut());
        for (threads, resolve) in waiting {
            let updates = comments::resolve(app, doc, &threads);
            if let Err(e) = resolve.call1(&JsValue::UNDEFINED, &js_of(&updates)) {
                log::warn!("erpHost.resolveAnchors: couldn't answer: {e:?}");
            }
        }
    }

    /// End of a frame: the selection's anchor for the next `anchorOfSelection`, and clicked marks.
    pub(crate) fn after_frame(&self, app: &mut SheetApp, doc: Option<u64>) {
        *self.shared.selection.borrow_mut() = comments::anchor_of_selection(app, doc).to_string();
        for thread in app.comment_marks.take_clicked() {
            let Ok(f) = js_sys::Reflect::get(&self.host, &JsValue::from_str("commentClicked")) else { continue };
            let Ok(f) = f.dyn_into::<js_sys::Function>() else { continue };
            if let Err(e) = f.call1(&self.host, &JsValue::from_str(&thread)) {
                log::warn!("erpHost.commentClicked failed: {e:?}");
            }
        }
    }
}
