//! The JavaScript side of `erpHost` v3: the region handlers given to `erpHost.ready`, the region
//! calls the fork makes, and the "keep mine / use theirs" question of `regionConflict`. A shell
//! with no `regionWillEdit` gets none of this.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use gridcraft_engine::live::Call;
use gridcraft_ui_egui::SheetApp;
use serde_json::Value as Json;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use super::live::{LiveMsg, cursors_of, locks_of};

pub(crate) type Inbox = Rc<RefCell<Vec<LiveMsg>>>;
/// `regionConflict` questions waiting for the user: (region, the promise's resolve).
pub(crate) type Conflicts = Rc<RefCell<VecDeque<(String, js_sys::Function)>>>;

/// The shell takes region calls (`erpHost` v3, found by feature: its `version` stays 2).
pub(crate) fn speaks_regions(host: &JsValue) -> bool {
    js_sys::Reflect::get(host, &JsValue::from_str("regionWillEdit")).is_ok_and(|f| f.is_function())
}

fn json_of(value: &JsValue) -> Json {
    js_sys::JSON::stringify(value).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Json::Null)
}

fn set(handlers: &js_sys::Object, name: &str, f: JsValue) -> Result<(), JsValue> {
    js_sys::Reflect::set(handlers, &JsValue::from_str(name), &f).map(|_| ())
}

/// Adds the v3 handlers to those given to `erpHost.ready`.
pub(crate) fn register(handlers: &js_sys::Object, inbox: &Inbox, conflicts: &Conflicts, ctx: &egui::Context) -> Result<(), JsValue> {
    let push = |inbox: &Inbox, ctx: &egui::Context| {
        let (inbox, ctx) = (inbox.clone(), ctx.clone());
        move |m: LiveMsg| {
            inbox.borrow_mut().push(m);
            ctx.request_repaint();
        }
    };
    let send = push(inbox, ctx);
    let apply = Closure::<dyn FnMut(JsValue, JsValue, JsValue)>::new(move |region: JsValue, payload: JsValue, _author: JsValue| {
        match (region.as_string(), payload.as_string()) {
            (Some(r), Some(p)) => send(LiveMsg::Apply(r, p)),
            _ => log::warn!("erpHost.applyRemote: ignored an odd call"),
        }
    });
    set(handlers, "applyRemote", apply.into_js_value())?;
    let send = push(inbox, ctx);
    let presence = Closure::<dyn FnMut(JsValue)>::new(move |members: JsValue| {
        let here = json_of(&members).as_array().is_some_and(|list| list.iter().any(|m| m.get("you").and_then(Json::as_bool) == Some(true)));
        send(LiveMsg::Presence(here));
    });
    set(handlers, "showPresence", presence.into_js_value())?;
    let send = push(inbox, ctx);
    let locks = Closure::<dyn FnMut(JsValue)>::new(move |list: JsValue| {
        let (others, mine) = locks_of(&json_of(&list));
        send(LiveMsg::Locks(others, mine));
    });
    set(handlers, "showLocks", locks.into_js_value())?;
    let send = push(inbox, ctx);
    let cursors = Closure::<dyn FnMut(JsValue)>::new(move |list: JsValue| send(LiveMsg::Cursors(cursors_of(&json_of(&list)))));
    set(handlers, "showCursors", cursors.into_js_value())?;
    let send = push(inbox, ctx);
    let denied = Closure::<dyn FnMut(JsValue)>::new(move |region: JsValue| {
        if let Some(r) = region.as_string() {
            send(LiveMsg::Denied(r));
        }
    });
    set(handlers, "lockDenied", denied.into_js_value())?;
    let send = push(inbox, ctx);
    let leader = Closure::<dyn FnMut(JsValue)>::new(move |on: JsValue| send(LiveMsg::Leader(on.as_bool() == Some(true))));
    set(handlers, "isLeader", leader.into_js_value())?;
    let (queue, repaint) = (conflicts.clone(), ctx.clone());
    let conflict = Closure::<dyn FnMut(JsValue) -> JsValue>::new(move |region: JsValue| {
        let region = region.as_string().unwrap_or_default();
        let queue = queue.clone();
        let promise = js_sys::Promise::new(&mut |resolve: js_sys::Function, _reject: js_sys::Function| {
            queue.borrow_mut().push_back((region.clone(), resolve));
        });
        repaint.request_repaint();
        promise.into()
    });
    set(handlers, "regionConflict", conflict.into_js_value())?;
    Ok(())
}

/// Makes the engine's calls on the shell; `regionWillEdit`'s answer comes back through `inbox`.
pub(crate) fn call(host: &JsValue, calls: Vec<Call>, inbox: &Inbox, ctx: &egui::Context) {
    for c in calls {
        match c {
            Call::WillEdit(region) => {
                let asked = invoke(host, "regionWillEdit", &[JsValue::from_str(&region)]);
                let (inbox, ctx) = (inbox.clone(), ctx.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    let granted = match asked {
                        Some(v) if v.is_instance_of::<js_sys::Promise>() => {
                            match wasm_bindgen_futures::JsFuture::from(js_sys::Promise::from(v)).await {
                                Ok(v) => v.as_bool() == Some(true),
                                Err(e) => {
                                    log::warn!("erpHost.regionWillEdit was rejected: {e:?}");
                                    false
                                }
                            }
                        }
                        Some(v) => v.as_bool() == Some(true),
                        None => false,
                    };
                    inbox.borrow_mut().push(LiveMsg::Answer(region, granted));
                    ctx.request_repaint();
                });
            }
            Call::Changed(region, payload) => {
                invoke(host, "regionChanged", &[JsValue::from_str(&region), JsValue::from_str(&payload)]);
            }
            Call::DidEndEdit(region) => {
                invoke(host, "regionDidEndEdit", &[JsValue::from_str(&region)]);
            }
            Call::Selection(region, range) => {
                invoke(host, "selectionChanged", &[JsValue::from_str(&region), JsValue::from_str(&range)]);
            }
        }
    }
}

fn invoke(host: &JsValue, method: &str, args: &[JsValue]) -> Option<JsValue> {
    let f = js_sys::Reflect::get(host, &JsValue::from_str(method)).ok()?.dyn_into::<js_sys::Function>().ok()?;
    match f.apply(host, &args.iter().collect::<js_sys::Array>()) {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("erpHost.{method} failed: {e:?}");
            None
        }
    }
}

/// The oldest `regionConflict` question, until the user answers it.
pub(crate) fn ask_conflict(conflicts: &Conflicts, app: &SheetApp, ctx: &egui::Context) {
    let Some((region, resolve)) = conflicts.borrow().front().cloned() else { return };
    let tr = |s: &str| app.ui.language.tr(s).to_string();
    let mut answer = None;
    egui::Window::new(tr("Changed while you were away"))
        .id(egui::Id::new("live_conflict").with(&region))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -40.0))
        .show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.label(tr("Someone else changed this part while you were offline. Which version do you keep?"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Keep mine")).clicked() {
                    answer = Some("mine");
                }
                if ui.button(tr("Use theirs")).clicked() {
                    answer = Some("theirs");
                }
            });
        });
    if let Some(answer) = answer {
        conflicts.borrow_mut().pop_front();
        if let Err(e) = resolve.call1(&JsValue::UNDEFINED, &JsValue::from_str(answer)) {
            log::warn!("erpHost.regionConflict: couldn't answer: {e:?}");
        }
        ctx.request_repaint();
    }
}
