//! External clipboard import modes and regional formula entry.

use super::*;

pub(super) fn special(s: &mut Session, p: &Json) -> Result<Json> {
    let keep_source = match str_param(p, "what").unwrap_or("keepSource") {
        "all" | "keepSource" => true,
        "matchDestination" | "values" => false,
        other => return Err(bad("edit.pasteSpecial", format!("unsupported external `what`: {other}"))),
    };
    if str_param(p, "text").is_none() && str_param(p, "html").is_none() {
        return Err(EngineError::Other("Nothing to paste.".into()));
    }
    if str_param(p, "operation").is_some_and(|op| op != "none")
        || ["skipBlanks", "transpose", "link"].iter().any(|key| bool_param(p, key) == Some(true))
    {
        return Err(bad("edit.pasteSpecial", "external paste does not support operations, skipBlanks, transpose or link"));
    }
    paste(s, p, str_param(p, "text").unwrap_or_default(), keep_source)
}

pub(super) fn paste(s: &mut Session, p: &Json, text: &str, keep_source: bool) -> Result<Json> {
    let sheet = target_sheet(s, p)?;
    let region = display_region(s, p, "edit.paste")?;
    let at = cell_param_on(s, p, "at", sheet)?.unwrap_or(s.doc()?.selection.current().start);
    let html = str_param(p, "html").map(crate::clip_html::table_cells).unwrap_or_default();
    let rows: Vec<Vec<String>> =
        if !html.is_empty() { html.iter().map(|row| row.iter().map(|cell| cell.text.clone()).collect()).collect() } else { parse_tsv(text) };
    let h = rows.len();
    let w = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut formula_error = None;
    edit(s, |cx| {
        if keep_source && !html.is_empty() && h > 0 && w > 0 {
            let end = at.offset_clamped(h as i64 - 1, w as i64 - 1);
            let pasted = RangeRef::new(at, end);
            cx.sheet_mut(sheet)?.merges.retain(|m| !m.intersects(&pasted));
        }
        for (ri, row) in rows.iter().enumerate() {
            for (ci, input) in row.iter().enumerate() {
                let Some(c) = at.offset(ri as i64, ci as i64) else { continue };
                let destination = cx.wb.sheet(sheet).map(|sh| sh.style_id(c)).unwrap_or_default();
                let hc = html.get(ri).and_then(|r| r.get(ci));
                let mut cell = if let Some(hc) = hc {
                    let mut source = hc.clone();
                    if region == Region::ViVn
                        && source.num.is_none()
                        && !source.string
                        && source.style.num_fmt.as_str() != "@"
                        && input.starts_with('=')
                    {
                        match canonical_formula(input, sheet, &cx.wb) {
                            Ok(text) => source.text = text,
                            Err(error) => {
                                formula_error.get_or_insert_with(|| error.to_string());
                                source.string = true;
                            }
                        }
                    }
                    Some(source.to_cell(cx.wb.date_system, region, &mut cx.wb.styles))
                } else {
                    match localized_input(input, sheet, c, &mut cx.wb, region) {
                        Ok(cell) => cell,
                        Err(error) => {
                            formula_error.get_or_insert_with(|| error.to_string());
                            Some(Cell { value: Value::text(input.as_str()), formula: None, style: destination })
                        }
                    }
                };
                if !keep_source && let Some(cell) = cell.as_mut() {
                    cell.style = destination;
                    if let Some(formula) = &cell.formula {
                        cell.value = gridcraft_calc::evaluate(&cx.wb, sheet, c, &formula.text);
                    }
                    cell.formula = None;
                }
                let sh = cx.sheet_mut(sheet)?;
                if keep_source
                    && let Some(hc) = hc.filter(|hc| hc.rowspan > 1 || hc.colspan > 1)
                    && let Some(end) = c.offset(hc.rowspan.saturating_sub(1) as i64, hc.colspan.saturating_sub(1) as i64)
                {
                    sh.merges.push(RangeRef::new(c, end));
                }
                match cell {
                    Some(cell) => sh.set_cell(c, cell),
                    None => {
                        sh.remove_cell(c);
                    }
                }
                cx.touch(sheet, c);
            }
        }
        if h > 0 && w > 0 {
            let end = at.offset_clamped(h as i64 - 1, w as i64 - 1);
            *cx.sel = crate::Selection { active: at, anchor: at, ranges: vec![RangeRef::new(at, end)] };
        }
        Ok(())
    })?;
    if let Some(error) = formula_error {
        s.ui_requests.push(crate::UiRequest::Message(error));
    }
    Ok(json!({"rows": h, "cols": w}))
}

fn localized_input(input: &str, sheet: usize, at: CellRef, wb: &mut gridcraft_model::Workbook, region: Region) -> Result<Option<Cell>> {
    if region != Region::ViVn
        || !input.starts_with('=')
        || wb.sheet(sheet).is_some_and(|sh| wb.styles.get(sh.style_id(at)).num_fmt.as_str() == "@")
    {
        return input_to_cell_in(input, sheet, at, wb, region);
    }
    let canonical = canonical_formula(input, sheet, wb)?;
    input_to_cell_in(&canonical, sheet, at, wb, region)
}

fn canonical_formula(input: &str, sheet: usize, wb: &gridcraft_model::Workbook) -> Result<String> {
    let known = |name: &str| wb.name(name, sheet).is_some() || wb.table(name).is_some();
    let mut body = input.to_string();
    let mut parsed = gridcraft_formula::parse_input(&body, gridcraft_formula::FormulaLocale::Vi, known);
    for _ in 0..8 {
        if parsed.is_ok() {
            break;
        }
        body.push(')');
        parsed = gridcraft_formula::parse_input(&body, gridcraft_formula::FormulaLocale::Vi, known);
    }
    // An unambiguous canonical formula copied from another locale remains usable.
    if parsed.is_err() {
        body = input.strip_prefix('=').unwrap_or(input).to_string();
        parsed = gridcraft_formula::parse(&body);
        for _ in 0..8 {
            if parsed.is_ok() {
                break;
            }
            body.push(')');
            parsed = gridcraft_formula::parse(&body);
        }
    }
    let expr = parsed.map_err(|e| EngineError::Other(format!("There's a problem with this formula: {e}")))?;
    Ok(format!("={}", gridcraft_formula::print(&expr)))
}
