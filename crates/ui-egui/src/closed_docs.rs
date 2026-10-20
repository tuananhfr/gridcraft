//! Documents closed in this session, kept so File ▸ Open ▸ Reopen Closed Document (Ctrl+Shift+T)
//! can bring the most recent one back as it was, unsaved changes included.

use std::collections::VecDeque;

use gridcraft_engine::io::FileKind;
use serde_json::{Value as Json, json};

use crate::SheetApp;

/// Most closed documents kept.
pub const MAX_CLOSED: usize = 10;
/// Most bytes kept for them together; the oldest go first.
pub const MAX_CLOSED_BYTES: usize = 64 * 1024 * 1024;
/// Why reopening did nothing (the interface translates it by this exact text).
pub const NOTHING_TO_REOPEN: &str = "There's no closed document to reopen.";

/// A closed document as it was when it closed.
#[derive(Clone, Debug, PartialEq)]
pub struct ClosedDoc {
    pub title: String,
    /// The workbook as a draft is written ([`crate::drafts::draft_bytes`]).
    pub bytes: Vec<u8>,
    pub unsaved: bool,
    pub path: Option<String>,
    pub content_kind: Option<FileKind>,
}

/// The closed documents, most recent first, within a count and a byte budget.
#[derive(Debug)]
pub struct ClosedDocs {
    docs: VecDeque<ClosedDoc>,
    max_docs: usize,
    max_bytes: usize,
}

impl Default for ClosedDocs {
    fn default() -> Self {
        ClosedDocs::with_limits(MAX_CLOSED, MAX_CLOSED_BYTES)
    }
}

impl ClosedDocs {
    pub fn with_limits(max_docs: usize, max_bytes: usize) -> ClosedDocs {
        ClosedDocs { docs: VecDeque::new(), max_docs, max_bytes }
    }

    /// Keeps `doc` as the most recent, dropping the oldest past the limits. A document larger
    /// than the whole budget isn't kept.
    pub fn push(&mut self, doc: ClosedDoc) {
        if doc.bytes.len() > self.max_bytes {
            return;
        }
        self.docs.push_front(doc);
        while self.docs.len() > self.max_docs || self.bytes() > self.max_bytes {
            self.docs.pop_back();
        }
    }

    pub fn latest(&self) -> Option<&ClosedDoc> {
        self.docs.front()
    }

    pub fn pop(&mut self) -> Option<ClosedDoc> {
        self.docs.pop_front()
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.docs.iter().map(|d| d.bytes.len()).sum()
    }
}

impl SheetApp {
    /// What `file.close` with `params` will close, serialized; `None` when it will ask first,
    /// or the document was opened with its password (never kept decrypted).
    pub(crate) fn closing_snapshot(&self, params: &Json) -> Option<ClosedDoc> {
        let i = params.get("index").and_then(Json::as_u64).map_or(self.session.active_index(), |i| i as usize);
        let d = self.session.documents().get(i)?;
        let force = params.get("force").and_then(Json::as_bool).unwrap_or(false);
        if (d.is_dirty() && !force) || self.decrypted.contains(&d.uid) {
            return None;
        }
        let bytes = crate::drafts::draft_bytes(&d.wb).map_err(|e| log::warn!("closed document not kept: {e}")).ok()?;
        Some(ClosedDoc { title: d.display_title(), bytes, unsaved: d.is_dirty(), path: d.path.clone(), content_kind: d.content_kind })
    }

    /// Whether Reopen Closed Document has something to reopen.
    pub fn can_reopen_closed(&self) -> bool {
        !self.closed_docs.is_empty()
    }

    /// The most recently closed document's title.
    pub fn last_closed_title(&self) -> Option<&str> {
        self.closed_docs.latest().map(|d| d.title.as_str())
    }

    /// Reopens the most recently closed document as a new tab, under its name and unsaved if it
    /// was. It stays in the list when it can't open (ten documents already open).
    pub(crate) fn reopen_closed(&mut self) -> Result<Json, String> {
        self.session.ensure_room().map_err(|e| e.to_string())?;
        let doc = self.closed_docs.pop().ok_or(NOTHING_TO_REOPEN)?;
        // A neutral name: the title may end in .csv, which `file.open` would parse as text.
        let params = Json::Object(serde_json::Map::from_iter([
            ("name".to_string(), Json::String("Reopened.xlsx".into())),
            ("base64".to_string(), Json::String(gridcraft_engine::io::base64_encode(&doc.bytes))),
        ]));
        if let Err(e) = self.run("file.open", params) {
            self.closed_docs.push(doc);
            return Err(e);
        }
        self.backstage = None;
        let Some(d) = self.session.active_mut() else { return Err(NOTHING_TO_REOPEN.into()) };
        d.title = doc.title.clone();
        d.path = doc.path;
        d.content_kind = doc.content_kind;
        if doc.unsaved {
            // A different `Arc` from the one shown makes it dirty, as for a recovered draft.
            d.saved = std::sync::Arc::new((*d.wb).clone());
        }
        Ok(json!({"title": doc.title, "unsaved": doc.unsaved}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(title: &str, len: usize) -> ClosedDoc {
        ClosedDoc { title: title.into(), bytes: vec![0; len], unsaved: false, path: None, content_kind: None }
    }

    fn titles(c: &ClosedDocs) -> Vec<&str> {
        c.docs.iter().map(|d| d.title.as_str()).collect()
    }

    #[test]
    fn keeps_the_ten_most_recent() {
        let mut c = ClosedDocs::default();
        for i in 1..=12 {
            c.push(doc(&format!("Book{i}"), 10));
        }
        assert_eq!(c.len(), MAX_CLOSED);
        assert_eq!(c.latest().map(|d| d.title.as_str()), Some("Book12"));
        assert_eq!(titles(&c).last(), Some(&"Book3"), "the two oldest were dropped");
    }

    #[test]
    fn keeps_within_the_byte_budget_oldest_dropped_first() {
        assert_eq!(MAX_CLOSED_BYTES, 64 * 1024 * 1024);
        let mut c = ClosedDocs::with_limits(MAX_CLOSED, 100);
        c.push(doc("a", 40));
        c.push(doc("b", 40));
        c.push(doc("c", 40));
        assert_eq!(titles(&c), ["c", "b"]);
        assert!(c.bytes() <= 100);
        c.push(doc("huge", 101));
        assert_eq!(titles(&c), ["c", "b"], "a document over the whole budget isn't kept");
        c.push(doc("whole", 100));
        assert_eq!(titles(&c), ["whole"]);
    }

    #[test]
    fn the_real_byte_budget_drops_the_oldest() {
        let mut c = ClosedDocs::default();
        for i in 0..5 {
            c.push(doc(&format!("{i}"), 20 * 1024 * 1024));
        }
        assert_eq!(titles(&c), ["4", "3", "2"]);
        assert!(c.bytes() <= MAX_CLOSED_BYTES);
    }

    #[test]
    fn pop_takes_the_most_recent() {
        let mut c = ClosedDocs::default();
        assert!(c.pop().is_none());
        c.push(doc("a", 1));
        c.push(doc("b", 1));
        assert_eq!(c.pop().map(|d| d.title), Some("b".into()));
        assert_eq!(c.pop().map(|d| d.title), Some("a".into()));
        assert!(c.is_empty());
    }
}
