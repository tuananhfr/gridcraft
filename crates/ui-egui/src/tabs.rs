//! Sheet tabs and the status bar.

use std::sync::Arc;

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use gridcraft_engine::model::{Sheet, Visibility};
use serde_json::json;

use crate::SheetApp;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// The rename field asks for focus only while it has not got it yet. Asking again on the frame it lost focus (Enter or a click elsewhere)
/// would keep the field focused after it is closed, so the keyboard stayed stuck in the sheet name.
fn rename_needs_focus(has_focus: bool, lost_focus: bool) -> bool {
    !has_focus && !lost_focus
}

/// Rows of the All Sheets navigator as `(workbook index, name)`. Hidden sheets stay out, like on the tab strip:
/// Unhide is the way to reach them.
fn navigator_items(sheets: &[Arc<Sheet>]) -> Vec<(String, String)> {
    sheets.iter().enumerate().filter(|(_, s)| s.visibility == Visibility::Visible).map(|(i, s)| (i.to_string(), s.name.clone())).collect()
}

/// Switch to sheet `i` from the tab strip or the navigator. In point mode the formula being typed keeps going on the
/// other sheet; otherwise the edit is committed first.
fn activate_sheet(app: &mut SheetApp, i: usize) {
    if app.editor.as_ref().is_some_and(|e| e.can_point()) {
        let _ = app.session.run("sheet.activate", json!({"sheet": i}));
    } else {
        if app.editor.is_some() {
            app.commit_edit(0, 0, false, false);
        }
        app.run_or_alert("sheet.activate", json!({"sheet": i}));
    }
}

