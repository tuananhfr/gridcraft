//! Which title-bar controls a narrow window keeps: the optional ones give way, lowest priority
//! first, until the rest fits without overlapping. The document title takes whatever is left.

/// Title-bar controls that may be hidden, in the order they give way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Optional {
    AutoSaveLabel,
    Theme,
    Comments,
    /// The PI ID account button (File › Account stays reachable when it gives way).
    Account,
    Home,
    UndoList,
    ShareLabel,
    Redo,
}

pub const DROP_ORDER: [Optional; 8] = [
    Optional::AutoSaveLabel,
    Optional::Theme,
    Optional::Comments,
    Optional::Account,
    Optional::Home,
    Optional::UndoList,
    Optional::ShareLabel,
    Optional::Redo,
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TitleFit {
    /// How many of [`DROP_ORDER`] are hidden.
    hidden: usize,
}

impl TitleFit {
    /// Everything shown (a wide window).
    pub const ALL: TitleFit = TitleFit { hidden: 0 };

    /// Hides optional controls until `fixed` (the controls that always show, with their spacing)
    /// plus the remaining optional widths fit in `available`.
    pub fn new(available: f32, fixed: f32, width: impl Fn(Optional) -> f32) -> TitleFit {
        let mut needed = fixed + DROP_ORDER.iter().map(|&o| width(o)).sum::<f32>();
        let mut hidden = 0;
        while hidden < DROP_ORDER.len() && matches!(needed.partial_cmp(&available), None | Some(std::cmp::Ordering::Greater)) {
            needed -= width(DROP_ORDER[hidden]);
            hidden += 1;
        }
        TitleFit { hidden }
    }

    pub fn shows(self, item: Optional) -> bool {
        DROP_ORDER.iter().position(|&o| o == item).is_none_or(|at| at >= self.hidden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(o: Optional) -> f32 {
        match o {
            Optional::AutoSaveLabel => 80.0,
            Optional::ShareLabel => 50.0,
            Optional::UndoList => 20.0,
            _ => 30.0,
        }
    }

    #[test]
    fn a_wide_bar_keeps_everything() {
        let fit = TitleFit::new(1366.0, 250.0, widths);
        assert!(DROP_ORDER.iter().all(|&o| fit.shows(o)));
    }

    #[test]
    fn controls_give_way_lowest_priority_first() {
        // 250 fixed + 300 optional: 130 short of 550 hides the label, the theme switch and Comments.
        let fit = TitleFit::new(420.0, 250.0, widths);
        assert!(!fit.shows(Optional::AutoSaveLabel) && !fit.shows(Optional::Theme) && !fit.shows(Optional::Comments));
        assert!(fit.shows(Optional::Account) && fit.shows(Optional::Redo));
        let mut last = 0;
        for available in (200..=600).rev().step_by(10) {
            let fit = TitleFit::new(available as f32, 250.0, widths);
            let shown: Vec<bool> = DROP_ORDER.iter().map(|&o| fit.shows(o)).collect();
            let hidden = shown.iter().take_while(|s| !**s).count();
            assert!(shown[hidden..].iter().all(|s| *s), "hidden ones are a prefix of the order at {available}");
            assert!(hidden >= last, "narrower never shows more");
            last = hidden;
        }
    }

    #[test]
    fn the_account_button_outlasts_theme_and_comments() {
        let at = |item| DROP_ORDER.iter().position(|&o| o == item);
        assert!(at(Optional::Account) > at(Optional::Theme) && at(Optional::Account) > at(Optional::Comments));
    }

    #[test]
    fn hostile_widths_hide_all_optional_controls() {
        let fit = TitleFit::new(f32::NAN, 250.0, widths);
        assert!(DROP_ORDER.iter().all(|&o| !fit.shows(o)));
    }
}
