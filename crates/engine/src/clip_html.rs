//! Reading the HTML flavour of the system clipboard on paste.
//!
//! Spreadsheets put an HTML table on the clipboard next to the tab-separated text. The text holds
//! what each cell *shows* ("Rp10,000.00"), which en-US input parsing can't always turn back into a
//! number. The HTML of the common sources also carries the underlying number as a cell attribute:
//! `x:num="10000"` (Excel, Excel for the web) and `sdval="10000"` (LibreOffice Calc). Paste uses
//! it to keep such cells numeric (#257). Formatting is limited to the clipboard CSS allow-list.

use gridcraft_model::Style;

use crate::clip_html_inline::WholeCellStyle;
use crate::clip_html_style::{self, Rules};

const MAX_CELLS: usize = 20_000;
/// Widest `colspan` honoured (the sheet's column count).
const MAX_SPAN: usize = gridcraft_core::MAX_COLS as usize;

/// One table cell: its visible text and the number the source recorded for it, if any.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HtmlCell {
    pub text: String,
    pub num: Option<f64>,
    pub string: bool,
    pub style: Style,
    pub rowspan: usize,
    pub colspan: usize,
    preserve: bool,
    breaks: Vec<usize>,
    inline: WholeCellStyle,
}

impl HtmlCell {
    pub fn to_cell(
        &self,
        date_system: gridcraft_core::DateSystem,
        region: gridcraft_core::Region,
        styles: &mut gridcraft_model::StyleTable,
    ) -> gridcraft_model::Cell {
        let mut style = self.style.clone();
        if self.num.is_none()
            && !self.string
            && style.num_fmt.as_str() != "@"
            && let Some(body) = self.text.strip_prefix('=')
            && let Ok(expr) = gridcraft_formula::parse(body)
        {
            let mut cell = gridcraft_model::Cell::formula(gridcraft_model::Formula::from_expr(expr));
            cell.style = styles.intern(style);
            return cell;
        }
        let value = if let Some(n) = self.num {
            let general =
                gridcraft_numfmt::format_value_in(&gridcraft_core::Value::number(n), &gridcraft_numfmt::NumberFormat::general(), date_system, region)
                    .text;
            if style.num_fmt.as_str() == "General"
                && normalized(&general) != normalized(&self.text)
                && let Some(code) = format_for(&self.text, n)
            {
                style.num_fmt = gridcraft_model::NumFmt::new(&code);
            }
            gridcraft_core::Value::number(n)
        } else if self.string || style.num_fmt.as_str() == "@" {
            gridcraft_core::Value::text(self.text.as_str())
        } else {
            let parsed = gridcraft_core::parse::parse_input_in(&self.text, date_system, region);
            if style.num_fmt.as_str() == "General"
                && let Some(code) = parsed.format
            {
                style.num_fmt = gridcraft_model::NumFmt::new(code);
            }
            parsed.value
        };
        gridcraft_model::Cell { value, formula: None, style: styles.intern(style) }
    }
}

/// The cells of the first table in `html`, row by row; a cell spanning columns is followed by
/// empty cells so positions line up with the tab-separated text. Empty when there is no table.
pub(crate) fn table_cells(html: &str) -> Vec<Vec<HtmlCell>> {
    if !crate::clipboard_limits::accepts_html(html) {
        return Vec::new();
    }
    parse_table(html).unwrap_or_default()
}