pub fn sheet_tabs(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("sheet_tabs").exact_size(32.0).frame(egui::Frame::NONE.fill(t.tab_bar)).show(ui, |ui| {
        let rect = ui.max_rect();
        ui.painter().line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, t.ribbon_border));
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(8.0);
            if crate::widgets::icon_button(ui, Icon::Left, t.text_dim, "Previous sheet", vec2(24.0, 24.0)).clicked() {
                app.run_or_alert("sheet.previous", json!({}));
            }
            if crate::widgets::icon_button(ui, Icon::Right, t.text_dim, "Next sheet", vec2(24.0, 24.0)).clicked() {
                app.run_or_alert("sheet.next", json!({}));
            }
            let all = crate::widgets::icon_button(ui, Icon::SheetList, t.text_dim, "All sheets", vec2(24.0, 24.0));
            all.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "All sheets"));
            let items = app.session.active().map(|d| navigator_items(&d.wb.sheets)).unwrap_or_default();
            let active = app.session.active().map(|d| d.wb.active_sheet.to_string()).unwrap_or_default();
            let rows: Vec<(&str, String)> = items.iter().map(|(i, name)| (i.as_str(), name.clone())).collect();
            if let crate::ribbon::ComboResult::Picked(i) = crate::ribbon::editable_combo(ui, &all, "sheet_navigator", &active, &rows, false)
                && let Ok(i) = i.parse::<usize>()
            {
                activate_sheet(app, i);
            }
            ui.add_space(8.0);
            let Some(d) = app.session.active() else { return };
            let sheets: Vec<(usize, String, Option<[u8; 3]>, bool)> =
                d.wb.sheets
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.visibility == Visibility::Visible)
                    .map(|(i, s)| (i, s.name.clone(), s.tab_color.and_then(|c| c.resolve(&d.wb.theme)), i == d.wb.active_sheet))
                    .collect();
            // Keep worksheet tabs in their own horizontally scrollable viewport. As in Excel, the
            // tabs get 60% of the bar and the grid's horizontal scrollbar the rest.
            let tabs_width = (ui.available_width() * 0.6).max(100.0);
            // Reveal the selected sheet after navigation or layout changes, but
            // allow browsing other tabs without snapping back on every frame.
            let reveal_key = egui::Id::new((d.uid, d.wb.active_sheet, &sheets, tabs_width.to_bits()));
            let reveal_id = ui.id().with("worksheet_tabs_reveal");
            let reveal_active = ui.ctx().data_mut(|data| {
                let changed = data.get_temp::<egui::Id>(reveal_id) != Some(reveal_key);
                data.insert_temp(reveal_id, reveal_key);
                changed
            });
            ui.allocate_ui_with_layout(vec2(tabs_width, 30.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                // Translate vertical wheel movement into horizontal
                // worksheet-tab scrolling.
                let wheel_scroll = if ui.rect_contains_pointer(ui.max_rect()) {
                    ui.input(|i| {
                        let d = i.smooth_scroll_delta;
                        if d.x.abs() > d.y.abs() { d.x } else { d.y }
                    })
                } else {
                    0.0
                };

                egui::ScrollArea::horizontal()
                    .id_salt("worksheet_tabs_horizontal")
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        if wheel_scroll.abs() > 0.0 {
                            ui.scroll_with_delta(vec2(wheel_scroll, 0.0));
                        }
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            for (i, name, color, active) in sheets {
                                let renaming = app.grid.renaming_tab == Some(i);
                                let font = if active { theme::ui_bold(13.0) } else { theme::ui_font(13.0) };
                                let tw = ui.painter().layout_no_wrap(name.clone(), font.clone(), t.text).size().x;
                                let w = (tw + 32.0).max(72.0);
                                let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click_and_drag());
                                if active && reveal_active {
                                    resp.scroll_to_me(None);
                                }
                                let r = r.shrink2(vec2(0.0, 1.0));
                                if active {
                                    ui.painter().rect_filled(Rect::from_min_max(pos2(r.left(), r.top() - 2.0), r.max), 4.0, t.tab_active);
                                    ui.painter().rect_stroke(
                                        Rect::from_min_max(pos2(r.left(), r.top() - 2.0), r.max),
                                        4.0,
                                        Stroke::new(1.0, t.ribbon_border),
                                        StrokeKind::Inside,
                                    );
                                } else if resp.hovered() {
                                    ui.painter().rect_filled(r, 4.0, t.hover);
                                }
                                if let Some(c) = color {
                                    let band = Rect::from_min_size(pos2(r.left() + 6.0, r.bottom() - 4.0), vec2(r.width() - 12.0, 3.0));
                                    ui.painter().rect_filled(band, 1.0, theme::color32(c));
                                } else if active {
                                    let band = Rect::from_center_size(pos2(r.center().x, r.bottom() - 3.0), vec2(tw.min(48.0), 2.5));
                                    ui.painter().rect_filled(band, 1.0, t.accent);
                                }
                                if renaming {
                                    let mut text = app.grid.rename_text.clone();
                                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(6.0, 5.0))));
                                    let ed =
                                        child.add(egui::TextEdit::singleline(&mut text).font(theme::ui_font(13.0)).desired_width(r.width() - 12.0));
                                    app.grid.rename_text = text.clone();
                                    if rename_needs_focus(ed.has_focus(), ed.lost_focus()) {
                                        ed.request_focus();
                                    }
                                    if ed.lost_focus() {
                                        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) && text != name {
                                            app.run_or_alert("sheet.rename", json!({"sheet": i, "name": text}));
                                        }
                                        app.grid.renaming_tab = None;
                                    }
                                } else {
                                    ui.painter().text(r.center(), Align2::CENTER_CENTER, &name, font, if active { t.accent } else { t.text });
                                }
                                if resp.clicked() {
                                    activate_sheet(app, i);
                                }
                                if resp.double_clicked() {
                                    app.grid.renaming_tab = Some(i);
                                    app.grid.rename_text = name.clone();
                                }
                                // Drag to reorder.
                                if resp.drag_stopped()
                                    && let Some(p) = ui.input(|inp| inp.pointer.interact_pos())
                                {
                                    let dx = p.x - r.center().x;
                                    let steps = (dx / w).round() as i64;
                                    if steps != 0 {
                                        let to = (i as i64 + steps).max(0) as usize;
                                        app.run_or_alert("sheet.move", json!({"sheet": i, "to": to}));
                                    }
                                }
                                let lang = app.ui.language;
                                resp.context_menu(|ui| {
                                    ui.set_min_width(180.0);
                                    if ui.button(lang.tr("Insert Sheet")).clicked() {
                                        app.run_or_alert("home.insertSheet", json!({"before": i}));
                                    }
                                    if ui.button(lang.tr("Delete")).clicked() {
                                        app.run_or_alert("home.deleteSheet", json!({"sheet": i}));
                                    }
                                    if ui.button(lang.tr("Rename")).clicked() {
                                        app.grid.renaming_tab = Some(i);
                                        app.grid.rename_text = name.clone();
                                    }
                                    if ui.button(lang.tr("Move or Copy…")).clicked() {
                                        app.open_dialog("moveSheet", json!({"sheet": i}));
                                    }
                                    if ui.button(lang.tr("Duplicate")).clicked() {
                                        app.run_or_alert("sheet.move", json!({"sheet": i, "to": i + 1, "copy": true}));
                                    }
                                    ui.menu_button(lang.tr("Tab Color"), |ui| {
                                        let colors = app.session.active().map(|d| d.wb.theme.colors).unwrap_or_default();
                                        if let Some(c) = crate::widgets::color_palette(ui, &colors, lang.tr("No Color")) {
                                            app.run_or_alert("sheet.tabColor", json!({"sheet": i, "color": c}));
                                            ui.close();
                                        }
                                    });
                                    ui.separator();
                                    if ui.button(lang.tr("Hide")).clicked() {
                                        app.run_or_alert("sheet.hide", json!({"sheet": i}));
                                    }
                                    if ui.button(lang.tr("Unhide…")).clicked() {
                                        app.run_or_alert("sheet.unhide", json!({}));
                                    }
                                    ui.separator();
                                    let protected = app.session.active().and_then(|d| d.wb.active().map(|s| s.is_protected())).unwrap_or(false);
                                    if ui.button(lang.tr(if protected { "Unprotect Sheet…" } else { "Protect Sheet…" })).clicked() {
                                        app.open_dialog(if protected { "unprotectSheet" } else { "protectSheet" }, json!({}));
                                    }
                                    if ui.button(lang.tr("Select All Sheets")).clicked() {
                                        ui.close();
                                    }
                                });
                            }
                        });
                    });
            });
            ui.add_space(6.0);
            if crate::widgets::icon_button(ui, Icon::Plus, t.text_dim, "New sheet (⇧F11)", vec2(26.0, 26.0)).clicked() {
                app.run_or_alert("home.insertSheet", json!({}));
            }
            // The track ends where the grid does, so it stops under the vertical scrollbar.
            let avail = ui.available_rect_before_wrap();
            let right = app.grid.rect.map_or(avail.right(), |r| r.right()).min(avail.right());
            let track = Rect::from_min_max(pos2(avail.left() + 8.0, avail.center().y - 7.0), pos2(right, avail.center().y + 7.0));
            crate::grid::horizontal_scrollbar(app, ui, track, &t);
        });
    });
}

