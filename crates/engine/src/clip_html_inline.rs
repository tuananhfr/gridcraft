//! Whole-cell formatting from the common ancestors of visible clipboard text.

use gridcraft_model::Style;

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct WholeCellStyle {
    depth: Option<usize>,
    candidate: Option<(Style, bool)>,
    inline_css: String,
    number_format_hint: bool,
}

impl WholeCellStyle {
    pub fn new(inline_css: String, number_format_hint: bool) -> Self {
        Self { inline_css, number_format_hint, ..Self::default() }
    }

    pub fn depth(&self, current: usize) -> usize {
        self.depth.unwrap_or(current)
    }

    pub fn observe(&mut self, depth: usize, style: &Style, preserve: bool) {
        self.depth = Some(depth);
        self.candidate = Some((style.clone(), preserve));
    }

    pub fn truncate(&mut self, depth: usize) {
        // A closed wrapper remains eligible until more non-whitespace text proves it was partial.
        if let Some(common) = self.depth.as_mut() {
            *common = (*common).min(depth);
        }
    }

    pub fn finish(&mut self, style: &mut Style, preserve: &mut bool) {
        let state = std::mem::take(self);
        if let Some((candidate, whitespace)) = state.candidate {
            *style = candidate;
            *preserve = whitespace;
        }
        // Cell declarations take precedence even when a descendant overrides an inherited value.
        let mut hidden = false;
        let number_format = style.num_fmt.clone();
        crate::clip_html_style::apply(&state.inline_css, style, &mut hidden, preserve);
        // Reapplying CSS must not replace the existing LibreOffice numeric-format hint.
        if state.number_format_hint {
            style.num_fmt = number_format;
        }
    }
}
