//! The "keep mine / use theirs" window of `regionConflict`, kept apart from the JavaScript glue
//! so it is drawn and measured in native tests too.

/// Widest the question's text runs, as on a desktop.
const TEXT_W: f32 = 360.0;
/// Room kept between the window and each edge of a narrow screen.
const EDGE: f32 = 16.0;

/// The text width that keeps the window (`chrome`: its margins and border) `EDGE` inside a
/// `screen` wide screen, never narrower than a short word.
fn text_width(screen: f32, chrome: f32) -> f32 {
    (screen - 2.0 * EDGE - chrome).clamp(120.0, TEXT_W)
}

/// Shows the question for `region` and returns the answer once a button is clicked.
pub(crate) fn show(ctx: &egui::Context, region: &str, tr: impl Fn(&str) -> String) -> Option<&'static str> {
    let mut answer = None;
    let style = ctx.global_style();
    let chrome = style.spacing.window_margin.sum().x + 2.0 * style.visuals.window_stroke.width;
    let width = text_width(ctx.content_rect().width(), chrome);
    egui::Window::new(tr("Changed while you were away"))
        .id(egui::Id::new("live_conflict").with(region))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -40.0))
        .show(ctx, |ui| {
            ui.set_max_width(width);
            ui.label(tr("Someone else changed this part while you were offline. Which version do you keep?"));
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button(tr("Keep mine")).clicked() {
                    answer = Some("mine");
                }
                if ui.button(tr("Use theirs")).clicked() {
                    answer = Some("theirs");
                }
            });
        });
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_ui_egui::i18n::Language;

    /// The window's rect after a few frames of a `width` wide screen, in Vietnamese (the longer texts).
    fn window_at(width: f32) -> egui::Rect {
        let ctx = egui::Context::default();
        let tr = |s: &str| Language::Vi.tr(s).to_string();
        for i in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 780.0))),
                time: Some(f64::from(i) * 0.1),
                ..Default::default()
            };
            ctx.run_ui(input, |ui| {
                let _ = show(ui.ctx(), "cell:s1:A1", tr);
            })
            .drop_without_applying_deltas();
        }
        let id = egui::Id::new("live_conflict").with("cell:s1:A1");
        ctx.memory(|m| m.area_rect(id)).expect("the conflict window is shown")
    }

    #[test]
    fn the_text_narrows_only_below_a_desktop_width() {
        assert_eq!(text_width(1280.0, 14.0), TEXT_W);
        assert_eq!(text_width(360.0, 14.0), 360.0 - 32.0 - 14.0);
        assert_eq!(text_width(100.0, 14.0), 120.0);
    }

    #[test]
    fn a_phone_screen_keeps_margins_around_the_conflict_window() {
        // The title doesn't wrap and egui keeps room for buttons on both its sides: a bit wider than the text.
        let r = window_at(360.0);
        assert!(r.left() >= 8.0 && r.right() <= 352.0, "the window {:?} touches the edges of a 360 screen", [r.left(), r.right()]);
        let wide = window_at(1280.0);
        assert!(wide.width() <= 390.0 && wide.width() > r.width(), "a wide screen keeps the 360 text width: {}", wide.width());
    }
}