/// Temp data id (in the context) under which the status bar leaves its last layout for tests:
/// `(left notes, right controls and totals)`.
pub const STATUS_BAR_LAYOUT: &str = "gridcraft.status_bar_layout";

pub fn status_bar(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("status_bar").exact_size(26.0).frame(egui::Frame::NONE.fill(t.status_bar).inner_margin(egui::Margin::symmetric(12, 0))).show(
        ui,
        |ui| {
            // A narrow window keeps only what fits: the right-hand controls' width is measured each
            // frame, and the notes on the left and the selection totals give way to them.
            let narrow = app.narrow.is_narrow();
            let fixed_id = ui.id().with("status_right_fixed");
            let right_fixed: f32 = ui.data(|d| d.get_temp(fixed_id)).unwrap_or(0.0);
            let fits =
                |ui: &Ui, text: &str| !narrow || text_width(ui, text) + 12.0 + ui.spacing().item_spacing.x <= ui.available_width() - right_fixed;
            ui.horizontal_centered(|ui| {
                if narrow {
                    // Keep room for Sum beside the zoom controls at phone widths.
                    ui.spacing_mut().item_spacing.x = 4.0;
                }
                let mode = match app.session.mode {
                    gridcraft_engine::Mode::Ready => "Ready",
                    gridcraft_engine::Mode::Enter => "Enter",
                    gridcraft_engine::Mode::Edit => "Edit",
                    gridcraft_engine::Mode::Point => "Point",
                };
                ui.label(egui::RichText::new(app.ui.language.tr(mode)).font(theme::ui_font(12.5)).color(t.text_dim));
                crate::save_status::status_label(app, ui, &t);
                if let Some(d) = app.session.active() {
                    let lang = app.ui.language;
                    let macro_status = if cfg!(target_arch = "wasm32") { "Macros preserved (not executed)" } else { "Macros" };
                    if d.wb.vba_project.is_some() && fits(ui, lang.tr(macro_status)) {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(lang.tr(macro_status)).font(theme::ui_font(12.5)).color(t.danger));
                    }
                    if !d.calc.stale.is_empty() && fits(ui, lang.tr("Recalculation needed")) {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(lang.tr("Recalculation needed")).font(theme::ui_font(12.5)).color(t.text_dim));
                    }
                    if (!d.calc.not_calculated.is_empty() || workbook_has_errors(ui, d)) && fits(ui, lang.tr("Calculation has errors")) {
                        ui.add_space(12.0);
                        let response = ui.label(egui::RichText::new(lang.tr("Calculation has errors")).font(theme::ui_font(12.5)).color(t.danger));
                        if let Some(cell) = d.wb.active().and_then(|sh| sh.cell(d.selection.active))
                            && cell.value.as_error() == Some(gridcraft_engine::core::CellError::Name)
                            && let Some(error) = gridcraft_engine::macros::udf_diagnostic(&d.wb, d.wb.active_sheet, cell)
                        {
                            response.on_hover_text(crate::macro_dialog::diagnostic_text(lang, &error));
                        }
                    }
                    if !d.calc.circular.is_empty() {
                        let (si, c) = d.calc.circular[0];
                        let name = d.wb.sheet(si).map(|s| s.name.clone()).unwrap_or_default();
                        let text = format!("{}: {name}!{}", lang.tr("Circular References"), c.a1());
                        if fits(ui, &text) {
                            ui.add_space(12.0);
                            ui.label(egui::RichText::new(text).font(theme::ui_font(12.5)).color(t.danger));
                        }
                    }
                    if app.session.clipboard.is_some()
                        && app.editor.is_none()
                        && fits(ui, lang.tr("Select destination and press Enter or choose Paste"))
                    {
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new(lang.tr("Select destination and press Enter or choose Paste"))
                                .font(theme::ui_font(12.5))
                                .color(t.text_dim),
                        );
                    }
                }
                let left_end = ui.min_rect().right();
                let zoom = app.session.active().and_then(|d| d.wb.active().map(|s| s.zoom)).unwrap_or(100);
                let parts = selection_totals(app);
                // A narrow window keeps what fits, by priority: the zoom buttons always, then Sum,
                // the view buttons, Count, Average and last the zoom slider.
                let gap = ui.spacing().item_spacing.x;
                let part_w = |p: &str| text_width(ui, p) + 10.0 + 2.0 * gap;
                let base = text_width(ui, &format!("{zoom}%")) + 4.0 + 2.0 * (18.0 + gap) + 16.0 + 2.0 * gap;
                let mut wants = vec![10.0 + 3.0 * (26.0 + gap), ui.spacing().slider_width + gap];
                wants.extend(parts.iter().map(|p| part_w(p)));
                // Indices into `wants`: 0 the views, 1 the slider, 2.. the totals.
                let mut order = Vec::new();
                if !parts.is_empty() {
                    order.push(2);
                }
                order.push(0);
                order.extend(3..2 + parts.len());
                order.push(1);
                let shown = if narrow { fit_in(ui.available_width(), base, &wants, &order) } else { vec![true; wants.len()] };
                let show_views = shown[0];
                let show_slider = shown[1];
                let right = ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Zoom percentage and slider.
                    let zl = ui.add(
                        egui::Label::new(egui::RichText::new(format!("{zoom}%")).font(theme::ui_font(12.5)).color(t.text_dim)).sense(Sense::click()),
                    );
                    if zl.clicked() {
                        app.open_dialog("zoom", json!({}));
                    }
                    ui.add_space(4.0);
                    if crate::widgets::icon_button(ui, Icon::Plus, t.text_dim, "Zoom In", vec2(18.0, 18.0)).clicked() {
                        let z = ((zoom as f32 / 10.0).floor() * 10.0 + 10.0).min(400.0);
                        app.run_or_alert("view.zoom", json!({"percent": z}));
                    }
                    if show_slider {
                        let mut z = zoom as f32;
                        let slider = ui.add(egui::Slider::new(&mut z, 10.0..=400.0).show_value(false).logarithmic(true));
                        if slider.changed() {
                            let _ = app.session.run("view.zoom", json!({"percent": z.round()}));
                        }
                    }
                    if crate::widgets::icon_button(ui, Icon::Minus, t.text_dim, "Zoom Out", vec2(18.0, 18.0)).clicked() {
                        let z = ((zoom as f32 / 10.0).ceil() * 10.0 - 10.0).max(10.0);
                        app.run_or_alert("view.zoom", json!({"percent": z}));
                    }
                    if show_views {
                        ui.add_space(10.0);
                        for (icon, tip, mode, cmd) in [
                            (Icon::PageBreak, "Page Break Preview", "pageBreakPreview", "view.pageBreakPreview"),
                            (Icon::PageLayout, "Page Layout", "pageLayout", "view.pageLayout"),
                            (Icon::Normal, "Normal", "normal", "view.normal"),
                        ] {
                            let (r, resp) = ui.allocate_exact_size(vec2(26.0, 20.0), Sense::click());
                            let cur = app.session.active().and_then(|d| d.wb.active().map(|s| s.view.as_str())).unwrap_or("normal");
                            if cur == mode {
                                ui.painter().rect_filled(r, 3.0, t.pressed);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(r, 3.0, t.hover);
                            }
                            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(14.0, 14.0)), icon, t.text_dim);
                            if resp.on_hover_text(app.ui.language.tr(tip)).clicked() {
                                app.run_or_alert(cmd, json!({}));
                            }
                        }
                    }
                    ui.add_space(16.0);
                    ui.data_mut(|d| d.insert_temp(fixed_id, ui.min_rect().width()));
                    // Sum, Count and Average, right to left.
                    for (p, show) in parts.into_iter().zip(shown.into_iter().skip(2)) {
                        if show {
                            ui.label(egui::RichText::new(p).font(theme::ui_font(12.5)).color(t.text_dim));
                            ui.add_space(10.0);
                        }
                    }
                });
                let left = Rect::from_x_y_ranges(ui.max_rect().left()..=left_end, ui.max_rect().y_range());
                ui.data_mut(|d| d.insert_temp(egui::Id::new(STATUS_BAR_LAYOUT), (left, right.response.rect)));
            });
        },
    );
}

