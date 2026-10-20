//! Web draft autosave: when to hand an unsaved workbook to the page's draft store
//! (`window.erpDrafts`, kept by the ERPCons office shell). Pure timing logic over a clock the
//! caller passes in, so it runs and is tested off the browser; the web app does the JS calls.

use std::collections::HashMap;
use std::sync::Arc;

use gridcraft_engine::model::Workbook;

/// Quiet time after the last change before a draft is written.
pub const IDLE_MS: f64 = 2500.0;
/// Continuous editing still writes a draft this long after the first change not yet drafted.
pub const CHECKPOINT_MS: f64 = 30_000.0;
/// Drafts of a document are at least this many times its last encoding time apart, so a large
/// workbook doesn't spend most frames encoding.
pub const ADAPTIVE_FACTOR: f64 = 10.0;
/// What draft bytes are.
pub const FORMAT: &str = "xlsx";

/// One open document as the planner sees it this frame.
#[derive(Clone, Debug)]
pub struct DraftDoc {
    pub uid: u64,
    pub wb: Arc<Workbook>,
    pub saved: Arc<Workbook>,
    /// Dirty, or a save of it is still in flight.
    pub unsaved: bool,
    /// The file name the draft is listed under (`Book1.xlsx`).
    pub name: String,
}

/// What to do this frame.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Documents to encode and put now ([`Planner::record_put`] each afterwards).
    pub put: Vec<u64>,
    /// Draft keys to remove: their document was closed or no longer has unsaved work.
    pub remove: Vec<String>,
    /// Some document waits for its moment: tick again without input.
    pub waiting: bool,
}

#[derive(Debug)]
struct Track {
    key: Option<String>,
    /// The workbook seen last frame; a different `Arc` is a change. `revision` misses writes
    /// that skip the undo history, so identity is what counts.
    seen: Arc<Workbook>,
    last_change: f64,
    first_undrafted_change: Option<f64>,
    /// The workbook last put (or restored from a draft).
    drafted: Option<Arc<Workbook>>,
    /// The store holds a draft under this document's key.
    stored: bool,
    last_put: Option<f64>,
    last_cost: f64,
}

/// Per-document draft state across frames.
#[derive(Debug, Default)]
pub struct Planner {
    docs: HashMap<u64, Track>,
}

impl Planner {
    /// The key drafts of document `uid` are stored under.
    pub fn key(&self, uid: u64) -> String {
        self.docs.get(&uid).and_then(|t| t.key.clone()).unwrap_or_else(|| format!("doc-{uid}"))
    }

    /// Document `uid` was opened from the draft stored under `key`: keep using that key, and
    /// `wb` counts as drafted, so it isn't put again until it changes.
    pub fn adopt(&mut self, uid: u64, key: &str, wb: &Arc<Workbook>, now: f64) {
        self.docs.insert(
            uid,
            Track {
                key: Some(key.to_string()),
                seen: wb.clone(),
                last_change: now,
                first_undrafted_change: None,
                drafted: Some(wb.clone()),
                stored: true,
                last_put: Some(now),
                last_cost: 0.0,
            },
        );
    }

    /// Whether `doc` has unsaved work its last draft doesn't hold.
    pub fn is_pending(&self, doc: &DraftDoc) -> bool {
        let drafted = self.docs.get(&doc.uid).and_then(|t| t.drafted.as_ref());
        // Never drafted: compare with the saved state, so a clean workbook whose save is in
        // flight isn't drafted.
        doc.unsaved && !Arc::ptr_eq(&doc.wb, drafted.unwrap_or(&doc.saved))
    }

    /// Decides this frame's puts and removes. `editing`: a cell or text box editor is open,
    /// whose text isn't in the workbook yet; its commit is a change and gets drafted.
    pub fn plan(&mut self, docs: &[DraftDoc], now: f64, editing: bool) -> Plan {
        let mut plan = Plan::default();
        self.docs.retain(|uid, t| {
            let open = docs.iter().any(|d| d.uid == *uid && d.unsaved);
            if !open && t.stored {
                plan.remove.push(t.key.clone().unwrap_or_else(|| format!("doc-{uid}")));
                t.stored = false;
                t.drafted = None;
            }
            docs.iter().any(|d| d.uid == *uid)
        });
        for d in docs {
            let t = self.docs.entry(d.uid).or_insert_with(|| Track {
                key: None,
                seen: d.wb.clone(),
                last_change: now,
                first_undrafted_change: None,
                drafted: None,
                stored: false,
                last_put: None,
                last_cost: 0.0,
            });
            if !Arc::ptr_eq(&t.seen, &d.wb) {
                t.seen = d.wb.clone();
                t.last_change = now;
            }
        }
        for d in docs {
            let pending = self.is_pending(d);
            let Some(t) = self.docs.get_mut(&d.uid) else { continue };
            if !pending {
                t.first_undrafted_change = None;
                continue;
            }
            let first = *t.first_undrafted_change.get_or_insert(t.last_change);
            let due = now - t.last_change >= IDLE_MS || now - first >= CHECKPOINT_MS;
            let spaced = t.last_put.is_none_or(|at| now - at >= ADAPTIVE_FACTOR * t.last_cost);
            if due && spaced && !editing {
                plan.put.push(d.uid);
            } else {
                plan.waiting = true;
            }
        }
        plan
    }

