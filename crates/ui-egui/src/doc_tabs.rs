//! The document tab strip under the title bar (one tab per open workbook, shown once two are
//! open) and switching between and closing those documents.

use egui::{CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use gridcraft_engine::Session;
use serde_json::json;

use crate::SheetApp;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets::tr;

pub const STRIP_H: f32 = 28.0;
const TAB_MIN_W: f32 = 96.0;
const TAB_MAX_W: f32 = 220.0;
const CLOSE_W: f32 = 18.0;
const DOT_W: f32 = 12.0;
const PAD: f32 = 8.0;

/// One document tab.
#[derive(Clone, Debug, PartialEq)]
pub struct DocTab {
    /// Index into [`Session::documents`].
    pub index: usize,
    pub title: String,
    pub dirty: bool,
    pub active: bool,
}

/// The tabs to show, empty below two so a single workbook looks as before. Windows hidden with
/// View ▸ Hide get none: View ▸ Unhide brings them back, as in Excel.
pub fn tabs(session: &Session) -> Vec<DocTab> {
    let tabs: Vec<DocTab> = session
        .documents()
        .iter()
        .enumerate()
        .filter(|(i, _)| !session.is_hidden(*i))
        .map(|(i, d)| DocTab { index: i, title: d.display_title(), dirty: d.is_dirty(), active: i == session.active_index() })
        .collect();
    if tabs.len() < 2 { Vec::new() } else { tabs }
}

enum Click {
    Activate,
    Close,
    CloseOthers,
    CloseAll,
    Reopen,
}

pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    let tabs = tabs(&app.session);
    if tabs.is_empty() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    let can_reopen = app.can_reopen_closed();
    egui::Panel::top("document_tabs").exact_size(STRIP_H).frame(egui::Frame::NONE.fill(t.window)).show(ui, |ui| {
        let rect = ui.max_rect();
        ui.painter()
            .line_segment([pos2(rect.left(), rect.bottom() - 0.5), pos2(rect.right(), rect.bottom() - 0.5)], Stroke::new(1.0, t.ribbon_border));
        let width = ((rect.width() - 2.0 * PAD) / tabs.len() as f32).clamp(TAB_MIN_W, TAB_MAX_W);
        // Too many tabs for a narrow window scroll: bring the active one into view when it or the
        // layout changes, but let the user browse the others without snapping back.
        let active = app.session.active().map(|d| d.uid);
        let reveal_key = egui::Id::new((active, tabs.len(), width.to_bits()));
        let reveal_id = ui.id().with("document_tabs_reveal");
        let reveal = ui.ctx().data_mut(|data| {
            let changed = data.get_temp::<egui::Id>(reveal_id) != Some(reveal_key);
            data.insert_temp(reveal_id, reveal_key);
            changed
        });
        egui::ScrollArea::horizontal()
            .id_salt("document_tabs_scroll")
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    ui.add_space(PAD);
                    for tab in &tabs {
                        if let Some(c) = tab_button(ui, &t, tab, width, reveal, can_reopen) {
                            clicked = Some((tab.index, c));
                        }
                    }
                });
            });
    });
    match clicked {
        Some((i, Click::Close)) => app.close_document_at(i),
        Some((i, Click::Activate)) => {
            app.activate_document(i);
        }
        Some((i, Click::CloseOthers)) => {
            if let Some(uid) = app.session.documents().get(i).map(|d| d.uid) {
                app.close_others(uid);
            }
        }
        Some((_, Click::CloseAll)) => app.close_all(),
        Some((_, Click::Reopen)) => app.run_or_alert("file.reopenClosed", json!({})),
        None => return,
    }
    app.note_active_document();
}