/// The selection's Sum, Count and Average as the status bar shows them (none for one cell).
fn selection_totals(app: &SheetApp) -> Vec<String> {
    let Some(d) = app.session.active() else { return Vec::new() };
    let Some(sh) = d.wb.active() else { return Vec::new() };
    if d.selection.is_single_cell() {
        return Vec::new();
    }
    let st = gridcraft_engine::display::stats(sh, &d.selection.ranges);
    let region = app.ui.region();
    let fmt = |v: f64| {
        let code = d.wb.styles.get(sh.style_id(d.selection.active)).num_fmt.as_str().to_string();
        let code = if code == "General" { "#,##0.##########".to_string() } else { code };
        gridcraft_engine::display::format_in(&gridcraft_engine::core::Value::Number(v), &code, &d.wb, region)
            .text
            // A whole number leaves the decimal point of `#,##0.###…`: `.` in en-US, `,` in vi-VN.
            .trim_end_matches(['.', ','])
            .to_string()
    };
    let mut parts = Vec::new();
    if st.numeric > 0 {
        parts.push(format!("{}: {}", app.ui.language.tr("Sum"), fmt(st.sum)));
    }
    if st.count > 0 {
        parts.push(format!("{}: {}", app.ui.language.tr("Count"), st.count));
    }
    if let Some(a) = st.average {
        parts.push(format!("{}: {}", app.ui.language.tr("Average"), fmt(a)));
    }
    parts
}

