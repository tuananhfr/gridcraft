//! `window.erpHost` v1 (`erpcons-office` `docs/erp-host.md`): workbooks the ERPCons office shell
//! keeps on its server. The shell opens them with `open(doc)`; Ctrl+S / "Lưu", AutoSave and
//! `flush` save them back with `erpHost.save`, and a document counts as saved only as of the
//! revision the shell acknowledged. They never go to `erpDrafts`. Without `erpHost` v1 nothing
//! here runs and the page behaves as before.

mod autosave;
#[cfg(target_arch = "wasm32")]
mod bridge;
mod docs;
mod protocol;
#[cfg(test)]
mod tests;

#[cfg(target_arch = "wasm32")]
pub(crate) use bridge::{ErpHost, detect};
