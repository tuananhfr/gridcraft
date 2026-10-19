//! The regional format preference ([`crate::UiState::region`]): the notation numbers, dates,
//! currency and formulas are typed and edited in. Separate from the interface language, which it
//! follows until the user picks one in File ▸ Options.

use gridcraft_engine::formula::FormulaLocale;
use serde::Deserialize;

use crate::i18n::Language;

pub use gridcraft_engine::core::Region;

/// The code `app.region.set` takes for "follow the interface language".
pub const AUTO: &str = "auto";

/// The regional format an interface language implies while none is chosen.
pub const fn default_for(language: Language) -> Region {
    match language {
        Language::Vi => Region::ViVn,
        Language::Es => Region::EsEs,
        _ => Region::EnUs,
    }
}

pub fn formula_locale(region: Region) -> FormulaLocale {
    FormulaLocale::of(region)
}

/// The English label of a region, translated by [`Language::tr`].
pub const fn label(region: Region) -> &'static str {
    match region {
        Region::EnUs => "English (United States)",
        Region::EsEs => "Spanish (Spain)",
        Region::ViVn => "Vietnamese (Vietnam)",
    }
}

/// Parses `app.region.set`'s value: a region code, or `auto` (`Ok(None)`).
pub fn parse_choice(code: &str) -> Option<Option<Region>> {
    if code.trim().eq_ignore_ascii_case(AUTO) { Some(None) } else { Region::parse(code).map(Some) }
}

/// Saves a chosen region by its code; "follow the language" is saved as nothing.
pub fn serialize<S: serde::Serializer>(region: &Option<Region>, s: S) -> Result<S::Ok, S::Error> {
    match region {
        Some(r) => s.serialize_str(r.code()),
        None => s.serialize_none(),
    }
}

/// Reads a saved region leniently: an unknown or malformed value follows the language again
/// instead of failing the whole `UiState`.
pub fn lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Region>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Code(String),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Raw::deserialize(d)? {
        Raw::Code(code) => Region::parse(&code),
        Raw::Other(_) => None,
    })
}

fn region_key() -> egui::Id {
    egui::Id::new("gridcraft.region")
}

/// The regional format of the frame being drawn, for widgets that only get a `Ui`.
pub fn current(ctx: &egui::Context) -> Region {
    ctx.data(|d| d.get_temp(region_key())).unwrap_or_default()
}

pub fn set_current(ctx: &egui::Context, region: Region) {
    ctx.data_mut(|d| d.insert_temp(region_key(), region));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_follows_the_interface_language() {
        assert_eq!(default_for(Language::Vi), Region::ViVn);
        assert_eq!(default_for(Language::Es), Region::EsEs);
        for l in Language::ALL.into_iter().filter(|l| !matches!(l, Language::Vi | Language::Es)) {
            assert_eq!(default_for(l), Region::EnUs, "{l:?}");
        }
    }

    #[test]
    fn choices_parse_codes_and_auto() {
        assert_eq!(parse_choice("auto"), Some(None));
        assert_eq!(parse_choice("vi-VN"), Some(Some(Region::ViVn)));
        assert_eq!(parse_choice("xx"), None);
    }
}
