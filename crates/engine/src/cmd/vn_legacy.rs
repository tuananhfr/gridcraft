//! Vietnamese legacy conversion through the normal edit transaction.

use std::sync::Arc;

use gridcraft_core::Value;
use gridcraft_model::style::StyleId;
use serde_json::{Value as Json, json};

use super::*;
use crate::vn_legacy::{Encoding, encoding};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "review.convertVietnameseLegacy",
            "Convert TCVN3/VNI to Unicode",
            ["Review", "Language"],
            None,
            "{} -> {convertedCells, skippedFormulaCells}",
            has_doc,
            |s, _| convert(s, None)
        ),
        cmd!(
            "review.convertTcvn3",
            "Convert TCVN3 to Unicode",
            ["Review", "Language"],
            None,
            "{} (selected cells) -> {convertedCells, skippedFormulaCells}",
            selected,
            |s, _| convert(s, Some(Encoding::Tcvn3))
        ),
        cmd!(
            "review.convertVni",
            "Convert VNI to Unicode",
            ["Review", "Language"],
            None,
            "{} (selected cells) -> {convertedCells, skippedFormulaCells}",
            selected,
            |s, _| convert(s, Some(Encoding::Vni))
        ),
    ]
}

fn selected(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|d| !d.selection.ranges.is_empty()) { Ok(()) } else { Err("Select cells to convert.".into()) }
}

fn convert(s: &mut Session, forced: Option<Encoding>) -> Result<Json> {
    let d = s.doc()?;
    let selection = d.selection.clone();
    let active = d.wb.active_sheet;
    let scoped = forced.is_some() || !selection.is_single_cell();
    let bytes =
        d.wb.sheets
            .iter()
            .enumerate()
            .filter(|(si, _)| !scoped || *si == active)
            .flat_map(|(_, sheet)| sheet.cells.iter())
            .filter(|(pos, cell)| cell.formula.is_none() && (!scoped || selection.contains(*pos)))
            .filter_map(|(_, cell)| if let Value::Text(text) = &cell.value { Some(text.len()) } else { None })
            .fold(0usize, usize::saturating_add);
    if bytes > 64 * 1024 * 1024 {
        return Err(EngineError::Other("The conversion is too large. Select a smaller range.".into()));
    }
    edit(s, |cx| {
        let font = cx
            .wb
            .cell_styles
            .iter()
            .find(|(name, _)| name == "Normal")
            .map(|(_, style)| style.font.name.clone())
            .unwrap_or_else(|| cx.wb.styles.get(StyleId::DEFAULT).font.name.clone());
        let font = if encoding(&font).is_some() || font.trim().is_empty() { gridcraft_model::DEFAULT_FONT.into() } else { font };
        let mut converted = 0usize;
        let mut skipped = 0usize;
        for si in 0..cx.wb.sheets.len() {
            if scoped && si != active {
                continue;
            }
            let Some(sheet) = cx.wb.sheets.get(si) else { continue };
            let mut changes = Vec::new();
            for (pos, cell) in sheet.cells.iter() {
                if scoped && !selection.contains(pos) {
                    continue;
                }
                if cell.formula.is_some() {
                    skipped = skipped.saturating_add(1);
                    continue;
                }
                let Value::Text(text) = &cell.value else { continue };
                let style = sheet.style_id(pos);
                let detected = encoding(&cx.wb.styles.get(style).font.name);
                let Some(choice) = forced.or(detected) else { continue };
                let mut replacement = cell.clone();
                replacement.value = Value::from(choice.convert(text));
                changes.push((pos, replacement, style, detected.is_some()));
            }
            for (pos, mut cell, style, legacy) in changes {
                if legacy {
                    cell.style = cx.wb.styles.derive(style, |st| st.font.name = font.clone());
                }
                if let Some(sheet) = cx.wb.sheets.get_mut(si) {
                    let sheet = Arc::make_mut(sheet);
                    sheet.set_cell(pos, cell);
                    cx.changed.push((si, pos));
                    converted = converted.saturating_add(1);
                }
            }
        }
        Ok(json!({"convertedCells": converted, "skippedFormulaCells": skipped}))
    })
}