fn parse_table(html: &str) -> Option<Vec<Vec<HtmlCell>>> {
    let mut grid = Grid::default();
    let mut rules = Rules::default();
    let mut stack: Vec<(String, Style, bool, bool)> = Vec::new();
    let mut cell: Option<HtmlCell> = None;
    let mut in_table = false;
    let mut row_hidden = false;
    let mut rest = html;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            append_text(&mut cell, &stack, rest);
            break;
        };
        append_text(&mut cell, &stack, rest.get(..lt).unwrap_or_default());
        rest = rest.get(lt..).unwrap_or_default();
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").and_then(|e| after.get(e + 3..)).unwrap_or_default();
            continue;
        }
        let Some(gt) = tag_end(rest) else { break };
        let tag = rest.get(1..gt).unwrap_or_default();
        rest = rest.get(gt + 1..).unwrap_or_default();
        let closing = tag.starts_with('/');
        let body = tag.trim_start_matches('/');
        let name_end = body.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(body.len());
        let name = body.get(..name_end).unwrap_or_default().to_ascii_lowercase();
        let attrs = body.get(name_end..).unwrap_or_default();
        if closing {
            if matches!(name.as_str(), "td" | "th" | "tr" | "table") {
                grid.finish(&mut cell)?;
            }
            if let Some(i) = stack.iter().rposition(|s| s.0 == name) {
                stack.truncate(i);
                if let Some(c) = cell.as_mut() {
                    c.inline.truncate(i);
                }
            }
            if name == "table" && in_table {
                break;
            }
            continue;
        }
        if matches!(name.as_str(), "td" | "th" | "tr") {
            grid.finish(&mut cell)?;
            let close = if name == "tr" { "tr" } else { "td" };
            if let Some(i) = stack.iter().rposition(|s| s.0 == close || close == "td" && s.0 == "th") {
                stack.truncate(i);
            }
        }
        let (mut style, parent_hidden) = stack.last().map(|s| (s.1.clone(), s.2)).unwrap_or_default();
        let mut preserve = stack.last().is_some_and(|s| s.3);
        let mut hidden = false;
        clip_html_style::apply_tag(&name, attrs, &mut style);
        rules.apply(&name, attr(attrs, "class").unwrap_or_default(), &mut style, &mut hidden, &mut preserve);
        if let Some(css) = attr(attrs, "style") {
            if css.len() > clip_html_style::MAX_STYLE {
                return None;
            }
            clip_html_style::apply(&decode_entities(css), &mut style, &mut hidden, &mut preserve);
        }
        let number_format = if matches!(name.as_str(), "td" | "th") {
            attr(attrs, "sdnum").and_then(|v| v.splitn(3, ';').nth(2)).and_then(clip_html_style::number_format)
        } else {
            None
        };
        if let Some(code) = number_format.as_ref() {
            style.num_fmt = gridcraft_model::NumFmt::new(code);
        }
        hidden |= parent_hidden || has_attr(attrs, "hidden");
        if !is_void(&name) && !tag.ends_with('/') {
            if stack.len() >= 64 {
                return None;
            }
            stack.push((name.clone(), style.clone(), hidden, preserve));
        }
        match (name.as_str(), closing) {
            ("style" | "script" | "caption", false) => {
                let end = format!("</{name}");
                let i = find_ci(rest, &end).unwrap_or(rest.len());
                if name == "style" {
                    rules.add(rest.get(..i).unwrap_or_default());
                }
                rest = rest.get(i..).unwrap_or_default();
            }
            ("table", false) => {
                if in_table {
                    return None;
                }
                in_table = true;
            }
            ("tr", false) if in_table => {
                row_hidden = hidden;
                if hidden {
                    grid.skip_row();
                } else {
                    grid.start_row()?;
                }
            }
            ("td" | "th", false) if in_table => {
                if row_hidden || hidden {
                    continue;
                }
                let num = attr(attrs, "x:num").or_else(|| attr(attrs, "sdval")).and_then(|v| v.trim().parse::<f64>().ok()).filter(|n| n.is_finite());
                let colspan = span(attrs, "colspan", MAX_SPAN)?;
                let rowspan = span(attrs, "rowspan", MAX_CELLS)?;
                let inline = WholeCellStyle::new(attr(attrs, "style").map(decode_entities).unwrap_or_default(), number_format.is_some());
                cell = Some(HtmlCell { num, string: has_attr(attrs, "x:str"), style, rowspan, colspan, preserve, inline, ..Default::default() });
            }
            ("br", _) => {
                if let Some(c) = cell.as_mut().filter(|_| !hidden) {
                    c.breaks.push(c.text.len());
                }
            }
            _ => {}
        }
    }
    grid.finish(&mut cell)?;
    if in_table && grid.rows.is_empty() {
        grid.rows.push(Vec::new());
    }
    Some(grid.rows)
}

fn append_text(cell: &mut Option<HtmlCell>, stack: &[(String, Style, bool, bool)], text: &str) {
    let Some(c) = cell.as_mut().filter(|_| !stack.last().is_some_and(|s| s.2)) else { return };
    c.text.push_str(text);
    if decode_entities(text).chars().any(|ch| !ch.is_whitespace()) {
        let depth = c.inline.depth(stack.len());
        if let Some((_, style, _, preserve)) = depth.checked_sub(1).and_then(|i| stack.get(i)) {
            c.inline.observe(depth, style, *preserve);
        } else {
            c.inline.observe(0, &c.style, c.preserve);
        }
    }
}