    /// `wb` of document `uid` was handed to the store at `now`; encoding it took `cost_ms`.
    /// Recorded even when encoding or the store failed, so a failure isn't retried every frame
    /// but on the next change.
    pub fn record_put(&mut self, uid: u64, wb: &Arc<Workbook>, now: f64, cost_ms: f64) {
        let Some(t) = self.docs.get_mut(&uid) else { return };
        t.drafted = Some(wb.clone());
        t.stored = true;
        t.last_put = Some(now);
        t.last_cost = cost_ms.max(0.0);
        t.first_undrafted_change = None;
    }
}

/// The name a draft of the workbook titled `title` is listed under: what web Save names it.
pub fn draft_name(title: &str) -> String {
    let stem = std::path::Path::new(title).file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or("Book");
    format!("{stem}.{FORMAT}")
}

/// The draft bytes: the XLSX web Save writes, deflated fast. The JSON format would drop
/// preserved parts; the usual deflate was half the encoding time of a large workbook.
pub fn draft_bytes(wb: &Workbook) -> Result<Vec<u8>, String> {
    gridcraft_engine::io::save_xlsx_fast(wb).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Arc<Workbook> {
        Arc::new(Workbook::new())
    }

    /// A document `uid` whose saved state is `saved` and current state `wb`.
    fn doc(uid: u64, wb: &Arc<Workbook>, saved: &Arc<Workbook>) -> DraftDoc {
        DraftDoc { uid, wb: wb.clone(), saved: saved.clone(), unsaved: !Arc::ptr_eq(wb, saved), name: "Book1.xlsx".into() }
    }

    #[test]
    fn a_clean_workbook_is_never_drafted() {
        let mut p = Planner::default();
        let saved = book();
        for now in [0.0, 5_000.0, 60_000.0] {
            assert_eq!(p.plan(&[doc(1, &saved, &saved)], now, false), Plan::default());
        }
    }

    #[test]
    fn an_edit_is_drafted_once_the_workbook_is_idle() {
        let mut p = Planner::default();
        let saved = book();
        p.plan(&[doc(1, &saved, &saved)], 0.0, false);
        let edited = book();
        let plan = p.plan(&[doc(1, &edited, &saved)], 1_000.0, false);
        assert!(plan.put.is_empty() && plan.waiting, "waits for the idle time");
        assert!(p.plan(&[doc(1, &edited, &saved)], 3_000.0, false).put.is_empty(), "2000 ms since the change");
        assert_eq!(p.plan(&[doc(1, &edited, &saved)], 3_500.0, false).put, vec![1]);
        p.record_put(1, &edited, 3_500.0, 5.0);
        for now in [4_000.0, 10_000.0, 100_000.0] {
            assert_eq!(p.plan(&[doc(1, &edited, &saved)], now, false), Plan::default(), "an idle workbook isn't drafted again");
        }
    }

    #[test]
    fn continuous_editing_is_drafted_at_the_checkpoint() {
        let mut p = Planner::default();
        let saved = book();
        p.plan(&[doc(1, &saved, &saved)], 0.0, false);
        let mut put_at = None;
        let mut now = 0.0;
        while now <= 40_000.0 {
            now += 1_000.0;
            let wb = book();
            if p.plan(&[doc(1, &wb, &saved)], now, false).put == vec![1] {
                put_at = Some(now);
                break;
            }
        }
        assert_eq!(put_at, Some(31_000.0), "30 s after the first undrafted change (at 1 s)");
    }

    #[test]
    fn a_slow_encoding_spaces_out_the_drafts() {
        let mut p = Planner::default();
        let saved = book();
        p.plan(&[doc(1, &saved, &saved)], 0.0, false);
        let a = book();
        p.plan(&[doc(1, &a, &saved)], 0.0, false);
        assert_eq!(p.plan(&[doc(1, &a, &saved)], 2_500.0, false).put, vec![1]);
        p.record_put(1, &a, 2_500.0, 1_000.0);
        let b = book();
        p.plan(&[doc(1, &b, &saved)], 3_000.0, false);
        let plan = p.plan(&[doc(1, &b, &saved)], 6_000.0, false);
        assert!(plan.put.is_empty() && plan.waiting, "idle, but only 3.5 s since a put that took 1 s to encode");
        assert_eq!(p.plan(&[doc(1, &b, &saved)], 12_500.0, false).put, vec![1]);
    }

    #[test]
    fn a_five_second_encoding_is_put_at_most_every_fifty_seconds_of_typing() {
        let mut p = Planner::default();
        let saved = book();
        p.plan(&[doc(1, &saved, &saved)], 0.0, false);
        let mut puts = Vec::new();
        // An edit every 500 ms for four minutes, each draft taking 5 s to encode.
        let mut now = 0.0;
        while now < 240_000.0 {
            now += 500.0;
            let wb = book();
            if p.plan(&[doc(1, &wb, &saved)], now, false).put == vec![1] {
                p.record_put(1, &wb, now, 5_000.0);
                puts.push(now);
            }
        }
        assert_eq!(puts.first(), Some(&30_500.0), "the first draft waits for the checkpoint");
        assert!(puts.windows(2).all(|w| w[1] - w[0] >= 50_000.0), "{puts:?}");
        assert!(puts.len() >= 4, "still drafted while typing: {puts:?}");
        // Typing stops: the last edit is drafted once the spacing allows, then nothing more.
        let last = *puts.last().unwrap();
        let wb = book();
        p.plan(&[doc(1, &wb, &saved)], now, false);
        let mut after = Vec::new();
        while now < 400_000.0 {
            now += 500.0;
            if p.plan(&[doc(1, &wb, &saved)], now, false).put == vec![1] {
                p.record_put(1, &wb, now, 5_000.0);
                after.push(now);
            }
        }
        assert_eq!(after, vec![(last + 50_000.0).max(240_000.0 + IDLE_MS)]);
    }

    #[test]
    fn nothing_is_put_while_a_cell_is_being_edited() {
        let mut p = Planner::default();
        let saved = book();
        p.plan(&[doc(1, &saved, &saved)], 0.0, false);
        let edited = book();
        p.plan(&[doc(1, &edited, &saved)], 0.0, false);
        let plan = p.plan(&[doc(1, &edited, &saved)], 40_000.0, true);
        assert!(plan.put.is_empty() && plan.waiting);
        assert_eq!(p.plan(&[doc(1, &edited, &saved)], 40_001.0, false).put, vec![1]);
    }

    #[test]
    fn saving_or_closing_removes_the_draft_once() {
        let mut p = Planner::default();
        let saved = book();
        let edited = book();
        p.plan(&[doc(1, &edited, &saved)], 0.0, false);
        p.plan(&[doc(1, &edited, &saved)], 3_000.0, false);
        p.record_put(1, &edited, 3_000.0, 1.0);
        // Saved: the saved state is now the edited workbook.
        assert_eq!(p.plan(&[doc(1, &edited, &edited)], 4_000.0, false).remove, vec!["doc-1".to_string()]);
        assert_eq!(p.plan(&[doc(1, &edited, &edited)], 5_000.0, false), Plan::default());
        // Edited again, drafted again under the same key, then closed with Don't Save.
        let again = book();
        p.plan(&[doc(1, &again, &edited)], 6_000.0, false);
        assert_eq!(p.plan(&[doc(1, &again, &edited)], 9_000.0, false).put, vec![1]);
        p.record_put(1, &again, 9_000.0, 1.0);
        assert_eq!(p.key(1), "doc-1");
        assert_eq!(p.plan(&[], 10_000.0, false).remove, vec!["doc-1".to_string()]);
        assert_eq!(p.plan(&[], 11_000.0, false), Plan::default());
    }

    #[test]
    fn a_document_never_drafted_is_not_removed() {
        let mut p = Planner::default();
        let saved = book();
        let edited = book();
        p.plan(&[doc(1, &edited, &saved)], 0.0, false);
        assert_eq!(p.plan(&[], 1_000.0, false), Plan::default());
    }

    #[test]
    fn a_save_in_flight_keeps_the_draft_until_it_settles() {
        let mut p = Planner::default();
        let saved = book();
        let edited = book();
        p.plan(&[doc(1, &edited, &saved)], 0.0, false);
        p.plan(&[doc(1, &edited, &saved)], 3_000.0, false);
        p.record_put(1, &edited, 3_000.0, 1.0);
        // Undo back to the saved state while the save picker is still open.
        let in_flight = DraftDoc { unsaved: true, ..doc(1, &saved, &saved) };
        let plan = p.plan(&[in_flight], 4_000.0, false);
        assert!(plan.remove.is_empty(), "still unsaved while the save is in flight");
        assert_eq!(p.plan(&[doc(1, &saved, &saved)], 5_000.0, false).remove, vec!["doc-1".to_string()]);
    }

    #[test]
    fn a_clean_workbook_with_a_save_in_flight_is_not_drafted() {
        let mut p = Planner::default();
        let saved = book();
        let in_flight = DraftDoc { unsaved: true, ..doc(1, &saved, &saved) };
        p.plan(std::slice::from_ref(&in_flight), 0.0, false);
        assert_eq!(p.plan(&[in_flight], 10_000.0, false), Plan::default());
    }

    #[test]
    fn a_restored_draft_keeps_its_key_and_is_not_put_again_until_it_changes() {
        let mut p = Planner::default();
        let opened = book();
        let saved = book();
        p.adopt(7, "draft-abc", &opened, 0.0);
        assert_eq!(p.key(7), "draft-abc");
        for now in [0.0, 3_000.0, 60_000.0] {
            assert_eq!(p.plan(&[doc(7, &opened, &saved)], now, false), Plan::default());
        }
        let edited = book();
        p.plan(&[doc(7, &edited, &saved)], 61_000.0, false);
        assert_eq!(p.plan(&[doc(7, &edited, &saved)], 64_000.0, false).put, vec![7]);
        p.record_put(7, &edited, 64_000.0, 1.0);
        assert_eq!(p.plan(&[], 65_000.0, false).remove, vec!["draft-abc".to_string()], "closed: removed under the adopted key");
    }

    #[test]
    fn pending_lists_unsaved_work_the_last_draft_lacks() {
        let mut p = Planner::default();
        let saved = book();
        let edited = book();
        p.plan(&[doc(1, &edited, &saved)], 0.0, false);
        assert!(p.is_pending(&doc(1, &edited, &saved)));
        p.record_put(1, &edited, 0.0, 1.0);
        assert!(!p.is_pending(&doc(1, &edited, &saved)));
        assert!(!p.is_pending(&doc(1, &saved, &saved)));
    }

    #[test]
    fn draft_names_use_the_xlsx_extension() {
        assert_eq!(draft_name("Book1"), "Book1.xlsx");
        assert_eq!(draft_name("Report.xlsx"), "Report.xlsx");
        assert_eq!(draft_name("data.csv"), "data.xlsx");
        assert_eq!(draft_name("Sales (imported).xlsx"), "Sales (imported).xlsx");
        assert_eq!(draft_name(""), "Book.xlsx");
    }

    #[test]
    fn draft_bytes_reopen_as_the_same_workbook() {
        let mut wb = Workbook::new();
        if let Some(sh) = wb.sheet_mut(0) {
            sh.name = "Data".into();
        }
        let bytes = draft_bytes(&wb).unwrap();
        let (back, _) = gridcraft_engine::io::open_bytes("Book1.xlsx", &bytes).unwrap();
        assert_eq!(back.sheets[0].name, "Data");
    }

    #[test]
    fn a_draft_reopens_as_the_saved_file_does() {
        let mut s = gridcraft_engine::Session::new();
        s.run("file.new", serde_json::json!({"sample": "sales"})).unwrap();
        s.run("cell.set", serde_json::json!({"cell": "H2", "input": "=SUM(B2:B9)"})).unwrap();
        s.run("home.bold", serde_json::json!({"range": "A1:H1"})).unwrap();
        let wb = &s.active().unwrap().wb;
        let saved = gridcraft_engine::io::save_bytes(wb, "Sales.xlsx").unwrap();
        let draft = draft_bytes(wb).unwrap();
        let open = |bytes: &[u8]| gridcraft_engine::io::open_bytes("Sales.xlsx", bytes).unwrap().0;
        assert_eq!(open(&draft), open(&saved));
    }
}
