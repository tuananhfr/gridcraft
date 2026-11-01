//! Region co-editing (ERPCons Office online, `erpHost` v3): every edit of the shared document
//! names the region it touched ([`Region`]), runs only once that region's lock is ours, and
//! goes to the other editors as a payload holding the region's whole content; theirs are applied
//! here. Undo takes back only this user's steps. A [`Session`] without [`Session::live`] (the
//! desktop app, a shell without v3) never gets here and edits as before.
//!
//! The host bridge drains [`Live::take_calls`] into the shell and feeds its answers back through
//! the `live_*` methods of [`Session`].

pub mod anchor;
mod apply;
mod content;
mod guard;
mod payload;
pub mod region;
mod undo;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use gridcraft_core::CellRef;
use gridcraft_model::Workbook;
use serde_json::Value as Json;

pub use region::Region;

use crate::{Selection, Session};
use content::Scope;
use payload::StepEdit;

/// A call the fork makes on the shell.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    /// `regionWillEdit(region)`; answer with [`Session::live_answer`].
    WillEdit(String),
    /// `regionChanged(region, payload)`.
    Changed(String, String),
    /// `regionDidEndEdit(region)`.
    DidEndEdit(String),
    /// `selectionChanged(region, range)`.
    Selection(String, String),
}

/// A lock another member holds (`showLocks`).
#[derive(Clone, Debug, PartialEq)]
pub struct Lock {
    pub region: String,
    pub holder: String,
    pub name: String,
    pub colour: String,
}

/// Another member's selection (`showCursors`): `range` lists A1 ranges separated by commas.
#[derive(Clone, Debug, PartialEq)]
pub struct Cursor {
    pub author: String,
    pub name: String,
    pub colour: String,
    pub region: String,
    pub range: String,
}

/// What waits for a lock: run again once it is granted.
#[derive(Clone, Debug)]
enum Action {
    Command { id: String, params: Json, selection: Selection, sheet: usize },
    Undo,
    Redo,
}

/// A sheet's row/column layout as the room knows it: the token of the last sheet payload sent or
/// applied here, and the structure sequence number each token stands for in this workbook.
#[derive(Clone, Debug, Default)]
struct Layout {
    token: Option<String>,
    seqs: Vec<(String, u64)>,
}

impl Layout {
    fn seq_of(&self, token: &str) -> Option<u64> {
        self.seqs.iter().rev().find(|(t, _)| t == token).map(|(_, s)| *s)
    }
    fn set(&mut self, token: String, seq: u64) {
        self.seqs.push((token.clone(), seq));
        if self.seqs.len() > LAYOUTS_KEPT {
            self.seqs.remove(0);
        }
        self.token = Some(token);
    }
}

const LAYOUTS_KEPT: usize = 64;

#[derive(Clone, Debug)]
struct Offline {
    region: String,
    sheet: String,
    cell: CellRef,
    token: Option<String>,
    data: Option<content::CellData>,
}

const OFFLINE_KEPT: usize = 1_000;

/// The structural edits of one `sheet:` edit session, cut into chunks named by the token each
/// payload ended at, so a receiver that missed a coalesced payload still finds where it is.
#[derive(Clone, Debug)]
struct SheetEdit {
    base: Option<String>,
    last_seq: u64,
    chunks: Vec<(String, Vec<StepEdit>)>,
}

/// A held region nobody edits for this long is handed back.
const IDLE_CELL_MS: f64 = 10_000.0;
const IDLE_SHEET_MS: f64 = 5_000.0;
const IDLE_BOOK_MS: f64 = 2_000.0;

/// The co-editing state of the one document the shell shares ([`Session::live`]).
#[derive(Debug)]
pub struct Live {
    doc: u64,
    prefix: String,
    counter: u64,
    calls: Vec<Call>,
    /// Regions whose lock is ours, with when they were last edited.
    held: BTreeMap<Region, f64>,
    /// `regionWillEdit` asked and not answered yet, with what waits for it.
    asking: BTreeMap<Region, Vec<Action>>,
    others: Vec<Lock>,
    cursors: Vec<Cursor>,
    feedback: Option<Feedback>,
    editor: Option<Region>,
    editor_denied: bool,
    layouts: HashMap<String, Layout>,
    /// The layout token of the content as opened (every sheet starts there).
    origin: Option<String>,
    /// Cell edits made while not in the room, kept until their lock is ours again: a sheet
    /// payload replayed before them doesn't hold them.
    offline: Vec<Offline>,
    online: bool,
    sheet_edits: HashMap<String, SheetEdit>,
    undo: Vec<undo::Step>,
    redo: Vec<undo::Step>,
    selection_sent: Option<(String, String)>,
    leader: bool,
    now: f64,
}

