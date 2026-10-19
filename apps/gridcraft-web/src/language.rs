//! The interface language on the ERPCons office site: Vietnamese until the visitor picks another
//! one, remembered in one localStorage key every editor of the site reads, so a choice made in
//! one editor carries over to the others.

use gridcraft_ui_egui::i18n::Language;

const KEY: &str = "erpcons-ui-language";
const DEFAULT: Language = Language::Vi;

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// The remembered language, if GridCraft has it and can draw it. The web build bundles no CJK
/// font, so a Chinese, Japanese or Korean choice made in another editor falls back to the default.
pub fn initial() -> Language {
    let stored = storage().and_then(|s| s.get_item(KEY).ok().flatten());
    let wanted = match stored.as_deref() {
        // The other editors save "auto" for "follow the browser".
        Some("auto") => Some(Language::system()),
        Some(code) => Language::parse(code),
        None => None,
    };
    wanted.filter(|l| !matches!(l, Language::Zh | Language::Ja | Language::Ko)).unwrap_or(DEFAULT)
}

pub fn remember(language: Language) {
    if let Some(s) = storage()
        && let Err(e) = s.set_item(KEY, language.code())
    {
        log::warn!("couldn't remember the interface language: {e:?}");
    }
}
