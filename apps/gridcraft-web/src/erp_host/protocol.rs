//! The `erpHost` v1 vocabulary (`erpcons-office` `docs/erp-host.md`), free of JavaScript so the
//! rules can be tested natively.

use serde_json::Value;

pub(crate) const API_VERSION: f64 = 1.0;
/// The only format the online store holds for workbooks in v1.
pub(crate) const FORMAT: &str = "xlsx";

/// Why a save was started; the order is the precedence when saves coalesce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Reason {
    Auto,
    Flush,
    Manual,
}

impl Reason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Reason::Auto => "auto",
            Reason::Flush => "flush",
            Reason::Manual => "manual",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SaveResult {
    Saved { version: Option<u64> },
    Conflict,
    Offline,
    Failed { retryable: bool, message: String },
}

impl SaveResult {
    /// A `SaveResult` as the shell resolved it. Anything else reads as offline: the shell keeps
    /// the bytes and the document stays dirty, which loses nothing.
    pub(crate) fn from_json(value: &Value) -> SaveResult {
        let version = value.get("version").and_then(Value::as_u64);
        match value.get("status").and_then(Value::as_str) {
            Some("saved") => SaveResult::Saved { version },
            Some("conflict") => SaveResult::Conflict,
            Some("failed") => SaveResult::Failed {
                retryable: value.get("retryable").and_then(Value::as_bool).unwrap_or(false),
                message: value.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
            },
            _ => SaveResult::Offline,
        }
    }
}

/// A document the shell handed over with `open(doc)`.
pub(crate) struct OpenRequest {
    pub(crate) doc_id: String,
    pub(crate) version: Option<u64>,
    pub(crate) title: String,
    pub(crate) format: String,
    pub(crate) bytes: Option<Vec<u8>>,
}

/// The calls the fork makes on `window.erpHost`.
pub(crate) trait Shell {
    /// `save(docId, bytes, {revision, reason})`; `reply` gets what its promise resolves to.
    fn save(&self, doc_id: &str, bytes: Vec<u8>, revision: u64, reason: Reason, reply: super::docs::Reply);
    fn dirty(&self, doc_id: &str, dirty: bool);
    fn opened(&self, doc_id: &str, ok: bool, reason: Option<&str>);
    fn closed(&self, doc_id: &str);
}
