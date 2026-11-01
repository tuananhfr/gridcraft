//! `erpHost` v3, region co-editing, without JavaScript: what the shell's calls do to the app and
//! what the app asks of the shell each frame. The engine's [`Live`] does the work; this keeps it
//! pointed at the document the shell shares and turns its feedback into toasts.

use gridcraft_engine::live::{Call, Cursor, Live, Lock};
use gridcraft_ui_egui::SheetApp;
use serde_json::Value as Json;

/// One of the shell's v3 calls, applied at the start of the next frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LiveMsg {
    /// `regionWillEdit`'s promise resolved.
    Answer(String, bool),
    /// `applyRemote(regionId, payload, author)`.
    Apply(String, String),
    /// `showLocks(locks)`: others' locks, and the regions whose lock is ours.
    Locks(Vec<Lock>, Vec<String>),
    /// `showCursors(cursors)`.
    Cursors(Vec<Cursor>),
    /// `showPresence(members)`: whether this page is among them.
    Presence(bool),
    /// `lockDenied(regionId)`.
    Denied(String),
    /// `isLeader(bool)`.
    Leader(bool),
}

/// Keeps [`Session::live`](gridcraft_engine::Session::live) on document `target` (uid, version).
pub(crate) fn follow(app: &mut SheetApp, target: Option<(u64, Option<u64>)>) {
    let Some((uid, version)) = target else { return };
    let origin = version.map(|v| format!("v{v}"));
    match app.session.live.as_mut() {
        Some(live) => live.set_doc(uid, origin),
        None => {
            let prefix = app.session.client_prefix();
            app.session.live = Some(Live::new(uid, &prefix, origin));
        }
    }
}

pub(crate) fn deliver(app: &mut SheetApp, messages: Vec<LiveMsg>) {
    for message in messages {
        match message {
            LiveMsg::Answer(region, granted) => app.session.live_answer(&region, granted),
            LiveMsg::Apply(region, payload) => {
                if let Err(e) = app.session.live_apply_remote(&region, &payload) {
                    log::warn!("erpHost.applyRemote: not applied: {e}");
                }
            }
            LiveMsg::Denied(region) => app.session.live_lock_denied(&region),
            other => {
                let Some(live) = app.session.live.as_mut() else { continue };
                match other {
                    LiveMsg::Locks(others, mine) => live.set_locks(others, &mine),
                    LiveMsg::Cursors(cursors) => live.set_cursors(cursors),
                    LiveMsg::Presence(online) => live.set_online(online),
                    LiveMsg::Leader(leader) => live.set_leader(leader),
                    _ => {}
                }
            }
        }
    }
}

/// End of a frame: hands back regions left, shows feedback, and returns the calls for the shell.
pub(crate) fn after_frame(app: &mut SheetApp, now: f64) -> Vec<Call> {
    app.session.live_tick(now);
    let Some(live) = app.session.live.as_mut() else { return Vec::new() };
    let denied = live.take_editor_denied();
    let feedback = live.take_feedback();
    let calls = live.take_calls();
    if denied {
        app.cancel_edit();
    }
    if let Some(f) = feedback {
        let text = f.fill(&app.ui.language.tr_message(f.text));
        app.toast = Some((text, gridcraft_ui_egui::now_ms()));
    }
    calls
}

fn text(v: &Json, key: &str) -> String {
    v.get(key).and_then(Json::as_str).unwrap_or_default().to_string()
}

/// `showLocks`' list as (others' locks, regions whose lock is ours).
pub(crate) fn locks_of(list: &Json) -> (Vec<Lock>, Vec<String>) {
    let mut others = Vec::new();
    let mut mine = Vec::new();
    for l in list.as_array().into_iter().flatten() {
        let region = text(l, "region");
        if region.is_empty() {
            continue;
        }
        if l.get("mine").and_then(Json::as_bool) == Some(true) {
            mine.push(region);
        } else {
            others.push(Lock { region, holder: text(l, "holder"), name: text(l, "name"), colour: text(l, "colour") });
        }
    }
    (others, mine)
}

pub(crate) fn cursors_of(list: &Json) -> Vec<Cursor> {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|c| Cursor {
            author: text(c, "author"),
            name: text(c, "name"),
            colour: text(c, "colour"),
            region: text(c, "region"),
            range: text(c, "range"),
        })
        .collect()
}