/// Which of `widths` to show in `available` after `base`, taking them greedily in `order`
/// (indices into `widths`, most important first); one that doesn't fit is skipped.
fn fit_in(available: f32, base: f32, widths: &[f32], order: &[usize]) -> Vec<bool> {
    let mut shown = vec![false; widths.len()];
    let mut used = base;
    for &i in order {
        if let Some(&w) = widths.get(i)
            && used + w <= available
        {
            used += w;
            shown[i] = true;
        }
    }
    shown
}

fn text_width(ui: &Ui, text: &str) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), theme::ui_font(12.5), egui::Color32::WHITE).size().x
}

/// Whether any cell or spill holds an error, cached per workbook snapshot: scanning every cell each frame stalls big files.
fn workbook_has_errors(ui: &egui::Ui, d: &gridcraft_engine::DocState) -> bool {
    let key = (d.uid, d.revision, std::sync::Arc::as_ptr(&d.wb) as usize);
    let id = egui::Id::new("gridcraft.workbook_has_errors");
    if let Some((cached, has)) = ui.data(|m| m.get_temp::<((u64, u64, usize), bool)>(id))
        && cached == key
    {
        return has;
    }
    let has = d.wb.sheets.iter().any(|sh| sh.cells.iter().any(|(_, cell)| cell.value.is_error()) || sh.spill.values().any(|value| value.is_error()));
    ui.data_mut(|m| m.insert_temp(id, (key, has)));
    has
}

