//! The `erpHost` vocabulary (`erpcons-office` `docs/erp-host.md`, v1 and the v2 access modes),
//! free of JavaScript so the rules can be tested natively.

use gridcraft_engine::doc_access::DocAccess;
use serde_json::Value;

/// The `erpHost.version` values this fork speaks.
pub(crate) const API_VERSIONS: [u32; 2] = [1, 2];

/// `doc.mode` / `setMode` (v2). In phase 2 `comment` treats the content like `view`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    Edit,
    Comment,
    View,
}

impl Mode {
    /// Absent is `edit`; a value this fork doesn't know reads as `view`, which can't lose anything.
    pub(crate) fn parse(value: Option<&str>) -> Mode {
        match value {
            None | Some("edit") => Mode::Edit,
            Some("comment") => Mode::Comment,
            Some(_) => Mode::View,
        }
    }

    /// The access the document gets; only `canDownload: false` turns downloads off.
    pub(crate) fn access(self, can_download: bool) -> DocAccess {
        DocAccess { read_only: self != Mode::Edit, no_download: !can_download, commenter: self == Mode::Comment }
    }
}
/// From v2 the shell's own Share serves its documents; the title bar's would be a second one.
pub(crate) fn shell_shares(version: u32) -> bool {
    version >= 2
}

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
    /// v2 `mode` and `canDownload`; full access under v1.
    pub(crate) access: DocAccess,
    /// v2: a newer version of a document already open; keep what the user was looking at.
    pub(crate) preserve_view: bool,
}

/// What the shell asked for, applied in order at the start of the next frame.
pub(crate) enum Request {
    Open(OpenRequest),
    /// `setMode(docId, mode, canDownload)` (v2).
    Mode(String, DocAccess),
    /// `setUser(user)` (v2): the account's name; `None` (null) writes the local preference's.
    User(Option<String>),
}

/// The calls the fork makes on `window.erpHost`.
pub(crate) trait Shell {
    /// `save(docId, bytes, {revision, reason})`; `reply` gets what its promise resolves to.
    fn save(&self, doc_id: &str, bytes: Vec<u8>, revision: u64, reason: Reason, reply: super::docs::Reply);
    fn dirty(&self, doc_id: &str, dirty: bool);
    fn opened(&self, doc_id: &str, ok: bool, reason: Option<&str>);
    fn closed(&self, doc_id: &str);
}
