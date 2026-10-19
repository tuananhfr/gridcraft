//! Several open documents: the open limit, closing a window by index and hidden windows.
use gridcraft_engine::{MAX_DOCUMENTS, Session, TOO_MANY_DOCUMENTS, UiRequest};
use serde_json::{Value as Json, json};

fn uids(s: &Session) -> Vec<u64> {
    s.documents().iter().map(|d| d.uid).collect()
}

fn active_uid(s: &Session) -> u64 {
    s.doc().unwrap().uid
}

fn session_with(n: usize) -> Session {
    let mut s = Session::new();
    for _ in 0..n {
        s.execute("file.new", json!({})).unwrap();
    }
    s
}

fn refused(r: gridcraft_engine::Result<Json>) {
    let e = r.expect_err("an 11th document must be refused").to_string();
    assert_eq!(e, TOO_MANY_DOCUMENTS);
}

#[test]
fn an_eleventh_document_is_refused_on_every_open_path() {
    assert!(TOO_MANY_DOCUMENTS.contains(&MAX_DOCUMENTS.to_string()));
    let xlsx = {
        let mut s = session_with(1);
        s.execute("file.saveBytes", json!({})).unwrap()["base64"].as_str().unwrap().to_string()
    };
    let mut s = session_with(MAX_DOCUMENTS);
    assert_eq!(s.documents().len(), MAX_DOCUMENTS);
    s.execute("window.activate", json!({"index": 3})).unwrap();
    let before = uids(&s);
    refused(s.execute("file.new", json!({})));
    refused(s.execute("file.new", json!({"sample": "sales"})));
    refused(s.execute("file.open", json!({"name": "Other.xlsx", "base64": xlsx})));
    refused(s.execute("view.newWindow", json!({})));
    assert_eq!(s.new_workbook(), None);
    assert_eq!(uids(&s), before, "nothing was opened or replaced");
    assert_eq!(s.active_index(), 3, "the active document stays");
    s.execute("file.close", json!({"index": 0})).unwrap();
    s.execute("file.open", json!({"name": "Other.xlsx", "base64": xlsx})).expect("room again after closing one");
    assert_eq!(s.documents().len(), MAX_DOCUMENTS);
}

#[test]
fn closing_another_window_keeps_the_active_one() {
    let mut s = session_with(4);
    s.execute("window.activate", json!({"index": 2})).unwrap();
    let active = active_uid(&s);
    s.execute("file.close", json!({"index": 0})).unwrap();
    assert_eq!(active_uid(&s), active, "closing a window before the active one");
    s.execute("file.close", json!({"index": 2})).unwrap();
    assert_eq!(active_uid(&s), active, "closing a window after the active one");
    assert_eq!(s.documents().len(), 2);
    assert!(s.execute("file.close", json!({"index": 7})).is_err(), "no such window");
}

#[test]
fn closing_the_active_window_activates_its_neighbour() {
    let mut s = session_with(3);
    let ids = uids(&s);
    s.execute("window.activate", json!({"index": 1})).unwrap();
    s.execute("file.close", json!({})).unwrap();
    assert_eq!(active_uid(&s), ids[2], "the next one to the right");
    s.execute("file.close", json!({})).unwrap();
    assert_eq!(active_uid(&s), ids[0], "the last one closed: the one to its left");
}

#[test]
fn closing_a_dirty_window_by_index_asks_first() {
    let mut s = session_with(2);
    s.execute("window.activate", json!({"index": 0})).unwrap();
    s.execute("cell.set", json!({"cell": "A1", "input": "work"})).unwrap();
    let title = s.doc().unwrap().display_title();
    s.execute("window.activate", json!({"index": 1})).unwrap();
    s.take_ui_requests();
    let r = s.execute("file.close", json!({"index": 0})).unwrap();
    assert_eq!(r["closed"], false);
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.take_ui_requests(), vec![UiRequest::Dialog("saveChanges".into(), json!({"title": title, "index": 0}))]);
    s.execute("file.close", json!({"index": 0, "force": true})).unwrap();
    assert_eq!(s.documents().len(), 1);
}

#[test]
fn a_hidden_window_stays_hidden_when_a_window_before_it_closes() {
    let mut s = session_with(3);
    let ids = uids(&s);
    s.execute("window.activate", json!({"index": 2})).unwrap();
    s.execute("view.hideWindow", json!({})).unwrap();
    assert_eq!(active_uid(&s), ids[0], "hiding activates a shown window");
    s.execute("file.close", json!({"index": 0})).unwrap();
    assert_eq!(active_uid(&s), ids[1], "the next shown window, not the hidden one");
    let windows = s.execute("view.switchWindows", json!({})).unwrap();
    let hidden: Vec<bool> = windows.as_array().unwrap().iter().map(|w| w["hidden"].as_bool().unwrap()).collect();
    assert_eq!(hidden, vec![false, true], "the hidden window moved to index 1 and is still the hidden one");
    s.execute("view.unhideWindow", json!({})).unwrap();
    assert_eq!(active_uid(&s), ids[2]);
    assert!(!s.is_hidden(1));
}

#[test]
fn closing_a_hidden_window_forgets_it() {
    let mut s = session_with(2);
    s.execute("window.activate", json!({"index": 1})).unwrap();
    s.execute("view.hideWindow", json!({})).unwrap();
    s.execute("file.close", json!({"index": 1})).unwrap();
    assert!(s.hidden_windows.is_empty());
    s.execute("file.new", json!({})).unwrap();
    assert!(!s.is_hidden(1), "a new window at the closed one's index is shown");
}
