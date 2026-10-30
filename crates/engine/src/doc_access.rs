//! Access a page host grants to one open document (web: the ERPCons office shell's view and
//! comment modes). Session state only: unlike sheet protection it is never written into the file,
//! so the same workbook opens editable for someone else.

use crate::cmd::CommandSpec;

/// What the host lets the user do with a document. The default is full access.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DocAccess {
    /// Nothing may change the document; navigation, selection, zoom and find still work.
    pub read_only: bool,
    /// Save As, Export, Print and copying content out of it are refused.
    pub no_download: bool,
    /// The host's `comment` mode, read-only until comments come to the web: only its notice differs.
    pub commenter: bool,
}

/// Why an edit of a read-only document was refused (the interface translates it by this text).
pub const READ_ONLY: &str = "This document is read-only.";
/// Why an edit of a `comment`-mode document was refused.
pub const COMMENTS_LATER: &str = "Commenting on the web is coming later; for now this document is view-only.";
/// Why Save As, Export, Print or Copy was refused.
pub const NO_DOWNLOAD: &str = "Downloading, printing and copying are turned off for this document.";

/// Undoable commands that only move the view; in a read-only document they stay out of the history.
const VIEW_ONLY: &[&str] = &["view.zoom", "view.zoom100", "view.zoomToSelection", "link.open"];

/// Commands outside the undo history that still change the document, start an edit or run code
/// that could (a macro, a recording that is saved into the workbook).
const EDITS: &[&str] = &[
    "edit.undo",
    "edit.redo",
    "edit.cut",
    "edit.repeat",
    "edit.beginEdit",
    "file.save",
    "formulas.calculateNow",
    "formulas.calculateSheet",
    "formulas.calculateSelection",
    "view.customViews",
    "automate.recordActions",
    "macros.run",
    "draw.pen",
    "draw.eraser",
    "draw.lasso",
];

/// Commands that hand the document's content to the user outside the editor.
const DOWNLOADS: &[&str] = &[
    "edit.copy",
    "edit.cut",
    "file.saveAs",
    "file.saveBytes",
    "file.exportCsv",
    "file.exportHtml",
    "file.exportPdf",
    "file.print",
    "file.printPreview",
];

impl DocAccess {
    pub fn is_full(self) -> bool {
        self == DocAccess::default()
    }

    /// The notice for an edit this read-only document refuses.
    pub fn read_only_notice(self) -> &'static str {
        if self.commenter { COMMENTS_LATER } else { READ_ONLY }
    }

    /// Why `spec` is refused on a document with this access, if it is.
    pub fn refusal(self, spec: &CommandSpec) -> Option<&'static str> {
        let id = spec.id;
        if self.read_only && ((spec.undoable && !VIEW_ONLY.contains(&id)) || EDITS.contains(&id)) {
            return Some(self.read_only_notice());
        }
        if self.no_download && DOWNLOADS.contains(&id) {
            return Some(NO_DOWNLOAD);
        }
        None
    }
}
