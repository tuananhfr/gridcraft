//! Narrow windows (phones in portrait): below [`NARROW_WIDTH`] the ribbon folds to its tab row and
//! task panes float over the sheet instead of squeezing it.

/// Window content width, in points, under which the layout compacts. A tablet in portrait
/// (~768 points) stays wide.
pub const NARROW_WIDTH: f32 = 600.0;

pub fn is_narrow(width: f32) -> bool {
    width < NARROW_WIDTH
}

/// Edge-triggered compacting: the ribbon folds once when the width drops below the breakpoint and
/// the user's wide-window choice comes back when it rises above it. In between, the user's own
/// expand or collapse is left alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NarrowLayout {
    /// `None` until the first frame measured the window.
    narrow: Option<bool>,
    /// The ribbon state the user had before the window went narrow; restored when it widens.
    wide_ribbon_collapsed: Option<bool>,
}

impl NarrowLayout {
    /// Feeds this frame's content width; changes `ribbon_collapsed` only on a crossing.
    pub fn update(&mut self, width: f32, ribbon_collapsed: &mut bool) {
        let narrow = is_narrow(width);
        let was = self.narrow.replace(narrow);
        if was == Some(narrow) {
            return;
        }
        if narrow {
            self.wide_ribbon_collapsed = Some(*ribbon_collapsed);
            *ribbon_collapsed = true;
        } else if let Some(wide) = self.wide_ribbon_collapsed.take() {
            *ribbon_collapsed = wide;
        }
    }

    pub fn is_narrow(&self) -> bool {
        self.narrow == Some(true)
    }

    /// The ribbon state worth saving: while narrow, the choice the user made for a wide window,
    /// so a phone-sized session doesn't leave the desktop ribbon folded.
    pub fn saved_ribbon_collapsed(&self, current: bool) -> bool {
        self.wide_ribbon_collapsed.unwrap_or(current)
    }
}

/// Sheet width, in points, a floating pane leaves uncovered so a tap there still reaches the cells.
const SHEET_STRIP: f32 = 48.0;

/// A task pane on the right: a resizable side panel in a wide window; in a narrow one it floats
/// over the sheet instead, so the grid keeps its width under it.
pub fn side_pane(
    narrow: bool,
    ui: &mut egui::Ui,
    id: &'static str,
    default_width: f32,
    max_width: Option<f32>,
    frame: egui::Frame,
    add: impl FnOnce(&mut egui::Ui),
) {
    if !narrow {
        let mut panel = egui::Panel::right(id).default_size(default_width).resizable(true);
        if let Some(max) = max_width {
            panel = panel.max_size(max);
        }
        panel.frame(frame).show(ui, add);
        return;
    }
    let room = ui.available_rect_before_wrap();
    let width = default_width.min((room.width() - SHEET_STRIP).max(room.width() * 0.5)).max(0.0);
    let at = egui::pos2(room.right() - width, room.top());
    let margin = frame.total_margin();
    let inner = egui::vec2((width - margin.sum().x).max(0.0), (room.height() - margin.sum().y).max(0.0));
    let shadow = ui.visuals().popup_shadow;
    egui::Area::new(egui::Id::new((id, "floating"))).order(egui::Order::Middle).fixed_pos(at).constrain(false).show(ui.ctx(), |ui| {
        frame.shadow(shadow).show(ui, |ui| {
            ui.set_width(inner.x);
            ui.set_min_height(inner.y);
            ui.set_max_height(inner.y);
            add(ui);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: f32 = 390.0;
    const TABLET: f32 = 768.0;

    #[test]
    fn going_narrow_folds_the_ribbon_once() {
        let mut layout = NarrowLayout::default();
        let mut collapsed = false;
        layout.update(TABLET, &mut collapsed);
        assert!(!collapsed && !layout.is_narrow(), "a tablet in portrait stays wide");
        layout.update(PHONE, &mut collapsed);
        assert!(collapsed && layout.is_narrow());
    }

    #[test]
    fn a_manual_expand_sticks_while_narrow() {
        let mut layout = NarrowLayout::default();
        let mut collapsed = false;
        layout.update(TABLET, &mut collapsed);
        layout.update(PHONE, &mut collapsed);
        collapsed = false;
        for width in [PHONE, 360.0, 420.0, PHONE] {
            layout.update(width, &mut collapsed);
            assert!(!collapsed, "resizing within the narrow range keeps the user's expand ({width})");
        }
        assert!(!layout.saved_ribbon_collapsed(collapsed));
    }

    #[test]
    fn widening_restores_the_wide_choice() {
        for wide_choice in [false, true] {
            let mut layout = NarrowLayout::default();
            let mut collapsed = wide_choice;
            layout.update(1366.0, &mut collapsed);
            layout.update(PHONE, &mut collapsed);
            assert!(collapsed);
            assert_eq!(layout.saved_ribbon_collapsed(collapsed), wide_choice, "saving while narrow keeps the wide choice");
            // Whatever the user did while narrow, the wide window gets its own choice back.
            collapsed = !wide_choice;
            layout.update(TABLET, &mut collapsed);
            assert_eq!(collapsed, wide_choice);
            assert_eq!(layout.saved_ribbon_collapsed(collapsed), wide_choice);
            // A manual change while wide holds too.
            collapsed = !wide_choice;
            layout.update(1366.0, &mut collapsed);
            assert_eq!(collapsed, !wide_choice);
        }
    }

    #[test]
    fn starting_narrow_folds_and_widening_restores() {
        let mut layout = NarrowLayout::default();
        let mut collapsed = false;
        layout.update(PHONE, &mut collapsed);
        assert!(collapsed);
        layout.update(TABLET, &mut collapsed);
        assert!(!collapsed);
    }

    #[test]
    fn the_breakpoint_itself_is_wide() {
        assert!(!is_narrow(NARROW_WIDTH));
        assert!(is_narrow(NARROW_WIDTH - 0.5));
    }
}
