//! The regional format on the ERPCons office site, remembered in its own localStorage key next to
//! the shared interface language. Nothing stored means "follow the interface language".

use gridcraft_ui_egui::region::{AUTO, Region};

const KEY: &str = "erpcons-grid-region";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// The remembered choice; an unknown value follows the language again.
pub fn initial() -> Option<Region> {
    storage().and_then(|s| s.get_item(KEY).ok().flatten()).and_then(|code| Region::parse(&code))
}

pub fn remember(region: Option<Region>) {
    let Some(s) = storage() else { return };
    if let Err(e) = s.set_item(KEY, region.map_or(AUTO, Region::code)) {
        log::warn!("couldn't remember the regional format: {e:?}");
    }
}
