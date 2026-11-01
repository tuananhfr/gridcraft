//! Ids this client makes while editing (`erpHost.setClientId`, `erpcons-office` `docs/erp-host.md`):
//! each carries the client's 6-hex-digit prefix, so two people editing the same workbook at once
//! never make the same one. Without a prefix from the host the client picks a random one.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gridcraft_model::Workbook;

#[derive(Debug)]
struct State {
    prefix: String,
    next: u64,
}

/// The id source the session and its documents share.
#[derive(Clone, Debug)]
pub struct ClientIds(Arc<Mutex<State>>);

impl Default for ClientIds {
    fn default() -> Self {
        ClientIds(Arc::new(Mutex::new(State { prefix: random_prefix(), next: 1 })))
    }
}

/// Whether `prefix` is what `setClientId` takes: exactly 6 lowercase hex digits.
pub fn is_client_prefix(prefix: &str) -> bool {
    prefix.len() == 6 && prefix.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl ClientIds {
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn prefix(&self) -> String {
        self.state().prefix.clone()
    }

    /// Uses `prefix` for ids made from now on; `false` (nothing changes) unless it is 6 lowercase
    /// hex digits. The counter carries on, so switching back to an earlier prefix repeats nothing.
    pub fn set_prefix(&self, prefix: &str) -> bool {
        if !is_client_prefix(prefix) {
            return false;
        }
        self.state().prefix = prefix.to_string();
        true
    }

    /// A new id, `<prefix>-<n>` with `n` counting from 1 in decimal; never one in `taken`.
    pub fn next_id(&self, taken: &HashSet<String>) -> String {
        let mut state = self.state();
        loop {
            let id = format!("{}-{}", state.prefix, state.next);
            state.next = state.next.saturating_add(1);
            if !taken.contains(&id) {
                return id;
            }
        }
    }

    /// Gives every sheet of `wb` that has no uid, or shares one with another sheet, a new one.
    /// Of sheets sharing a uid, the one with the name that uid had in `before` keeps it (a copied
    /// sheet gets the new one), else the first.
    pub fn assign_sheet_uids(&self, before: &Workbook, wb: &mut Workbook) {
        let mut seen = HashSet::new();
        if wb.sheets.iter().all(|s| !s.uid.is_empty() && seen.insert(s.uid.as_str())) {
            return;
        }
        let keeper = |uid: &str| -> Option<usize> {
            let named = before.sheet_index_by_uid(uid).and_then(|i| before.sheets.get(i)).map(|s| s.name.as_str());
            let mut same = wb.sheets.iter().enumerate().filter(|(_, s)| s.uid == uid);
            let first = same.clone().next().map(|(i, _)| i);
            same.find(|(_, s)| Some(s.name.as_str()) == named).map(|(i, _)| i).or(first)
        };
        let mut fresh = Vec::new();
        for (i, s) in wb.sheets.iter().enumerate() {
            if s.uid.is_empty() || keeper(&s.uid) != Some(i) {
                fresh.push(i);
            }
        }
        let mut taken: HashSet<String> = wb.sheets.iter().map(|s| s.uid.clone()).collect();
        for i in fresh {
            let uid = self.next_id(&taken);
            taken.insert(uid.clone());
            if let Some(s) = wb.sheet_mut(i) {
                s.uid = uid;
            }
        }
    }
}

/// A random prefix from the OS's hash seed and the clock. The web app has neither (and no clock
/// at all: reading it traps), so it sets one from the browser's random source at start-up.
fn random_prefix() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    let bits = {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        #[allow(clippy::disallowed_methods)]
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
        h.write_u128(now);
        h.write_u32(std::process::id());
        h.finish()
    };
    #[cfg(target_arch = "wasm32")]
    let bits = {
        let probe = Box::new(0u8);
        std::ptr::addr_of!(*probe) as u64
    };
    format!("{:06x}", bits & 0xff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_model::Sheet;

    #[test]
    fn prefixes_are_six_lowercase_hex_digits() {
        assert!(is_client_prefix("0a9f3c"));
        for bad in ["", "0a9f3", "0a9f3c1", "0A9F3C", "0a9g3c", " a9f3c"] {
            assert!(!is_client_prefix(bad), "{bad:?}");
        }
        let ids = ClientIds::default();
        assert!(is_client_prefix(&ids.prefix()));
        assert!(!ids.set_prefix("XYZ"));
        assert!(ids.set_prefix("00beef"));
        assert_eq!(ids.next_id(&HashSet::new()), "00beef-1");
        assert!(ids.set_prefix("c0ffee"));
        assert_eq!(ids.next_id(&HashSet::from(["c0ffee-2".to_string()])), "c0ffee-3");
    }

    #[test]
    fn two_clients_never_make_the_same_id() {
        let (a, b) = (ClientIds::default(), ClientIds::default());
        assert!(a.set_prefix("aaaaaa") && b.set_prefix("aaaaab"));
        let taken = HashSet::new();
        let mut all = HashSet::new();
        for _ in 0..1000 {
            assert!(all.insert(a.next_id(&taken)));
            assert!(all.insert(b.next_id(&taken)));
        }
    }

    #[test]
    fn a_copied_sheet_gets_a_new_uid_and_the_original_keeps_its_own() {
        let ids = ClientIds::default();
        assert!(ids.set_prefix("123abc"));
        let mut before = Workbook::new();
        before.fill_missing_sheet_uids();
        let mut wb = before.clone();
        let mut copy = (*wb.sheets[0]).clone();
        copy.name = "Sheet1 (2)".into();
        wb.sheets.insert(0, std::sync::Arc::new(copy));
        wb.sheets.push(std::sync::Arc::new(Sheet::new("New")));
        ids.assign_sheet_uids(&before, &mut wb);
        let uids: Vec<_> = wb.sheets.iter().map(|s| (s.name.as_str(), s.uid.as_str())).collect();
        assert_eq!(uids, [("Sheet1 (2)", "123abc-1"), ("Sheet1", "f1"), ("New", "123abc-2")]);
    }
}