/// A message for the user: `text` is English for the interface to translate, with `{name}`
/// standing for `name`.
#[derive(Clone, Debug, PartialEq)]
pub struct Feedback {
    pub text: &'static str,
    pub name: String,
}

impl Feedback {
    fn plain(text: &'static str) -> Feedback {
        Feedback { text, name: String::new() }
    }
    fn locked_by(name: String) -> Feedback {
        Feedback { text: LOCKED_BY, name }
    }
    /// `text` (already translated) with the name put in.
    pub fn fill(&self, text: &str) -> String {
        text.replace("{name}", if self.name.is_empty() { "?" } else { &self.name })
    }
}

/// Why an edit can't go ahead now.
enum Verdict {
    Held,
    Blocked(String),
    Ask,
}

pub const LOCKED_BY: &str = "Someone else is editing this part: {name}.";
pub const LOCK_DENIED: &str = "Someone else is editing this part; your change was not made.";
pub const NOTHING_TO_UNDO: &str = "Nothing of yours left to undo.";

impl Live {
    /// Co-editing of document `doc` (uid). `prefix` names the layout tokens this client makes;
    /// `origin` names the layout of the content as opened, the same for every client that opened
    /// the same version (`v<version>`).
    pub fn new(doc: u64, prefix: &str, origin: Option<String>) -> Live {
        Live {
            doc,
            prefix: prefix.to_string(),
            counter: 0,
            calls: Vec::new(),
            held: BTreeMap::new(),
            asking: BTreeMap::new(),
            others: Vec::new(),
            cursors: Vec::new(),
            feedback: None,
            editor: None,
            editor_denied: false,
            layouts: HashMap::new(),
            origin,
            offline: Vec::new(),
            online: false,
            sheet_edits: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            selection_sent: None,
            leader: false,
            now: 0.0,
        }
    }

    pub fn doc(&self) -> u64 {
        self.doc
    }
    /// The shell opened another copy of the document (a reload): co-editing follows it.
    pub fn set_doc(&mut self, doc: u64, origin: Option<String>) {
        if doc != self.doc {
            self.doc = doc;
            self.origin = origin;
            self.offline.clear();
            self.layouts.clear();
            self.sheet_edits.clear();
            self.undo.clear();
            self.redo.clear();
        }
    }
    pub fn take_calls(&mut self) -> Vec<Call> {
        std::mem::take(&mut self.calls)
    }
    /// A message for the user.
    pub fn take_feedback(&mut self) -> Option<Feedback> {
        self.feedback.take()
    }
    /// The in-cell editor's cell went to someone else: the interface closes the editor.
    pub fn take_editor_denied(&mut self) -> bool {
        std::mem::take(&mut self.editor_denied)
    }
    pub fn others(&self) -> &[Lock] {
        &self.others
    }
    pub fn cursors(&self) -> &[Cursor] {
        &self.cursors
    }
    pub fn is_leader(&self) -> bool {
        self.leader
    }
    pub fn set_leader(&mut self, leader: bool) {
        self.leader = leader;
    }
    /// `showLocks`: `mine` lists the regions whose lock is ours (an edit made offline there has
    /// gone to the room by now), `others` the rest.
    pub fn set_locks(&mut self, others: Vec<Lock>, mine: &[String]) {
        self.others = others;
        self.offline.retain(|o| !mine.contains(&o.region));
    }
    /// `showPresence` listed this page (`true`) or the room is gone (`false`).
    pub fn set_online(&mut self, online: bool) {
        self.online = online;
    }

    fn layout(&mut self, uid: &str) -> &mut Layout {
        let origin = self.origin.clone();
        self.layouts.entry(uid.to_string()).or_insert_with(|| {
            let mut layout = Layout::default();
            if let Some(o) = origin {
                layout.set(o, 0);
            }
            layout
        })
    }
    pub fn set_cursors(&mut self, cursors: Vec<Cursor>) {
        self.cursors = cursors;
    }
    pub fn holds(&self, region: &str) -> bool {
        Region::parse(region).is_some_and(|r| self.held.contains_key(&r))
    }

    fn token(&mut self) -> String {
        self.counter += 1;
        format!("{}.{}", if self.prefix.is_empty() { "c" } else { &self.prefix }, self.counter)
    }