#[derive(Default)]
struct Grid {
    rows: Vec<Vec<HtmlCell>>,
    occupied: std::collections::BTreeSet<(usize, usize)>,
    row: Option<usize>,
    col: usize,
    width: usize,
}

impl Grid {
    fn start_row(&mut self) -> Option<()> {
        let row = self.row.map_or(0, |r| r.saturating_add(1));
        if row >= MAX_CELLS || row.saturating_add(1).checked_mul(self.width)? > MAX_CELLS {
            return None;
        }
        self.row = Some(row);
        self.col = 0;
        if self.rows.len() <= row {
            self.rows.push(vec![HtmlCell::default(); self.width]);
        }
        Some(())
    }

    fn skip_row(&mut self) {
        let next = self.row.map_or(0, |r| r.saturating_add(1));
        if next >= self.rows.len() {
            return;
        }
        self.rows.remove(next);
        self.occupied =
            self.occupied.iter().filter_map(|&(r, c)| if r == next { None } else { Some((if r > next { r - 1 } else { r }, c)) }).collect();
        for (r, cells) in self.rows.iter_mut().enumerate().take(next) {
            for cell in cells {
                if r.saturating_add(cell.rowspan) > next {
                    cell.rowspan = cell.rowspan.saturating_sub(1);
                }
            }
        }
    }

    fn finish(&mut self, cell: &mut Option<HtmlCell>) -> Option<()> {
        let Some(mut cell) = cell.take() else { return Some(()) };
        if self.row.is_none() {
            self.start_row()?;
        }
        let row = self.row?;
        while self.occupied.contains(&(row, self.col)) {
            self.col = self.col.checked_add(1)?;
        }
        let end_row = row.checked_add(cell.rowspan)?;
        let end_col = self.col.checked_add(cell.colspan)?;
        let width = self.width.max(end_col);
        let height = self.rows.len().max(end_row);
        if width > MAX_SPAN || height.checked_mul(width)? > MAX_CELLS {
            return None;
        }
        for r in row..end_row {
            for c in self.col..end_col {
                if !self.occupied.insert((r, c)) {
                    return None;
                }
            }
        }
        self.rows.resize_with(height, || vec![HtmlCell::default(); width]);
        if width != self.width {
            for cells in &mut self.rows {
                cells.resize(width, HtmlCell::default());
            }
        }
        self.width = width;
        cell.inline.finish(&mut cell.style, &mut cell.preserve);
        let mut text = String::new();
        let mut start = 0;
        for (i, end) in cell.breaks.iter().copied().chain(std::iter::once(cell.text.len())).enumerate() {
            if i > 0 {
                text.push('\n');
            }
            let raw = cell.text.get(start..end).unwrap_or_default();
            text.push_str(&if cell.preserve { decode_entities(raw) } else { decode(raw) });
            start = end;
        }
        cell.text = text;
        cell.breaks.clear();
        *self.rows.get_mut(row)?.get_mut(self.col)? = cell;
        self.col = end_col;
        Some(())
    }
}

fn span(attrs: &str, name: &str, cap: usize) -> Option<usize> {
    match attr(attrs, name) {
        Some(v) if v.trim().bytes().all(|b| b.is_ascii_digit()) && !v.trim().is_empty() => Some(v.trim().parse::<usize>().ok()?.clamp(1, cap)),
        _ => Some(1),
    }
}

fn is_void(name: &str) -> bool {
    matches!(name, "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "param" | "source" | "track" | "wbr")
        || name.starts_with(['!', '?'])
}

/// Byte index of the `>` that closes the tag `rest` starts with; a `>` inside a quoted attribute
/// value (an inline `mso-number-format` condition such as `[>=1000]`) doesn't end it.
fn tag_end(rest: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in rest.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// Case-insensitive ASCII search.
pub(crate) fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h.get(i..i + n.len()).is_some_and(|w| w.eq_ignore_ascii_case(n)))
}

