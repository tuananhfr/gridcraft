//! Co-editing on the grid (web, `erpHost` v3): an outline with the holder's name around each
//! region another editor has locked, and the other editors' selections. Nothing is drawn without
//! co-editing.

use egui::{Align2, Color32, FontId, Painter, Rect, Stroke, StrokeKind, pos2};
use gridcraft_engine::core::RangeRef;
use gridcraft_engine::live::Region;
use gridcraft_engine::model::Sheet;

use crate::SheetApp;
use crate::grid::Geo;

/// `#rrggbb` as the shell sends member colours; anything else gets a neutral blue.
pub(crate) fn colour(css: &str) -> Color32 {
    let hex = css.trim().trim_start_matches('#');
    let channel = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    match (hex.len(), channel(0), channel(2), channel(4)) {
        (6, Some(r), Some(g), Some(b)) => Color32::from_rgb(r, g, b),
        _ => Color32::from_rgb(0x33, 0x66, 0xcc),
    }
}

fn tag(p: &Painter, at: egui::Pos2, text: &str, fill: Color32) {
    if text.is_empty() {
        return;
    }
    let font = FontId::proportional(11.0);
    let galley = p.layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let rect = Rect::from_min_size(pos2(at.x, at.y - galley.size().y - 2.0), galley.size() + egui::vec2(6.0, 2.0));
    p.rect_filled(rect, 2.0, fill);
    p.galley(rect.min + egui::vec2(3.0, 1.0), galley, Color32::WHITE);
}

/// Paints others' locks and selections on sheet `sh` (clipped to the cells by the caller).
pub(crate) fn paint(app: &SheetApp, p: &Painter, geo: &Geo, sh: &Sheet) {
    let Some(live) = app.session.live.as_ref() else { return };
    if !app.session.live_serves_active() {
        return;
    }
    for cursor in live.cursors() {
        let Some(Region::Cell { sheet, .. }) = Region::parse(&cursor.region) else { continue };
        if sheet != sh.uid {
            continue;
        }
        let c = colour(&cursor.colour);
        let mut first = true;
        for range in cursor.range.split(',').filter_map(RangeRef::parse) {
            let r = geo.range_rect(sh, range);
            if !r.intersects(geo.cells) {
                continue;
            }
            p.rect_stroke(r, 0.0, Stroke::new(1.5, c), StrokeKind::Inside);
            if first {
                tag(p, r.left_top(), &cursor.name, c);
                first = false;
            }
        }
    }
    for lock in live.others() {
        let c = colour(&lock.colour);
        match Region::parse(&lock.region) {
            Some(Region::Cell { sheet, cell }) if sheet == sh.uid => {
                let at = sh.merge_at(cell).unwrap_or(RangeRef::new(cell, cell));
                let r = geo.range_rect(sh, at);
                if r.intersects(geo.cells) {
                    p.rect_filled(r, 0.0, c.gamma_multiply(0.12));
                    p.rect_stroke(r, 0.0, Stroke::new(2.0, c), StrokeKind::Inside);
                    tag(p, r.left_top(), &lock.name, c);
                }
            }
            Some(Region::Sheet(sheet)) if sheet == sh.uid => whole(p, geo, &lock.name, c),
            Some(Region::Book) => whole(p, geo, &lock.name, c),
            _ => {}
        }
    }
}

fn whole(p: &Painter, geo: &Geo, name: &str, c: Color32) {
    p.rect_stroke(geo.cells.shrink(1.0), 0.0, Stroke::new(3.0, c), StrokeKind::Inside);
    let font = FontId::proportional(12.0);
    let at = pos2(geo.cells.right() - 6.0, geo.cells.top() + 6.0);
    let galley = p.layout_no_wrap(name.to_string(), font, Color32::WHITE);
    let rect = Align2::RIGHT_TOP.anchor_size(at, galley.size() + egui::vec2(8.0, 4.0));
    p.rect_filled(rect, 3.0, c);
    p.galley(rect.min + egui::vec2(4.0, 2.0), galley, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_colours_parse_and_fall_back() {
        assert_eq!(colour("#ff8000"), Color32::from_rgb(0xff, 0x80, 0x00));
        assert_eq!(colour("nope"), Color32::from_rgb(0x33, 0x66, 0xcc));
        assert_eq!(colour("#ff80"), Color32::from_rgb(0x33, 0x66, 0xcc));
    }
}