    /// The other member whose lock keeps `region` from us.
    pub fn blocker(&self, region: &Region) -> Option<&Lock> {
        self.others.iter().find(|l| Region::parse(&l.region).is_some_and(|r| r.clashes(region)))
    }

    fn verdict(&self, region: &Region) -> Verdict {
        if self.held.keys().any(|h| h.covers(region)) {
            return Verdict::Held;
        }
        match self.blocker(region) {
            Some(lock) => Verdict::Blocked(lock.name.clone()),
            None => Verdict::Ask,
        }
    }

    fn ask(&mut self, region: Region, action: Option<Action>) {
        let fresh = !self.asking.contains_key(&region);
        let waiting = self.asking.entry(region.clone()).or_default();
        waiting.extend(action);
        if fresh {
            self.calls.push(Call::WillEdit(region.id()));
        }
    }

    fn touch(&mut self, region: &Region) {
        let now = self.now;
        for (held, at) in self.held.iter_mut() {
            if held.covers(region) {
                *at = now;
            }
        }
    }

    fn end(&mut self, region: &Region) {
        if self.held.remove(region).is_some() {
            self.calls.push(Call::DidEndEdit(region.id()));
        }
        if let Region::Sheet(uid) | Region::Cell { sheet: uid, .. } = region
            && !self.held.keys().any(|h| matches!(h, Region::Sheet(s) if s == uid))
        {
            self.sheet_edits.remove(uid);
        }
        if *region == Region::Book {
            self.sheet_edits.clear();
        }
    }

    /// Sends the content `scope` names in `after` as `region`'s payload.
    fn emit(&mut self, before: &Workbook, after: &Workbook, scope: &Scope, region: &Region) {
        let payload = match (scope, region) {
            (Scope::Cell { sheet, cell }, Region::Cell { sheet: uid, .. }) => {
                let token = self.layout(uid).token.clone();
                let data = content::data_at(after, *sheet, *cell);
                if !self.online {
                    self.offline.push(Offline { region: region.id(), sheet: uid.clone(), cell: *cell, token: token.clone(), data: data.clone() });
                    if self.offline.len() > OFFLINE_KEPT {
                        self.offline.remove(0);
                    }
                }
                Ok(payload::encode_cell(token.as_deref(), data.as_ref()))
            }
            (Scope::Sheet { sheet, others, names }, Region::Sheet(uid)) => {
                let chunks = self.sheet_chunks(uid, before, after);
                let edit = self.sheet_edits.get(uid);
                let base = edit.and_then(|e| e.base.clone());
                payload::encode_sheet(after, &payload::SheetChange { sheet: *sheet, base: base.as_deref(), chunks: &chunks, others, names: *names })
            }
            (Scope::Book, Region::Book) => {
                self.layouts.clear();
                self.origin = None;
                self.sheet_edits.clear();
                payload::encode_book(after)
            }
            _ => Err("region and change disagree".into()),
        };
        match payload {
            Ok(p) => self.calls.push(Call::Changed(region.id(), p)),
            Err(e) => log::warn!("live: no payload for an edit: {e}"),
        }
        self.touch(region);
    }

    /// The structural edits of the current `sheet:<uid>` session, including those `after` adds.
    fn sheet_chunks(&mut self, uid: &str, before: &Workbook, after: &Workbook) -> Vec<(String, Vec<StepEdit>)> {
        let base = self.layout(uid).token.clone();
        let edit =
            self.sheet_edits.entry(uid.to_string()).or_insert_with(|| SheetEdit { base, last_seq: before.structure_seq(), chunks: Vec::new() });
        let last_seq = edit.last_seq;
        let steps = after.structure.steps_since(last_seq);
        let token = match steps {
            Some(steps) if steps.is_empty() => None,
            Some(steps) => Some((steps, after.structure_seq())),
            // An undone branch: the edits since the base can't be listed; receivers just take the layout.
            None => {
                if let Some(e) = self.sheet_edits.get_mut(uid) {
                    e.base = None;
                    e.chunks.clear();
                }
                Some((Vec::new(), after.structure_seq()))
            }
        };
        if let Some((steps, seq)) = token {
            let t = self.token();
            if let Some(e) = self.sheet_edits.get_mut(uid) {
                e.chunks.push((t.clone(), steps));
                e.last_seq = seq;
            }
            self.layout(uid).set(t, seq);
        }
        self.sheet_edits.get(uid).map(|e| e.chunks.clone()).unwrap_or_default()
    }
}

