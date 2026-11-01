//! `erpHost` comment calls (M5), without JavaScript: anchors of the selection, the marks of the
//! threads the shell shows, focusing a thread's cells, and placing anchors again after the
//! workbook changed. Anchors and marks live outside the document: nothing here edits or saves
//! it. The engine's [`gridcraft_engine::live::anchor`] does the placing.

use gridcraft_engine::live::anchor::{Anchor, Placed};
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::comment_marks::MarkThread;
use serde_json::{Value as Json, json};

/// One of the shell's comment calls, applied at the start of the next frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CommentMsg {
    /// `showComments(threads)`.
    Show(Json),
    /// `focusAnchor(anchor, threadId?)`.
    Focus(Json, Option<String>),
}

fn thread_id(thread: &Json) -> Option<String> {
    thread.get("id").or_else(|| thread.get("threadId")).and_then(Json::as_str).filter(|id| !id.is_empty()).map(str::to_string)
}

/// `anchorOfSelection()`: the anchor of the selection in document `doc`, or null.
pub(crate) fn anchor_of_selection(app: &SheetApp, doc: Option<u64>) -> Json {
    doc.and_then(|doc| app.session.comment_anchor(doc)).map_or(Json::Null, |a| a.to_json())
}

pub(crate) fn deliver(app: &mut SheetApp, doc: Option<u64>, messages: Vec<CommentMsg>) {
    let Some(doc) = doc else { return };
    for message in messages {
        match message {
            CommentMsg::Show(threads) => show(app, doc, &threads),
            CommentMsg::Focus(anchor, thread) => {
                focus(app, doc, &anchor, thread);
            }
        }
    }
}

/// `showComments(threads)`: marks for the open threads, nothing for resolved or orphaned ones.
pub(crate) fn show(app: &mut SheetApp, doc: u64, threads: &Json) {
    let open = threads
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t.get("status").and_then(Json::as_str).is_none_or(|s| s == "open"))
        .filter_map(|t| Some(MarkThread { id: thread_id(t)?, anchor: Anchor::parse(t.get("anchor")?)? }))
        .collect();
    app.comment_marks.show(doc, open);
}

/// `focusAnchor(anchor, threadId)`: shows document `doc` and selects the anchor's cells. `false`
/// when the anchor has no place in it.
pub(crate) fn focus(app: &mut SheetApp, doc: u64, anchor: &Json, thread: Option<String>) -> bool {
    let Some(anchor) = Anchor::parse(anchor) else { return false };
    let Placed::At { sheet, range, .. } = app.session.place_comment_anchor(doc, &anchor) else { return false };
    let Some(index) = app.session.documents().iter().position(|d| d.uid == doc) else { return false };
    if app.session.active_index() != index {
        app.session.set_active(index);
    }
    if let Err(e) = app.run("selection.set", json!({"range": range.a1(), "sheet": sheet})) {
        log::warn!("erpHost.focusAnchor: couldn't select: {e}");
        return false;
    }
    app.grid.ensure_visible = true;
    app.comment_marks.focused = thread;
    true
}

/// `resolveAnchors(threads)`: each thread's place in document `doc` now, as
/// `{id, threadId, status: "open" | "orphaned", anchor?}`; `anchor` only when it changed. No
/// answers before the document is open: its threads aren't orphaned, just not placed yet.
pub(crate) fn resolve(app: &SheetApp, doc: Option<u64>, threads: &Json) -> Json {
    let Some(doc) = doc.filter(|doc| app.session.documents().iter().any(|d| d.uid == *doc)) else { return Json::Array(Vec::new()) };
    let updates = threads
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let id = thread_id(t)?;
            let old = Anchor::parse(t.get("anchor")?)?;
            Some(match app.session.place_comment_anchor(doc, &old) {
                Placed::At { anchor, .. } if *anchor != old => json!({"id": id, "threadId": id, "status": "open", "anchor": anchor.to_json()}),
                Placed::At { .. } => json!({"id": id, "threadId": id, "status": "open"}),
                Placed::Orphaned => json!({"id": id, "threadId": id, "status": "orphaned"}),
            })
        })
        .collect();
    Json::Array(updates)
}
