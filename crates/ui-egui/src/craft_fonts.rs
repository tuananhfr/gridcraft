//! Fonts embedded from craft-fonts at build time (see `build.rs`).

pub struct CraftFont {
    pub family: &'static str,
    pub style: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

/// Hands Inter Regular/Bold to the PDF writer, so printed and exported Vietnamese keeps its
/// letters. Without craft-fonts nothing is registered and PDFs keep the standard fonts only.
pub(crate) fn register_print_fonts() {
    let face = |style: &str| CRAFT_FONTS.iter().find(|f| f.family == "Inter" && f.style == style).map(|f| f.bytes);
    if let (Some(regular), Some(bold)) = (face("Regular"), face("Bold"))
        && let Some(fonts) = gridcraft_pdf::UnicodeFonts::new(regular, bold)
    {
        gridcraft_pdf::register_unicode_fonts(fonts);
    }
}

#[cfg(test)]
mod tests {
    use gridcraft_engine::Session;

    use crate::{Services, SheetApp};

    #[test]
    fn starting_the_app_gives_the_pdf_writer_inter() {
        assert!(!super::CRAFT_FONTS.is_empty(), "built without CRAFT_FONTS_DIR (.cargo/config.toml points at ../craft-fonts)");
        let _app = SheetApp::new(Session::new(), Services::default());
        assert!(gridcraft_pdf::unicode_fonts().is_some());
    }
}