fn tab_button(ui: &mut Ui, t: &Tokens, tab: &DocTab, width: f32, reveal: bool, can_reopen: bool) -> Option<Click> {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, STRIP_H), Sense::click());
    if tab.active && reveal {
        resp.scroll_to_me(None);
    }
    let body = Rect::from_min_max(pos2(rect.left(), rect.top() + 4.0), rect.max);
    let top = CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 };
    let painter = ui.painter();
    if tab.active {
        // Covers the strip's bottom line, so the active tab joins the ribbon below it.
        painter.rect_filled(body, top, t.ribbon);
        let edge = Stroke::new(1.0, t.ribbon_border);
        painter.line_segment([body.left_bottom(), body.left_top() + vec2(0.0, 2.0)], edge);
        painter.line_segment([body.right_bottom(), body.right_top() + vec2(0.0, 2.0)], edge);
        painter.rect_filled(Rect::from_min_size(body.min, vec2(body.width(), 2.0)), top, t.accent);
    } else if resp.hovered() {
        painter.rect_filled(body.shrink2(vec2(0.0, 1.0)), top, t.hover);
    }
    let close = Rect::from_center_size(pos2(body.right() - 6.0 - CLOSE_W / 2.0, body.center().y), vec2(CLOSE_W, CLOSE_W));
    let text_left = body.left() + 10.0;
    let dot = if tab.dirty { DOT_W } else { 0.0 };
    let color = if tab.active { t.text } else { t.text_dim };
    let font = if tab.active { theme::ui_bold(12.5) } else { theme::ui_font(12.5) };
    let galley = painter.layout_job(elided(&tab.title, font, color, close.left() - 4.0 - dot - text_left));
    painter.galley(pos2(text_left, body.center().y - galley.size().y / 2.0), galley, color);
    if tab.dirty {
        painter.circle_filled(pos2(close.left() - 2.0 - DOT_W / 2.0, body.center().y), 3.5, if tab.active { t.accent } else { t.text_dim });
    }
    let close_resp = ui.interact(close, resp.id.with("close"), Sense::click());
    if close_resp.hovered() {
        ui.painter().rect_filled(close, 3.0, t.pressed);
    }
    icons::paint(ui.painter(), close.shrink(3.0), Icon::Close, if close_resp.hovered() { t.text } else { t.text_dim });
    let close_label = tr(ui, "Close {title}").replace("{title}", &tab.title);
    close_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &close_label));
    let close_resp = close_resp.on_hover_text(tr(ui, "Close"));
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, tab.active, &tab.title));
    let tip = if tab.dirty { format!("{}\n{}", tab.title, tr(ui, "Unsaved changes")) } else { tab.title.clone() };
    let resp = resp.on_hover_text(tip);
    let mut menu = None;
    resp.context_menu(|ui| {
        for (label, click) in [("Close", Click::Close), ("Close Others", Click::CloseOthers), ("Close All", Click::CloseAll)] {
            if ui.button(tr(ui, label)).clicked() {
                menu = Some(click);
            }
        }
        ui.separator();
        if ui.add_enabled(can_reopen, egui::Button::new(tr(ui, "Reopen Closed Document"))).clicked() {
            menu = Some(Click::Reopen);
        }
    });
    if menu.is_some() {
        menu
    } else if close_resp.clicked() || resp.middle_clicked() {
        Some(Click::Close)
    } else if resp.clicked() && !tab.active {
        Some(Click::Activate)
    } else {
        None
    }
}

/// `text` on one line, cut with "…" where it would pass `max_width`.
fn elided(text: &str, font: egui::FontId, color: egui::Color32, max_width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: max_width.max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    job.halign = egui::Align::LEFT;
    job
}

impl SheetApp {
    /// The index of open document `uid`, or of the active document for `None`.
    pub fn document_index(&self, uid: Option<u64>) -> Option<usize> {
        match uid {
            Some(uid) => self.session.documents().iter().position(|d| d.uid == uid),
            None => self.session.active().map(|_| self.session.active_index()),
        }
    }

    /// Makes document `i` active through `window.activate`, after committing the cell or header
    /// being edited (it belongs to the document being left). `false` when that edit can't commit.
    pub fn activate_document(&mut self, i: usize) -> bool {
        if i == self.session.active_index() {
            return true;
        }
        if !self.commit_edit(0, 0, false, false) {
            return false;
        }
        crate::paper_view::commit(self);
        self.run_or_alert("window.activate", json!({"index": i}));
        true
    }

    /// Closes document `i` through `file.close`, which asks first when it has unsaved changes.
    pub fn close_document_at(&mut self, i: usize) {
        if i == self.session.active_index() && !self.commit_edit(0, 0, false, false) {
            return;
        }
        self.run_or_alert("file.close", json!({"index": i}));
    }

    /// Runs `f` with document `uid` (`None`: the active one) active, then returns to the document
    /// that was active if it is still open. Nothing runs when `uid` is no longer open.
    pub fn in_document(&mut self, uid: Option<u64>, f: impl FnOnce(&mut SheetApp)) {
        let Some(i) = self.document_index(uid) else { return };
        let back = self.session.active().map(|d| d.uid);
        self.session.set_active(i);
        f(self);
        if let Some(j) = back.and_then(|b| self.document_index(Some(b))) {
            self.session.set_active(j);
        }
    }

    /// Drops interface state tied to the previously shown document once another one is active,
    /// however it got there (a tab, Open, Close, the control channel): it would act on the wrong
    /// workbook.
    pub fn note_active_document(&mut self) {
        let uid = self.session.active().map(|d| d.uid);
        let previous = std::mem::replace(&mut self.shown_doc, uid);
        if previous.is_none() || previous == uid {
            return;
        }
        if self.editor.is_some() {
            self.cancel_edit();
        }
        crate::paper_view::cancel(self);
        self.selected_chart = None;
        self.grid.trace = None;
        self.grid.renaming_tab = None;
        self.grid.list_picker = None;
        self.grid.filter_menu = None;
        self.grid.context_menu = None;
        self.grid.header_menu = None;
        self.grid.ruler_drag = None;
        self.grid.ink.clear();
    }
}