/// A boolean attribute is present with an empty value.
pub(crate) fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let mut s = attrs;
    loop {
        s = s.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        if s.is_empty() {
            return None;
        }
        let key_end = s.find(|c: char| c.is_whitespace() || c == '=' || c == '/').unwrap_or(s.len());
        let key = s.get(..key_end).unwrap_or_default();
        s = s.get(key_end..).unwrap_or_default().trim_start();
        let mut value = None;
        if let Some(after) = s.strip_prefix('=') {
            let after = after.trim_start();
            let (v, next) = match after.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let inner = after.get(1..).unwrap_or_default();
                    let end = inner.find(q).unwrap_or(inner.len());
                    (inner.get(..end).unwrap_or_default(), inner.get(end + 1..).unwrap_or_default())
                }
                _ => {
                    let end = after.find(char::is_whitespace).unwrap_or(after.len());
                    (after.get(..end).unwrap_or_default(), after.get(end..).unwrap_or_default())
                }
            };
            value = Some(v);
            s = next;
        }
        if key.eq_ignore_ascii_case(name) {
            return Some(value.unwrap_or_default());
        }
        if key.is_empty() && value.is_none() {
            // Stray character (e.g. a lone '='): skip it so the loop always advances.
            let mut chars = s.chars();
            chars.next();
            s = chars.as_str();
        }
    }
}

fn has_attr(attrs: &str, name: &str) -> bool {
    attr(attrs, name).is_some()
}