#[cfg(test)]
mod tests {
    use super::{fit_in, navigator_items, rename_needs_focus};

    #[test]
    fn status_items_fit_by_priority_and_skip_what_does_not_fit() {
        let widths = [100.0, 110.0, 120.0, 40.0];
        // Base 50: index 2 (120) fits, then 0 (100) doesn't in 250, 3 (40) does, 1 doesn't.
        assert_eq!(fit_in(250.0, 50.0, &widths, &[2, 0, 3, 1]), vec![false, false, true, true]);
        assert_eq!(fit_in(1000.0, 50.0, &widths, &[2, 0, 3, 1]), vec![true; 4]);
        assert_eq!(fit_in(f32::NAN, 50.0, &widths, &[2, 0, 3, 1]), vec![false; 4]);
        assert_eq!(fit_in(500.0, 0.0, &widths, &[9]), vec![false; 4], "an index past the list is ignored");
    }
    use gridcraft_engine::model::{Sheet, Visibility};
    use std::sync::Arc;

    #[test]
    fn rename_field_takes_focus_once_and_lets_go_on_commit() {
        assert!(rename_needs_focus(false, false), "first frame: grab focus");
        assert!(!rename_needs_focus(true, false), "already focused: do not ask again");
        assert!(!rename_needs_focus(false, true), "Enter or click-away: must not take focus back");
    }

    #[test]
    fn navigator_lists_visible_sheets_by_workbook_index() {
        let mut hidden = Sheet::new("Hidden");
        hidden.visibility = Visibility::Hidden;
        let mut very_hidden = Sheet::new("VeryHidden");
        very_hidden.visibility = Visibility::VeryHidden;
        let sheets = [Sheet::new("Data"), hidden, very_hidden, Sheet::new("Summary")].map(Arc::new);
        assert_eq!(navigator_items(&sheets), vec![("0".to_string(), "Data".to_string()), ("3".to_string(), "Summary".to_string())]);
    }
}