/// The region a change of `wb` falls in.
fn region_of(scope: &Scope, wb: &Workbook) -> Option<Region> {
    let uid = |i: usize| wb.sheet(i).map(|s| s.uid.clone());
    match scope {
        Scope::None => None,
        Scope::Cell { sheet, cell } => Some(Region::Cell { sheet: uid(*sheet)?, cell: *cell }),
        Scope::Sheet { sheet, .. } => Some(Region::Sheet(uid(*sheet)?)),
        Scope::Book => Some(Region::Book),
    }
}

/// A command's snapshot of the document before it ran: (uid, workbook, selection, stale cells).
pub(crate) type Before = (u64, Arc<Workbook>, Selection, std::collections::BTreeSet<gridcraft_calc::Key>);

impl Session {
    /// Whether co-editing serves the active document.
    pub fn live_serves_active(&self) -> bool {
        matches!((&self.live, self.active()), (Some(live), Some(d)) if live.doc == d.uid)
    }

    /// Called by [`Session::execute`] after an outermost command: lets its change through when
    /// its region's lock is ours, else takes it back (`true`) until the lock is granted.
    pub(crate) fn live_gate(&mut self, id: &str, params: &Json, before: Option<&Before>) -> bool {
        let Some((uid, wb_before, sel, stale)) = before else { return false };
        let Some(mut live) = self.live.take() else { return false };
        let reverted = self.live_gate_with(&mut live, id, params, (*uid, wb_before, sel, stale));
        self.live = Some(live);
        reverted
    }

    fn live_gate_with(
        &mut self,
        live: &mut Live,
        id: &str,
        params: &Json,
        (uid, wb_before, sel, stale): (u64, &Arc<Workbook>, &Selection, &std::collections::BTreeSet<gridcraft_calc::Key>),
    ) -> bool {
        if live.doc != uid {
            return false;
        }
        let Some(d) = self.docs.iter_mut().find(|d| d.uid == uid) else { return false };
        if Arc::ptr_eq(&d.wb, wb_before) || d.access.locked() {
            return false;
        }
        let scope = content::scope(wb_before, &d.wb, wb_before.active_sheet);
        let Some(region) = region_of(&scope, &d.wb) else { return false };
        match live.verdict(&region) {
            Verdict::Held => {
                live.emit(wb_before, &d.wb, &scope, &region);
                live.record(undo::Step::new(region, wb_before.clone(), d.wb.clone()));
                false
            }
            verdict => {
                d.wb = wb_before.clone();
                d.selection = sel.clone();
                d.calc.rebuild(&d.wb);
                d.calc.stale = stale.clone();
                match verdict {
                    Verdict::Blocked(name) => live.feedback = Some(Feedback::locked_by(name)),
                    _ => live.ask(
                        region,
                        Some(Action::Command { id: id.to_string(), params: params.clone(), selection: sel.clone(), sheet: wb_before.active_sheet }),
                    ),
                }
                true
            }
        }
    }

    /// The shell answered `regionWillEdit(region)`.
    pub fn live_answer(&mut self, region: &str, granted: bool) {
        let Some(region) = Region::parse(region) else { return };
        let actions = {
            let Some(live) = self.live.as_mut() else { return };
            let actions = live.asking.remove(&region).unwrap_or_default();
            if granted {
                live.held.insert(region.clone(), live.now);
            } else {
                live.feedback = Some(Feedback::plain(LOCK_DENIED));
                if live.editor.as_ref() == Some(&region) {
                    live.editor = None;
                    live.editor_denied = true;
                }
            }
            actions
        };
        if granted {
            for action in actions {
                self.live_run(action);
            }
        }
    }

    fn live_run(&mut self, action: Action) {
        match action {
            Action::Command { id, params, selection, sheet } => {
                let Some(doc) = self.live.as_ref().map(|l| l.doc) else { return };
                let Some(i) = self.docs.iter().position(|d| d.uid == doc) else { return };
                let active = self.active;
                let (user_sel, user_sheet) = match self.docs.get_mut(i) {
                    Some(d) => {
                        let keep = (d.selection.clone(), d.wb.active_sheet);
                        d.selection = selection;
                        if d.wb.active_sheet != sheet && sheet < d.wb.sheets.len() {
                            Arc::make_mut(&mut d.wb).active_sheet = sheet;
                        }
                        keep
                    }
                    None => return,
                };
                self.active = i;
                if let Err(e) = self.execute(&id, params) {
                    log::warn!("live: a command waiting for its lock failed: {e}");
                }
                self.active = active;
                if let Some(d) = self.docs.get_mut(i) {
                    d.selection = user_sel;
                    if d.wb.active_sheet != user_sheet && user_sheet < d.wb.sheets.len() {
                        Arc::make_mut(&mut d.wb).active_sheet = user_sheet;
                    }
                }
            }
            Action::Undo => {
                let _ = self.live_undo(false);
            }
            Action::Redo => {
                let _ = self.live_undo(true);
            }
        }
    }