pub(super) fn decode_entities(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(rest.get(..amp).unwrap_or_default());
        rest = rest.get(amp..).unwrap_or_default();
        let semi = rest.get(..12).unwrap_or(rest).find(';');
        let entity = semi.and_then(|e| rest.get(1..e));
        let ch = entity.and_then(|e| match e {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => {
                let n = e
                    .strip_prefix("#x")
                    .or_else(|| e.strip_prefix("#X"))
                    .map(|h| u32::from_str_radix(h, 16))
                    .or_else(|| e.strip_prefix('#').map(str::parse::<u32>));
                n.and_then(Result::ok).and_then(char::from_u32)
            }
        });
        match (ch, semi) {
            (Some(c), Some(e)) => {
                out.push(c);
                rest = rest.get(e + 1..).unwrap_or_default();
            }
            _ => {
                out.push('&');
                rest = rest.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode(raw: &str) -> String {
    let out = decode_entities(raw);
    let mut text = String::with_capacity(out.len());
    let mut space = false;
    for c in out.chars() {
        if c.is_whitespace() && c != '\u{a0}' {
            space = true;
            continue;
        }
        if space && !text.is_empty() {
            text.push(' ');
        }
        space = false;
        text.push(c);
    }
    text
}

/// Same-looking text: non-breaking spaces as spaces, whitespace runs collapsed, trimmed.
pub(crate) fn normalized(s: &str) -> String {
    s.replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A number format that shows `n` exactly as `shown` ("Rp10,000.00" for 10000 →
/// `"Rp"#,##0.00`), inferred from the text around the digits. `None` unless the format renders
/// `shown` back character for character.
pub(crate) fn format_for(shown: &str, n: f64) -> Option<String> {
    let shown = normalized(shown);
    let start = shown.find(|c: char| c.is_ascii_digit())?;
    let end = shown.rfind(|c: char| c.is_ascii_digit()).map(|i| i + 1)?;
    let core = shown.get(start..end)?;
    let mut prefix = shown.get(..start)?.to_string();
    let suffix = shown.get(end..)?;
    if core.chars().any(|c| !(c.is_ascii_digit() || c == ',' || c == '.')) {
        return None;
    }
    if n < 0.0 {
        // A one-section format shows the minus sign itself, in front of everything.
        prefix = prefix.strip_prefix('-')?.to_string();
    }
    let decimals = core.split_once('.').map_or(0, |(_, f)| f.len());
    let mut code = String::new();
    let quote = |s: &str| if s.is_empty() { String::new() } else { format!("\"{}\"", s.replace('"', "")) };
    code.push_str(&quote(&prefix));
    code.push_str(if core.contains(',') { "#,##0" } else { "0" });
    if decimals > 0 {
        code.push('.');
        code.push_str(&"0".repeat(decimals.min(30)));
    }
    code.push_str(&quote(suffix));
    let rendered = gridcraft_numfmt::text_function(&gridcraft_core::Value::number(n), &code, gridcraft_core::date::DateSystem::D1900).ok()?;
    (normalized(&rendered) == shown).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_excel_and_libreoffice_cell_values() {
        let excel = r#"<html><head><style>td{mso-number-format:General}</style></head><body><table>
            <!--StartFragment--><tr height=20><td class=xl65 x:num="10000">Rp10,000.00</td><td x:str>Name</td></tr>
            <tr><td colspan=2 x:num="2.5">2.50 &amp; up</td><td>x</td></tr><!--EndFragment--></table></body></html>"#;
        let rows = table_cells(excel);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].text, "Rp10,000.00");
        assert_eq!(rows[0][0].num, Some(10000.0));
        assert_eq!(rows[0][1].text, "Name");
        assert!(rows[0][1].string);
        assert_eq!(rows[1].len(), 3, "colspan keeps the next cell in its column");
        assert_eq!(rows[1][0].text, "2.50 & up");
        assert_eq!(rows[1][2].text, "x");

        let calc = r#"<table cellspacing="0" border="0"><colgroup width="85"></colgroup><tr>
            <td height="17" align="right" sdval="10000" sdnum="1033;0;[$Rp-421]#,##0.00"><font face="Liberation Sans">Rp10,000.00</font></td></tr></table>"#;
        assert_eq!(table_cells(calc)[0][0].num, Some(10000.0));
        assert_eq!(table_cells(calc)[0][0].text, "Rp10,000.00");

        let quoted_gt = r#"<table><tr><td style='mso-number-format:"\[>=1000\]0"' x:num="1500">1500</td></tr></table>"#;
        assert_eq!(table_cells(quoted_gt)[0][0].num, Some(1500.0));
    }

    #[test]
    fn hostile_markup_does_not_panic() {
        for html in [
            "",
            "<",
            "<td",
            "<table><td x:num=>",
            "<table><tr><td colspan=999999999>a",
            "&#xFFFFFFFF;&",
            "<table><td x:num='1e999'>1</td>",
            "<!--",
            "<style>",
            "<table><table><td>",
        ] {
            let _ = table_cells(html);
        }
        assert_eq!(table_cells("<table><td x:num='1e999'>1</td></table>")[0][0].num, None);
        assert!(table_cells("<table><tr><td colspan=999999999>a</td></tr></table>")[0].len() <= MAX_SPAN);
    }

    #[test]
    fn infers_a_format_that_shows_the_same_text() {
        assert_eq!(format_for("Rp10,000.00", 10000.0).as_deref(), Some("\"Rp\"#,##0.00"));
        assert_eq!(format_for("1 234 kr", 1234.0), None, "space grouping is not reproduced");
        assert_eq!(format_for("10.000,00 €", 10000.0), None, "comma decimals don't render back in en-US");
        assert_eq!(format_for("-Rp5", -5.0).as_deref(), Some("\"Rp\"0"));
        assert_eq!(format_for("12 EUR", 12.0).as_deref(), Some("0\" EUR\""));
    }

    #[test]
    fn rowspans_reserve_columns_and_hidden_rows_shrink_merges() {
        let rows = table_cells("<table><tr><td rowspan=3 colspan=2>M</td><td>A</td></tr><tr hidden><td>hidden</td></tr><tr><td>B</td></tr></table>");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].rowspan, 2);
        assert_eq!(rows[0][0].colspan, 2);
        assert_eq!(rows[0][2].text, "A");
        assert_eq!(rows[1][2].text, "B");
        assert!(rows[1][0].text.is_empty());
    }

    #[test]
    fn hidden_cells_and_subtrees_do_not_contribute_values() {
        let rows = table_cells(
            "<style>.off {display:none}</style><table><tr class=off><td>row</td></tr><tr><td hidden>cell</td><td>A<span class=off>secret</span>B</td></tr></table>",
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 1);
        assert_eq!(rows[0][0].text, "AB");
    }

    #[test]
    fn sparse_tables_and_rowspan_allocations_are_bounded() {
        let sparse = "<table><tr><td colspan=16384>x</td></tr><tr></tr></table>";
        assert!(table_cells(sparse).is_empty());
        assert!(table_cells("<table><tr><td rowspan=20000 colspan=2>x</td></tr></table>").is_empty());
        assert!(table_cells("<table><tr><td rowspan=999999999999999999999999999999>x</td></tr></table>").is_empty());
    }
}
