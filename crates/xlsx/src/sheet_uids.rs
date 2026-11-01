//! Sheet uids (`Sheet::uid`, for live editing) in a workbook extension of our own, keyed by each
//! sheet's relationship id. Excel ignores an extension it doesn't know (and drops it when it
//! saves); `sheetId` can't hold them, as Office takes only 1 to 65534 there.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::xml::{El, esc_attr};

const URI: &str = "{8A0C7A52-3E44-4B6B-9F1D-5C2E7B9D4A61}";
const NS: &str = "https://office.erpcons.ai/ns/gridcraft/2026/ids";

/// Relationship id → uid, from the workbook part; malformed or repeated entries are skipped.
pub(crate) fn read(workbook: &El) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let ext = workbook.child("extLst").and_then(|l| l.kids("ext").find(|e| e.attr("uri").is_some_and(|u| u.eq_ignore_ascii_case(URI))));
    let Some(list) = ext.and_then(|e| e.child("sheetUids")) else { return out };
    for s in list.kids("sheet").take(65_534) {
        if let (Some(rid), Some(uid)) = (s.attr("rid"), s.attr("uid"))
            && gridcraft_model::is_valid_sheet_uid(uid)
        {
            out.entry(rid.to_string()).or_insert_with(|| uid.to_string());
        }
    }
    out
}

/// The `<ext>` for `(relationship id, uid)` pairs; empty when there are none.
pub(crate) fn ext(uids: &[(String, String)]) -> String {
    if uids.is_empty() {
        return String::new();
    }
    let mut x = format!("<ext uri=\"{URI}\" xmlns:ecid=\"{NS}\"><ecid:sheetUids>");
    for (rid, uid) in uids {
        let _ = write!(x, "<ecid:sheet rid=\"{}\" uid=\"{}\"/>", esc_attr(rid), esc_attr(uid));
    }
    x.push_str("</ecid:sheetUids></ext>");
    x
}