    /// `lockDenied(region)`: a lock we held or asked for is someone else's now.
    pub fn live_lock_denied(&mut self, region: &str) {
        let Some(region) = Region::parse(region) else { return };
        let Some(live) = self.live.as_mut() else { return };
        live.held.remove(&region);
        live.asking.remove(&region);
        live.feedback = Some(Feedback::plain(LOCK_DENIED));
        if live.editor.as_ref() == Some(&region) {
            live.editor = None;
            live.editor_denied = true;
        }
    }

    /// The in-cell editor opens on the active cell: claims its lock. `Err(name)` when another
    /// member holds it (the editor stays closed).
    pub fn live_begin_cell_edit(&mut self, cell: CellRef) -> std::result::Result<(), String> {
        if !self.live_serves_active() {
            return Ok(());
        }
        let Some(uid) = self.active().and_then(|d| d.wb.active()).map(|s| s.uid.clone()) else { return Ok(()) };
        let Some(live) = self.live.as_mut() else { return Ok(()) };
        let region = Region::Cell { sheet: uid, cell };
        match live.verdict(&region) {
            Verdict::Blocked(name) => {
                live.feedback = Some(Feedback::locked_by(name.clone()));
                Err(name)
            }
            Verdict::Held => {
                live.touch(&region);
                live.editor = Some(region);
                Ok(())
            }
            Verdict::Ask => {
                live.ask(region.clone(), None);
                live.editor = Some(region);
                Ok(())
            }
        }
    }

    pub fn live_end_cell_edit(&mut self) {
        if let Some(live) = self.live.as_mut()
            && let Some(region) = live.editor.take()
        {
            live.touch(&region);
        }
    }

    /// Who holds the lock that keeps the active document's `cell` on sheet `sheet` from us.
    pub fn live_blocker(&self, sheet: usize, cell: CellRef) -> Option<&Lock> {
        let live = self.live.as_ref()?;
        let d = self.active().filter(|d| d.uid == live.doc)?;
        let uid = d.wb.sheet(sheet)?.uid.clone();
        live.blocker(&Region::Cell { sheet: uid, cell })
    }

    /// Once a frame: reports our selection and hands back the regions we left.
    pub fn live_tick(&mut self, now: f64) {
        let Some(live) = self.live.as_mut() else { return };
        live.now = now;
        let Some(d) = self.docs.iter().find(|d| d.uid == live.doc) else { return };
        let Some(sh) = d.wb.active() else { return };
        let active_uid = sh.uid.clone();
        let here = d.selection.active;
        if self.docs.get(self.active).is_some_and(|a| a.uid == live.doc) {
            let region = Region::Cell { sheet: active_uid.clone(), cell: here }.id();
            let range = d.selection.ranges.iter().map(|r| r.a1()).collect::<Vec<_>>().join(",");
            if live.selection_sent.as_ref() != Some(&(region.clone(), range.clone())) {
                live.calls.push(Call::Selection(region.clone(), range.clone()));
                live.selection_sent = Some((region, range));
            }
        }
        let leaving: Vec<Region> = live
            .held
            .iter()
            .filter(|(region, at)| {
                if live.editor.as_ref() == Some(*region) || live.asking.contains_key(*region) {
                    return false;
                }
                let idle = now - **at;
                match region {
                    Region::Cell { sheet, cell } => *sheet != active_uid || *cell != here || idle > IDLE_CELL_MS,
                    Region::Sheet(sheet) => *sheet != active_uid || idle > IDLE_SHEET_MS,
                    Region::Book => idle > IDLE_BOOK_MS,
                }
            })
            .map(|(r, _)| r.clone())
            .collect();
        for region in leaving {
            live.end(&region);
        }
    }

    /// `applyRemote(region, payload, author)`: another member's content of `region`.
    pub fn live_apply_remote(&mut self, region: &str, payload: &str) -> std::result::Result<(), String> {
        let Some(mut live) = self.live.take() else { return Err("not co-editing".into()) };
        let result = apply::apply(self, &mut live, region, payload);
        self.live = Some(live);
        result
    }
}
